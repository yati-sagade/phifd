extern crate bytes;
extern crate futures;
extern crate tokio_io;
#[macro_use]
extern crate log;
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate statrs;

extern crate protobuf;
extern crate time;
extern crate rand;

use std::cell::RefCell;
use std::rc::Rc;
use std::io;
use std::net::SocketAddr;
use std::time::Instant;
use std::collections::HashMap;
use std::cmp;

use rand::{thread_rng, seq, Rng};
use futures::{Stream, stream};
use tokio_core::net::{UdpSocket, UdpCodec};
use tokio_core::reactor::{Core, Interval};
use proto::msg::{Gossip, Member};
use member::{MemberState, MemberID};
use protobuf::core::{Message, parse_from_bytes};

pub mod proto;
pub mod util;
pub mod config;
pub mod member;

pub use config::*;
pub use util::*;
pub use member::*;


pub struct PhiFD {
    state: Rc<RefCell<FDState>>,
}


enum FDEvent {
    PingOut(Vec<(SocketAddr, Gossip)>), // optimize this, gossip is the same for all
    AckOut(SocketAddr, Gossip),
    StateUpdated,
    Unexpected(String),
}

use FDEvent::*;


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
        let wnd_sz = ret.config.window_size;
        for member in members.into_iter() {
            let ip = member.get_ip();
            let port = member.get_port() as u16;
            ret.members.insert(
                (ip, port),
                MemberState::from_member(member, wnd_sz),
            );
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

        let mut sender = member_from_sockaddr(from_addr).expect("error recovering who pinged us");
        sender.set_heartbeat(gossip.get_heartbeat());

        // FIXME: don't do this over and over
        let our_addr = ip_number_and_port_from_sockaddr(self.config.addr).expect(
            "could not parse our own ip/port?",
        );


        // handle the sender
        let snd_ip = sender.get_ip();
        let snd_port = sender.get_port() as u16;
        let snd_addr = (snd_ip, snd_port);

        if our_addr != snd_addr {
            let heartbeat = sender.get_heartbeat();
            let susp = sender.get_suspicion();
            let wnd_sz = self.config.window_size;
            self.members
                .entry(snd_addr)
                .or_insert_with(move || MemberState::from_member(sender, wnd_sz))
                .merge(susp, heartbeat);
        } else {
            warn!(
                "We sent a ping to ourselves (us: {:?}, from: {:?})",
                &our_addr,
                &snd_addr
            );
        }

        for incoming_member in gossip.take_members().into_iter() {
            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port() as u16;
            let addr = (ip, port);

            if addr != our_addr {
                let susp = incoming_member.get_suspicion();
                let heartbeat = incoming_member.get_heartbeat();
                let wnd_sz = self.config.window_size;
                self.members
                    .entry(addr)
                    .or_insert_with(move || MemberState::from_member(incoming_member, wnd_sz))
                    .merge(susp, heartbeat);
            }
        }
    }
}


impl PhiFD {
    pub fn new(config: Option<Config>) -> PhiFD {
        PhiFD { state: Rc::new(RefCell::new(FDState::new(config))) }
    }

    pub fn with_members(members: Vec<Member>, config: Option<Config>) -> PhiFD {
        PhiFD { state: Rc::new(RefCell::new(FDState::with_members(members, config))) }
    }

    fn log_suspicisions(&self) {
        let now = Instant::now();
        for memberstate in self.state.borrow().members.values() {
            if let Some(susp) = memberstate.phi(now) {
                let (id, port) = memberstate.get_id();
                info!("phi({}:{})={:4}", id, port, susp);
            }
        }
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

        let slowness_level = self.state.borrow().config.ticker_delay;

        let num_members_to_ping = state.borrow().config.num_members_to_ping as usize;

        let pinger = ticker.and_then(|_| {

            // Pick up to k random peers
            let mut rng = thread_rng();
            let nmembers = state.borrow().members.len();
            let k = cmp::min(num_members_to_ping, nmembers);

            let ping_addrs = seq::sample_iter(&mut rng, state.borrow().members.values(), k)
                .map(|values| {
                    values
                        .iter()
                        .map(|v| member_addr(v.get_member_ref()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or(vec![]);

            // And signal for them to be pinged with a Syn ping. Note we just
            // return the (peer_addr, gossip) pairs letting the downstream
            // take care of actually sending the pings.
            let cur_heartbeat = state.borrow().cur_heartbeat();
            let pings = ping_addrs
                .into_iter()
                .map(|addr| {
                    let gossip = make_gossip(
                        cur_heartbeat,
                        state.borrow().members.values().map(|m| {
                            m.get_member_ref().clone()
                        }),
                        GossipType::Syn,
                    );
                    (addr, gossip)
                })
                .collect::<Vec<_>>();

            // Update heartbeat
            state.borrow_mut().epoch();

            // Print crap
            self.log_suspicisions();

            Ok(PingOut(pings))
        });

        let ping_listener = stream.and_then(|(addr_from, gossip)| {
            // 1. Merge the incoming membership state with our state.
            state.borrow_mut().merge(addr_from, gossip.clone());

            // 2. Then send an Ack ping with our updated membership list only if
            // the incoming gossip is a Syn. If the ping was an Ack, this means
            // we previously pinged that peer with a Syn and are just receiving
            // their merged membership list, which we merged again above.

            // Note that we don't actually send the ping here, but just
            // return a future (that resolves immediately, since Result<T,U>
            // is a type for which the Future trait is implemented.

            let cur_heartbeat = state.borrow().cur_heartbeat();
            match GossipType::from_u32(gossip.get_kind()) {
                Some(GossipType::Syn) => {
                    let gossip = make_gossip(
                        cur_heartbeat,
                        state.borrow().members.values().map(|m| {
                            m.get_member_ref().clone()
                        }),
                        GossipType::Ack,
                    );
                    Ok(AckOut(addr_from, gossip))
                }
                Some(GossipType::Ack) => Ok(StateUpdated),
                None => Ok(Unexpected(
                    "Expected a gossip type, but got none!".to_string(),
                )),
            }
        });

        let fut = pinger.select(ping_listener) // Pick events from any of the two streams, as they happen.
            // We produce events in the resulting stream only if want to
            // send stuff out. This happens if we are supposed to ping our
            // peers, or if we received a ping and are now supposed to 
            // send an ack ping to the peer. These events are encoded
            // as the variants of the FDEvent enum, which we consider
            // one by one below. The resulting stream yields (addr, gossip)
            // tuples (notice the flatten()), which are then just forwarded
            // to a sink wrapping over a UDP socket we previously bound to.
            // The GossipCodec type handles the encoding of (addr, gossip)
            // pairs into bytes and the other way around.
            .filter_map(|evt| match evt {
                AckOut(addr, gossip) => {
                    info!("Going to ack ping from {:?}, gossip kind {}", &addr,
                          gossip.get_kind());
                    Some(stream::iter_ok::<_, io::Error>(
                            vec![(addr, gossip)].into_iter()))
                }
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
                Unexpected(msg) => {
                    warn!("Something unexpected happened: {}", msg);
                    None
                }
            })
            // Till here, we have a stream yielding *iterators*, so flatten
            // them to the actual (addr, gossip) tuples.
            .flatten()
            .forward(sink);
        // All the above drama exists as a computation graph, condensed in the
        // form of a single future that should be driven to completion (which
        // is indefinite for our case, we achieve our goals as a side effect
        // of pursuing the completion of this future.
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
