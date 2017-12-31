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
use std::net::SocketAddr;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::cmp::min;

use rand::{Rng, thread_rng, seq};
use futures::{Future, Poll, Async, Stream, Sink};
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
    inter_arrival_windows: HashMap<u16, InterArrivalWindow>,
    members: Rc<RefCell<Vec<Member>>>,
    timestamps: Rc<RefCell<HashMap<(u32, u16), u64>>>,
    ping_interval: Duration,
    num_members_to_ping: u8,
}

impl PhiFD {
    pub fn new() -> PhiFD {
        PhiFD {
            inter_arrival_windows: HashMap::new(),
            members: Rc::new(RefCell::new(vec![])),
            ping_interval: Duration::from_millis(1000),
            timestamps: Rc::new(RefCell::new(HashMap::new())),
            num_members_to_ping: 3,
        }
    }

    pub fn with_members(members: Vec<Member>) -> PhiFD {
        let mut ret = PhiFD::new();
        ret.members.borrow_mut().extend(members.into_iter());
        ret
    }

    fn merge(&mut self, gossip: Gossip) {}

    pub fn run(&mut self, addr: &str) {
        let addr_orig = addr;
        let addr = addr.parse::<SocketAddr>().unwrap();
        let mut core = Core::new().unwrap();
        let handle = core.handle();
        let dur = self.ping_interval.clone();
        let ticker = Interval::new(dur, &handle).unwrap();

        let conn = UdpSocket::bind(&addr, &handle).unwrap();
        let (sink, stream) = conn.framed(GossipCodec).split();

        let num_members_to_ping = self.num_members_to_ping as usize;
        let members = self.members.borrow();

        let pinger = ticker.and_then(move |_| {
            // pick up to k members to ping from membership list that are alive
            let mut rng = thread_rng();
            let nmembers = members.len();
            let k = min(num_members_to_ping, nmembers);

            info!("Going to ping {} members now (have {} members)", k, nmembers);

            let sample_indices = seq::sample_indices(
                &mut rng, nmembers, k
            );
            for idx in sample_indices.into_iter() {
                let m: &Member = &members[idx];
                println!("  Pinging {:?}", m);
                let gossip = make_gossip(addr_orig, &*members).unwrap();
                let ping_one = sink.send((addr.clone(), gossip)).then(|_| {
                    info!("Done pinging ");
                    Ok(())
                });
                handle.spawn(ping_one);
            }
            // ping them and move on
            // ??? for SWIM style stuff we'll need to keep track of who we
            // pinged and set appropriate timeouts.
            Ok(NoResult)
        });

        let ping_receiver = stream.and_then(|(addr, gossip)| {
            info!("received ping!");
            merge(Gossip::new(), &mut *self.members.borrow_mut(),
                &mut *self.timestamps.borrow_mut());
            Ok(MembershipList(vec![]))
        });

        let srv = pinger.select(ping_receiver).for_each(|result| {
            match result {
                NoResult => {}
                MembershipList(merged) => {
                    info!("got merged list as {:?}", &merged);
                }
            }
            Ok(())
        });

        core.run(srv).unwrap();
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



#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
