use proto::msg::{Member, Gossip};
use std::net::{SocketAddr, AddrParseError, IpAddr, Ipv4Addr, ToSocketAddrs};
use std::io;

#[derive(Clone, Debug)]
pub enum GossipType {
    Syn,
    Ack,
}

impl GossipType {
    pub fn from_u32(v: u32) -> Option<GossipType> {
        if v == 0 {
            Some(GossipType::Syn)
        } else if v == 1 {
            Some(GossipType::Ack)
        } else {
            None
        }
    }
}

impl Into<u32> for GossipType {
    fn into(self) -> u32 {
        match self.clone() {
            GossipType::Ack => 1,
            GossipType::Syn => 0,
        }
    }
}


pub fn ip_number_and_port_from_sockaddr(addr: SocketAddr) -> Result<(u32, u16), AddrParseError> {
    if let SocketAddr::V4(addr) = addr {
        let octets = addr.ip().octets();
        let ip = ((octets[0] as u32) << 24) | ((octets[1] as u32) << 16) |
            ((octets[2] as u32) << 8) | octets[3] as u32;
        Ok((ip, addr.port()))
    } else {
        "invalidaddr".parse::<SocketAddr>().map(|_| (0, 0))
    }
}

pub fn member_from_address(addr: &str) -> Result<Member, AddrParseError> {
    let addr = addr.parse::<SocketAddr>()?;
    member_from_sockaddr(addr)
}

pub fn member_from_sockaddr(addr: SocketAddr) -> Result<Member, AddrParseError> {
    let mut member = Member::new();
    let (ip, port) = ip_number_and_port_from_sockaddr(addr)?;
    member.set_ip(ip);
    member.set_port(port as u32);
    member.set_heartbeat(0);
    member.set_suspicion(0f64);
    Ok(member)
}

pub fn make_gossip<I>(heartbeat: u64, members: I, typ: GossipType) -> Gossip 
        where I: Iterator<Item=Member> {
    let mut gossip = Gossip::new();
    gossip.set_kind(typ.into());
    gossip.set_heartbeat(heartbeat);
    for member in members {
        gossip.mut_members().push(member.clone());
    }
    gossip
}

pub fn member_addr(member: &Member) -> SocketAddr {
    let ip = member.get_ip();
    let a = ((ip >> 24) & 0xff) as u8;
    let b = ((ip >> 16) & 0xff) as u8;
    let c = ((ip >> 8) & 0xff) as u8;
    let d = (ip & 0xff) as u8;
    let ipaddr = IpAddr::V4(Ipv4Addr::new(a, b, c, d));
    let port = member.get_port() as u16;
    SocketAddr::new(ipaddr, port)
}




pub fn resolve_first_ipv4(host: &str) -> io::Result<Option<SocketAddr>> {
    Ok(host.to_socket_addrs()? // Err return happens here
        .filter(SocketAddr::is_ipv4)
        .take(1)
        .next())
}

