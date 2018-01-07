extern crate bytes;
extern crate futures;
extern crate tokio_io;
#[macro_use]
extern crate log;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate probability;
extern crate protobuf;
extern crate time;
extern crate rand;

use std::cell::RefCell;
use std::rc::Rc;
use std::io;
use std::net::SocketAddr;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::cmp;

use rand::{thread_rng, seq};
use futures::{Stream, stream};
use tokio_core::net::{UdpSocket, UdpCodec};
use tokio_core::reactor::{Core, Interval};
use proto::msg::{Gossip, Member};
use protobuf::core::{Message, parse_from_bytes};

pub mod proto;
pub mod util;

pub use util::*;

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[derive(Clone, Debug)]
struct InterArrivalWindow {
    mean: f64,
    variance: f64,
    std: f64,
}

pub struct PhiFD {
    state: Rc<RefCell<FDState>>,
}

enum FDEvent {
    PingOut(Vec<(SocketAddr, Gossip)>), // optimize this, gossip is the same for all
    StateUpdated,
}

use FDEvent::*;

/// This type is used to identify a member uniquely using its IPv4 number and
/// port.
type MemberID = (u32, u16);

/// This stores this process' knowledge about a given member at any given time.
#[derive(Clone, Debug)]
struct MemberState {

    /// Information about the member that we need to gossip with other peers.
    member: Member,

    /// The last UNIX time when this member's state was updated.
    timestamp: u64,

    /// Book keeping for estimating the distribution of inter-arrival times of
    /// pings from this node.
    inter_arrival_windows: Option<InterArrivalWindow>,
}

impl MemberState {
    fn from_member(member: Member) -> MemberState {
        MemberState {
            member: member,
            timestamp: unix_timestamp(),
            inter_arrival_windows: None,
        }
    }

    fn merge(&mut self, suspicion: f64, heartbeat: u64) {
        if self.member.get_heartbeat() < heartbeat {
            self.member.set_heartbeat(heartbeat);
            self.timestamp = unix_timestamp();
        }
    }

    fn get_id(&self) -> MemberID {
        (self.member.get_ip(), self.member.get_port() as u16)
    }
}

struct FDState {
    members: HashMap<MemberID, MemberState>,
    config: Config,
    heartbeat: u64,
}

impl FDState {
    fn new(config: Option<Config>) -> FDState {
        let config = config.unwrap_or(Config::default());
        FDState {
            members: HashMap::new(),
            config: config,
            heartbeat: 0u64,
        }
    }

    fn epoch(&mut self) {
        self.heartbeat += 1;
    }

    fn cur_heartbeat(&self) -> u64 {
        self.heartbeat
    }

    fn with_members(members: Vec<Member>, config: Option<Config>) -> FDState {
        let mut ret = FDState::new(config);
        for member in members.into_iter() {
            let ip = member.get_ip();
            let port = member.get_port() as u16;
            ret.members.insert((ip, port), MemberState::from_member(member));
        }
        ret
    }

    fn merge(&mut self, from_addr: SocketAddr, mut gossip: Gossip) {

        /* 1. We consider the sender node and the nodes present in the gossip.
         * 2. For each node considered, we check if their id (ip, port) is
         * in our membership list. If not there, we insert an entry. If there,
         * we need to merge the incoming knowledge with what we know. In reality,
         * we merge for both these cases anyway, because the .or_insert() API
         * is convenient. */

        let mut sender = member_from_sockaddr(from_addr)
                         .expect("error recovering who pinged us");
        sender.set_heartbeat(gossip.get_heartbeat());

        // FIXME: don't do this over and over
        let our_addr = ip_number_and_port_from_sockaddr(self.config.addr)
            .expect("could not parse our own ip/port?");


        // handle the sender
        let snd_ip = sender.get_ip();
        let snd_port = sender.get_port() as u16;
        let snd_addr = (snd_ip, snd_port);

        if our_addr != snd_addr {
            let heartbeat = sender.get_heartbeat();
            let susp = sender.get_suspicion();
            self.members.entry(snd_addr)
                .or_insert_with(move || MemberState::from_member(sender))
                .merge(susp, heartbeat);
        } else {
            warn!("We sent a ping to ourselves");
        }

        for incoming_member in gossip.take_members().into_iter() {

            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port() as u16;
            let addr = (ip, port);

            if addr != our_addr {
                let susp = incoming_member.get_suspicion();
                let heartbeat = incoming_member.get_heartbeat();
                self.members.entry(addr)
                    .or_insert_with(move || MemberState::from_member(incoming_member))
                    .merge(susp, heartbeat);
            } else {
                warn!("We sent a ping to ourselves");
            }
        }
    }
}

pub struct Config {
    ping_interval: Duration, // seconds
    num_members_to_ping: u8,
    addr: SocketAddr,
}

impl Config {
    pub fn default() -> Config {
        Config {
            ping_interval: Duration::from_millis(1000),
            num_members_to_ping: 3,
            addr: "0.0.0.0:12345".parse::<SocketAddr>().unwrap(),
        }
    }

    pub fn set_ping_interval(&mut self, interval: Duration) -> &mut Config {
        self.ping_interval = interval;
        self
    }

    pub fn set_num_members_to_ping(&mut self, n: u8) -> &mut Config {
        self.num_members_to_ping = n;
        self
    }

    pub fn set_addr(&mut self, addr: SocketAddr) -> &mut Config {
        self.addr = addr;
        self
    }
}

impl PhiFD {
    pub fn new(config: Option<Config>) -> PhiFD {
        PhiFD { state: Rc::new(RefCell::new(FDState::new(config))) }
    }

    pub fn with_members(members: Vec<Member>, config: Option<Config>) -> PhiFD {
        PhiFD { state: Rc::new(RefCell::new(FDState::with_members(members, config))) }
    }

    pub fn run(&mut self) {
        let mut core = Core::new().unwrap();
        let handle = core.handle();
        let dur = self.state.borrow().config.ping_interval.clone();
        let ticker = Interval::new(dur, &handle).unwrap();

        let state = &self.state;
        let listen_addr = state.borrow().config.addr.clone();

        let conn = UdpSocket::bind(&listen_addr, &handle).unwrap();
        let (sink, stream) = conn.framed(GossipCodec).split();

        let num_members_to_ping =
            state.borrow().config.num_members_to_ping as usize;

        let pinger = ticker.and_then(|_| {
            // pick up to k members to ping from membership list that are alive
            let mut rng = thread_rng();
            let nmembers = state.borrow().members.len();
            let k = cmp::min(num_members_to_ping, nmembers);

            let sample_indices = seq::sample_indices(&mut rng, nmembers, k);


            let cur_heartbeat = state.borrow().cur_heartbeat();

            let ping_addrs = seq::sample_iter(
                &mut rng,
                state.borrow().members.values(),
                nmembers
            ).map(|values| {
                values.iter().map(|v| member_addr(&v.member)).collect::<Vec<_>>()
            }).unwrap_or(vec![]);

            let pings = ping_addrs.into_iter().map(|addr| {
                let gossip = make_gossip(cur_heartbeat,
                                         state.borrow().members.values().map(|m| {
                                             m.member.clone()
                                         }));
                (addr, gossip)
            }).collect::<Vec<_>>();

            state.borrow_mut().epoch();

            Ok(PingOut(pings))
        });

        let ping_listener = stream.and_then(|(addr_from, gossip)| {
            state.borrow_mut().merge(addr_from, gossip);
            Ok(FDEvent::StateUpdated)
        });

        let fut = pinger
            .select(ping_listener)
            .filter_map(|evt| match evt {
                PingOut(ping_outs) => {
                    info!(
                        "going to ping {} member{} now (total {})",
                        ping_outs.len(),
                        if ping_outs.len() == 1 { "" } else { "s" },
                        state.borrow().members.len()
                    );
                    Some(stream::iter_ok::<_, io::Error>(ping_outs.into_iter()))
                }
                FDEvent::StateUpdated => {
                    info!("state updated");
                    None
                }
            })
            .flatten()
            .forward(sink);

        core.run(fut).unwrap();
    }
}

pub struct GossipCodec;

impl UdpCodec for GossipCodec {
    type In = (SocketAddr, Gossip);
    type Out = (SocketAddr, Gossip);

    fn decode(&mut self, src: &SocketAddr, buf: &[u8]) -> io::Result<Self::In> {
        info!("datagram received from {:?}", src);
        // TODO: this crashes when an old version of the FD pings us.
        let gossip = parse_from_bytes::<Gossip>(buf).unwrap();
        Ok((*src, gossip))
    }

    fn encode(&mut self, (addr, msg): Self::Out, buf: &mut Vec<u8>) -> SocketAddr {
        msg.write_to_vec(buf).expect(
            "an error occurred writing to the udp socket",
        );
        addr
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
