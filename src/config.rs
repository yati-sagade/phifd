use std::net::SocketAddr;
use std::time::Duration;

pub struct Config {
    pub ping_interval: Duration, // seconds
    pub num_members_to_ping: u8,
    pub window_size: usize,
    pub addr: SocketAddr,
}

impl Config {
    pub fn default() -> Config {
        Config {
            ping_interval: Duration::from_millis(1000),
            num_members_to_ping: 3,
            addr: "0.0.0.0:12345".parse::<SocketAddr>().unwrap(),
            window_size: 10usize,
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

    pub fn set_window_size(&mut self, sz: usize) -> &mut Config {
        self.window_size = sz;
        self
    }
}
