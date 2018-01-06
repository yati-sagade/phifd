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
use std::cmp::min;

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

struct FDState {
    inter_arrival_windows: HashMap<u16, InterArrivalWindow>,
    members: Vec<Member>,
    timestamps: HashMap<(u32, u16), u64>,
    config: Config,
    heartbeat: u64,
}

impl FDState {
    fn new(config: Option<Config>) -> FDState {
        let config = config.unwrap_or(Config::default());
        FDState {
            inter_arrival_windows: HashMap::new(),
            members: vec![],
            timestamps: HashMap::new(),
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
        ret.members.extend(members.into_iter());
        ret
    }

    fn merge(&mut self, from_addr: SocketAddr, mut gossip: Gossip) {
        let mut incoming_members: HashMap<(u32, u32), Member> = HashMap::new();

        let mut sender = member_from_sockaddr(from_addr).expect("error recovering who pinged us");
        sender.set_heartbeat(gossip.get_heartbeat());

        // FIXME: don't do this over and over
        let (our_ip, our_port) = ip_number_and_port_from_sockaddr(self.config.addr)
            .map(|(a, b)| (a as u32, b as u32))
            .expect("could not parse our own ip/port?");


        incoming_members.insert((sender.get_ip(), sender.get_port()), sender);
        for incoming_member in gossip.take_members().into_iter() {
            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port();
            // FIXME: when we listen on 0.0.0.0:$port and someone pings us
            // via 127.0.0.1:$port, we will inject ourselves in our own memberlist.
            if (ip, port) != (our_ip, our_port) {
                incoming_members.insert((ip, port), incoming_member);
            }
        }

        for member in self.members.iter_mut() {
            let key = (member.get_ip(), member.get_port());
            let incoming_member = incoming_members.remove(&key);
            if let Some(incoming_member) = incoming_member {
                if incoming_member.get_heartbeat() > member.get_heartbeat() {
                    member.set_heartbeat(incoming_member.get_heartbeat());
                    self.timestamps.insert(
                        (member.get_ip(), member.get_port() as u16),
                        unix_timestamp(),
                    );
                }
            }
        }

        for (_, incoming_member) in incoming_members.into_iter() {
            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port() as u16;
            self.members.push(incoming_member);
            self.timestamps.insert((ip, port), unix_timestamp());

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

        let num_members_to_ping = self.state.borrow().config.num_members_to_ping as usize;


        let pinger = ticker.and_then(|_| {
            // pick up to k members to ping from membership list that are alive
            let mut rng = thread_rng();
            let nmembers = state.borrow().members.len();
            let k = min(num_members_to_ping, nmembers);

            let sample_indices = seq::sample_indices(&mut rng, nmembers, k);

            let cur_heartbeat = state.borrow().cur_heartbeat();

            let pings = sample_indices
                .into_iter()
                .map(|idx| {
                    let to_addr = member_addr(&state.borrow().members[idx]);
                    let gossip = make_gossip(to_addr, cur_heartbeat, &state.borrow().members)
                        .unwrap();
                    (to_addr, gossip)
                })
                .collect::<Vec<_>>();

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
