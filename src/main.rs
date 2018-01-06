extern crate phifd;
#[macro_use]
extern crate log;
extern crate simple_logger;
extern crate getopts;

extern crate futures;
extern crate tokio_core;
extern crate tokio_io;

use futures::stream::Stream;
use futures::Future;
use tokio_core::reactor::Core;
use tokio_core::net::TcpListener;

use std::net::SocketAddr;
use std::env;
use std::time::Duration;
use phifd::{PhiFD, Config};
use phifd::util::member_from_address;
use getopts::Options;
use log::LogLevel;
use phifd::proto::msg::Member;


fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

fn _main() {
    let mut core = Core::new().unwrap();
    let addr = "0.0.0.0:12345".parse().unwrap();
    let handle = core.handle();
    let listener = TcpListener::bind(&addr, &handle).unwrap();

    let connections = listener.incoming();
    let server = connections.for_each(|(sock, _peer_addr)| {
        let serve = tokio_io::io::write_all(sock, b"hello, world!\n").then(|_| Ok(()));
        handle.spawn(serve);
        Ok(())
    });
    core.run(server).unwrap();
}

fn main() {
    simple_logger::init_with_level(LogLevel::Info).unwrap();
    let args: Vec<String> = env::args().collect();
    let prog = args[0].clone();

    let mut opts = Options::new();
    opts.optmulti(
        "i",
        "intro",
        "address of an introducer node",
        "INTRODUCER_ADDR",
    ).optflag("h", "help", "print this help menu and exit")
        .optopt(
            "a",
            "addr",
            "address to listen on, by default 0.0.0.0:12345",
            "ADDR",
        )
        .optopt(
            "t",
            "ping_interval",
            "how often, in integral seconds, to ping peers",
            "INTERVAL",
        );


    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };

    if matches.opt_present("h") {
        print_usage(&prog, opts);
        return;
    }

    let introducers = matches.opt_strs("i");
    if introducers.len() == 0 {
        info!("no introducer specified, starting own cluster");
    } else {
        info!(
            "{} introducers specified, going to try each",
            introducers.len()
        );
    }


    let mut cfg = Config::default();
    cfg.set_ping_interval(Duration::from_millis(
        matches
            .opt_str("ping_interval")
            .map(|s| s.parse::<u64>().expect("invalid number of secs"))
            .unwrap_or(1) * 1000,
    ));

    let addr = matches
        .opt_str("addr")
        .unwrap_or("0.0.0.0:12345".to_string())
        .parse::<SocketAddr>()
        .expect("invalid listen address");

    cfg.set_addr(addr);

    info!("starting failure detector now on {}", &addr);
    let mut fd = if introducers.len() != 0 {
        let members = introducers
            .into_iter()
            .map(|intro| member_from_address(&intro).unwrap())
            .collect::<Vec<Member>>();
        PhiFD::with_members(members, Some(cfg))
    } else {
        PhiFD::new(Some(cfg))
    };
    fd.run();
}
