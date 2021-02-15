#![warn(
    missing_debug_implementations,
    // missing_docs, // we shall remove thee, someday!
    rust_2018_idioms,
    unreachable_pub,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use anyhow::anyhow;
use argh::FromArgs;
use trust_dns_proto::rr::RecordType;

use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

/// nailgun
/// Evan Cameron <cameron.evan@gmail.com>
///
/// nailgun is a small, fast, configurable tool for functional testing,
/// benchmarking, and stress testing DNS servers and networks. It supports IPv4,
/// IPv6, UDP, TCP, and DoT and has a modular system for generating queries used
/// in the tests.
#[derive(FromArgs)]
struct CliArgs {
    /// IP address to bind to. Default is 0.0.0.0 for ipv4 or ::0 for ipv6
    #[argh(option, short = 'b', default = "[0,0,0,0].into()")]
    ip: IpAddr,
    /// which port to nail. Default is 53 for UDP/TCP
    #[argh(option, short = 'p', default = "53")]
    port: u16,
    /// delay between each traffic generator's run in milliseconds. Default is
    /// 1.
    #[argh(option, short = 'd', default = "1")]
    delay_ms: u64,
    /// the base record to use as the query for generators. Default is test.com.
    #[argh(option, short = 'r', default = "String::from(\"test.com.\")")]
    record: String,
    /// the query type to use for generators. Default is A.
    #[argh(option, short = 'T', default = "RecordType::A")]
    qtype: RecordType,
    /// query timeout in seconds. Default is 3.
    #[argh(option, short = 't', default = "3")]
    timeout: u64,
    /// protocol to use. Default is udp.
    #[argh(option, short = 'P', default = "Protocol::UDP")]
    protocol: Protocol,
    /// rate limit to a maximum queries per second, 0 is unlimited. Default is
    /// 0.
    #[argh(option, short = 'Q', default = "0")]
    qps: usize,
    /// read records from a file, one per row, QNAME and QTYPE. Used with the file generator.
    #[argh(option, short = 'f')]
    file: Option<usize>,
}

/* TODO: unfinished ones
 * /// internet family. Default is inet.
 * #[argh(option, short = 'F', default = "inet")]
 * family: String */

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum Protocol {
    UDP,
    TCP, // DOH
}

impl FromStr for Protocol {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "udp" => Ok(Protocol::UDP),
            "tcp" => Ok(Protocol::TCP),
            _ => Err(anyhow!(
                "unknown protocol type: {:?} must be \"udp\" or \"tcp\"",
                s
            )),
        }
    }
}

fn main() {
    println!("Hello, world!");
}
