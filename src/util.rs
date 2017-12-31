use proto::msg::{Member, Gossip};
use std::net::{SocketAddr, AddrParseError};

pub fn ip_number_and_port(addr: &str) -> Result<(u32, u16), AddrParseError> {
    let addr = addr.parse::<SocketAddr>()?;
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

pub fn make_gossip(from: &str, members: &Vec<Member>) -> Result<Gossip, AddrParseError> {
    let (ip, port) = ip_number_and_port(from)?;
    let mut gossip = Gossip::new();
    for member in members {
        gossip.mut_members().push(member.clone());
    }
    Ok(gossip)
}
