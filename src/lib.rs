extern crate bytes;
extern crate futures;
extern crate tokio_io;
#[macro_use]
extern crate log;
#[macro_use]
extern crate tokio_core;
extern crate tokio_proto;
extern crate tokio_service;
extern crate probability;
extern crate protobuf;
extern crate time;
extern crate rand;

use std::cell::RefCell;
use std::env;
use std::rc::Rc;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::net::SocketAddr;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::cmp::min;
use std::iter;

use rand::{Rng, thread_rng, seq};
use futures::{Future, Poll, Async, Stream, Sink, stream, StartSend, AsyncSink};
use tokio_core::net::{UdpSocket, UdpCodec};
use tokio_core::reactor::{Core, Interval};
use proto::msg::{Gossip, Member};
use protobuf::core::{Message, parse_from_bytes};

pub mod proto;
pub mod util;

pub use util::*;

fn unix_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

struct InterArrivalWindow {
    mean: f64,
    variance: f64,
    std: f64,
}

pub struct PhiFD {
    /*
    inter_arrival_windows: HashMap<u16, InterArrivalWindow>,
    members: Rc<RefCell<Vec<Member>>>,
    timestamps: Rc<RefCell<HashMap<(u32, u16), u64>>>,
    ping_interval: Duration,
    num_members_to_ping: u8,*/
    state: Rc<RefCell<FDState>>,
}

enum FDEvent {
    PingOut(Vec<(SocketAddr, Gossip)>), // optimize this, gossip is the same for all
    StateUpdated,
}

use FDEvent::*;

struct FDState {
    /*
    inter_arrival_windows: HashMap::new(),
    members: Rc::new(RefCell::new(vec![])),
    ping_interval: Duration::from_millis(1000),
    timestamps: Rc::new(RefCell::new(HashMap::new())),
    num_members_to_ping: 3,
    */
    inter_arrival_windows: HashMap<u16, InterArrivalWindow>,
    members: Vec<Member>,
    timestamps: HashMap<(u32, u16), u64>,
    ping_interval: Duration,
    num_members_to_ping: u8,
}

impl FDState {
    fn new() -> FDState {
        FDState {
            inter_arrival_windows: HashMap::new(),
            members: vec![],
            ping_interval: Duration::from_millis(1000),
            timestamps: HashMap::new(),
            num_members_to_ping: 3,
        }
    }
    fn with_members(members: Vec<Member>) -> FDState {
        let mut ret = FDState::new();
        ret.members.extend(members.into_iter());
        ret
    }

    fn merge(&mut self, _from_addr: SocketAddr, mut gossip: Gossip) {
        info!("merging state");
        let mut incoming_members: HashMap<(u32, u32), Member> = HashMap::new();

        let (_from_ip, _from_port) = (gossip.get_from_ip(), gossip.get_from_port());

        for incoming_member in gossip.take_members().into_iter() {
            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port();
            incoming_members.insert(
                (ip, port),
                incoming_member
            );
        }
        
        for member in self.members.iter_mut() {
            let key = (member.get_ip(), member.get_port());
            let incoming_member = incoming_members.remove(&key);
            if let Some(incoming_member) = incoming_member {
                if incoming_member.get_heartbeat() > member.get_heartbeat() {
                    member.set_heartbeat(incoming_member.get_heartbeat());
                    self.timestamps.insert((member.get_ip(), member.get_port() as u16),
                        unix_timestamp());
                }
            }
        }
        
        for (_, incoming_member) in incoming_members.into_iter() {
            let ip = incoming_member.get_ip();
            let port = incoming_member.get_port() as u16;
            self.members.push(incoming_member);
            self.timestamps.insert((ip, port), unix_timestamp());
                
        }
        info!("have {:?}, {:?} in the end", &self.members, &self.timestamps);
    }
}

impl PhiFD {
    pub fn new() -> PhiFD {
        PhiFD {
            state: Rc::new(RefCell::new(FDState::new())),
        }
    }

    pub fn with_members(members: Vec<Member>) -> PhiFD {
        PhiFD {
            state: Rc::new(RefCell::new(FDState::with_members(members))),
        }
    }

    fn merge(&mut self, addr_from: SocketAddr, gossip: Gossip) {
        info!("Going to merge gossip from {:?}", &addr_from);
    }

    pub fn run(&mut self, addr: &str) {
        let addr_orig = addr;
        let addr = addr.parse::<SocketAddr>().unwrap();
        let mut core = Core::new().unwrap();
        let handle = core.handle();
        let dur = self.state.borrow().ping_interval.clone();
        let ticker = Interval::new(dur, &handle).unwrap();

        let conn = UdpSocket::bind(&addr, &handle).unwrap();
        let (sink, stream) = conn.framed(GossipCodec).split();

        let num_members_to_ping = self.state.borrow().num_members_to_ping as usize;

        let state = &self.state;

        let pinger = ticker.and_then(|_| {
            // pick up to k members to ping from membership list that are alive
            let mut rng = thread_rng();
            let nmembers = state.borrow().members.len();
            let k = min(num_members_to_ping, nmembers);

            info!("Going to ping {} members now (have {} members)", k, nmembers);

            let sample_indices = seq::sample_indices(
                &mut rng, nmembers, k
            );

            let pings = sample_indices
                .into_iter()
                .map(|idx| {
                    let to_addr = member_addr(&state.borrow().members[idx]);
                    let gossip = make_gossip(to_addr, &state.borrow().members).unwrap();
                    (to_addr, gossip)
                }).collect::<Vec<_>>();

            Ok(PingOut(pings))
        });

        let ping_listener = stream.and_then(|(addr_from, gossip)| {
            state.borrow_mut().merge(addr_from, gossip);
            Ok(FDEvent::StateUpdated)
        });

        let fut = pinger
            .select(ping_listener)
            .filter_map(|evt| {
                match evt {
                    PingOut(ping_outs) => {
                        info!("going to ping {:?}", &ping_outs);
                        Some(stream::iter_ok::<_, io::Error>(ping_outs.into_iter()))
                    }
                    FDEvent::StateUpdated => {
                        info!("state updated");
                        None
                    }
                }
            })
            .flatten()
            .forward(sink);

        core.run(fut).unwrap();
    }
}

enum IntermediateResult {
    NoResult,
    MembershipList(Vec<Member>),
}

use IntermediateResult::*;

fn merge(mut gossip: Gossip, members: &mut Vec<Member>,
         timestamps: &mut HashMap<(u32, u16), u64>) {

    let mut incoming_members: HashMap<(u32, u32), Member> = HashMap::new();

    let (_from_ip, _from_port) = (gossip.get_from_ip(), gossip.get_from_port());

    for incoming_member in gossip.take_members().into_iter() {
        let ip = incoming_member.get_ip();
        let port = incoming_member.get_port();
        incoming_members.insert(
            (ip, port),
            incoming_member
        );
    }
    
    for member in members.iter_mut() {
        let key = (member.get_ip(), member.get_port());
        let incoming_member = incoming_members.remove(&key);
        if let Some(incoming_member) = incoming_member {
            if incoming_member.get_heartbeat() > member.get_heartbeat() {
                member.set_heartbeat(incoming_member.get_heartbeat());
                timestamps.insert((member.get_ip(), member.get_port() as u16),
                    unix_timestamp());
            }
        }
    }
    
    for (_, incoming_member) in incoming_members.into_iter() {
        let ip = incoming_member.get_ip();
        let port = incoming_member.get_port() as u16;
        members.push(incoming_member);
        timestamps.insert((ip, port), unix_timestamp());
            
    }
    // TODO: Remove crap from `timestamps`.
}

pub struct GossipCodec;

impl UdpCodec for GossipCodec {
    type In = (SocketAddr, Gossip);
    type Out = (SocketAddr, Gossip);

    fn decode(&mut self, src: &SocketAddr, buf: &[u8]) -> io::Result<Self::In> {
        let gossip = parse_from_bytes::<Gossip>(buf).unwrap();
        Ok((*src, gossip))
    }

    fn encode(&mut self, (addr, msg): Self::Out, buf: &mut Vec<u8>) -> SocketAddr {
        msg.write_to_vec(buf).unwrap();
        addr
    }
}

pub enum What {
    SendPing,
    HandlePing(SocketAddr, u64),
}

use What::*;

pub struct SimpleCodec;

impl UdpCodec for SimpleCodec {
    type In = (SocketAddr, u64);
    type Out = (SocketAddr, u64);

    fn decode(&mut self, src: &SocketAddr, buf: &[u8]) -> io::Result<Self::In> {
        let mut result = 0;
        for i in 0..8 {
            result |= (buf[i] as u64) << (i * 8);
        }
        Ok((*src, result))
    }

    fn encode(&mut self, (addr, msg): Self::Out, buf: &mut Vec<u8>) -> SocketAddr {
        for i in 0..8 {
            buf.push(((msg >> (i * 8)) | 0xff) as u8);
        }
        addr
    }
}

pub struct TickTock {
    heard: Rc<RefCell<HashMap<SocketAddr, u64>>>,
    curval: u64,
}

enum Event {
    PingSend(SocketAddr, u64),
    StateUpdated,
    Nothing,
}

use Event::*;


impl TickTock {
    pub fn new(introducers: Vec<SocketAddr>) -> TickTock {
        let mut map = HashMap::new();
        for intro in introducers.into_iter() {
            map.insert(intro, 0);
        }
        TickTock { heard: Rc::new(RefCell::new(map)), curval: 0 }
    }

    pub fn run(&mut self, bind_addr: &str) {
        let addr = bind_addr.parse::<SocketAddr>().unwrap();

        let mut core = Core::new().unwrap();
        let handle = core.handle();

        let dur = Duration::from_millis(1000);
        let ticker = Interval::new(dur, &handle).unwrap();

        let conn = UdpSocket::bind(&addr, &handle).unwrap();
        let (sink, stream) = conn.framed(SimpleCodec).split();

        let id = 42;

        let ticker = ticker.and_then(|_| {
            info!("tick");
            Ok(self.heard.borrow()
               .keys()
               .next()
               .map(|key| PingSend(key.clone(), id))
               .unwrap_or(Nothing))
        });

        let ping_listener = stream.and_then(|(addr_from, value)| {
            self.heard.borrow_mut().insert(addr_from, value);
            Ok(Event::StateUpdated)
        });

        let fut = ticker
            .select(ping_listener)
            .filter_map(|evt| {
                match evt {
                    Nothing => {
                        info!("nothing to do");
                        None
                    },
                    Event::StateUpdated => {
                        info!("state updated");
                        None
                    },
                    PingSend(to, val) => {
                        info!("will ping {:?}", &to);
                        Some((to, val))
                    },
                }
            }).forward(sink);

        core.run(fut).unwrap();
    }
}



#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
