use proto::msg::{Member, Gossip};
use std::net::{SocketAddr, AddrParseError, IpAddr, Ipv4Addr};

pub fn ip_number_and_port(addr: &str) -> Result<(u32, u16), AddrParseError> {
    let addr = addr.parse::<SocketAddr>()?;
    ip_number_and_port_from_sockaddr(addr)
}

pub fn ip_number_and_port_from_sockaddr(addr: SocketAddr)
        -> Result<(u32, u16), AddrParseError> {
    if let SocketAddr::V4(addr) = addr {
        let octets = addr.ip().octets();
        let ip = ((octets[0] as u32) << 24) |
                 ((octets[1] as u32) << 16) |
                 ((octets[2] as u32) << 8)  | octets[3] as u32;
        Ok((ip, addr.port()))
    } else {
        "invalidaddr".parse::<SocketAddr>().map(|_| (0, 0))
    }
}

pub fn member_from_address(addr: &str) -> Result<Member, AddrParseError> {
    let mut member = Member::new();
    let (ip, port) = ip_number_and_port(addr)?;
    member.set_ip(ip);
    member.set_port(port as u32);
    member.set_heartbeat(0);
    Ok(member)
}

pub fn make_gossip(addr: SocketAddr,
                   members: &Vec<Member>) -> Result<Gossip, AddrParseError> {
    let (ip, port) = ip_number_and_port_from_sockaddr(addr)?;
    let mut gossip = Gossip::new();
    for member in members {
        gossip.mut_members().push(member.clone());
    }
    Ok(gossip)
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
