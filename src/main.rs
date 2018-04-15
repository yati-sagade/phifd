extern crate phifd;
#[macro_use]
extern crate log;
extern crate simple_logger;
extern crate getopts;

extern crate futures;
extern crate tokio_core;
extern crate tokio_io;

use std::process;
use std::env;
use std::time::Duration;
use phifd::{PhiFD, Config};
use phifd::util;
use getopts::Options;
use log::LogLevel;
use phifd::proto::msg::Member;

fn main() {
    process::exit(match run() {
        Ok(_) => 0,
        Err(_) => 1,
    });
}


fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options]", program);
    print!("{}", opts.usage(&brief));
}

fn run() -> Result<(), ()> {
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
        )
        .optopt(
            "d",
            "ticker_delay_secs",
            "Upper limit of a random delay to apply to periodic ping-outs",
            "DELAY"
        );


    let matches = match opts.parse(&args[1..]) {
        Ok(m) => m,
        Err(f) => panic!(f.to_string()),
    };

    if matches.opt_present("h") {
        print_usage(&prog, opts);
        return Ok(());
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

    let introducer_ips = introducers
        .iter()
        .map(|addr| util::resolve_first_ipv4(addr))
        .filter_map(|addr_result| match addr_result {
            Ok(some_or_none) => some_or_none,
            Err(_) => None,
        })
        .collect::<Vec<_>>();

    if introducers.len() > 0 && introducer_ips.len() == 0 {
        warn!("Cannot resolve even one introducer. Quitting.");
        return Err(());
    }

    println!("Intoducer ips: {:?}", &introducer_ips);


    let mut cfg = Config::default();
    cfg.set_ping_interval(Duration::from_millis(
        matches
            .opt_str("ping_interval")
            .map(|s| s.parse::<u64>().expect("invalid number of secs"))
            .unwrap_or(1) * 1000,
    ));

    matches.opt_str("ticker_delay_secs")
           .map(|s| {
               let secs = s.parse::<u8>().expect("invalid # of secs");
               cfg.set_ticker_delay(secs);
           });

    let addrstr = matches.opt_str("addr").unwrap_or(
        "0.0.0.0:12345".to_string(),
    );

    let sockaddr = util::resolve_first_ipv4(&addrstr)
        .expect("Error resolving listen address")
        .expect("No resolution for given listen address");

    cfg.set_addr(sockaddr);

    info!("starting failure detector now on {}", &sockaddr);
    let mut fd = if introducers.len() != 0 {
        let members = introducer_ips
            .into_iter()
            .map(|intro| util::member_from_sockaddr(intro).unwrap())
            .collect::<Vec<Member>>();
        PhiFD::with_members(members, Some(cfg))
    } else {
        PhiFD::new(Some(cfg))
    };
    fd.run();
    Ok(())
}
