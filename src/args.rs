use anyhow::{anyhow, Error};
use clap::Clap;
use trust_dns_proto::rr::RecordType;

use std::{net::IpAddr, str::FromStr};

/// nailgun is a small, fast, configurable tool for functional testing,
/// benchmarking, and stress testing DNS servers and networks. It supports IPv4,
/// IPv6, UDP, TCP, and DoT and has a modular system for generating queries used
/// in the tests.
#[derive(Clap, Clone, PartialEq, Eq)]
#[clap(version = "0.1.0", author = "Evan Cameron <cameron.evan@gmail.com>")]
pub(crate) struct Args {
    /// IP address to bind to. Default is 0.0.0.0 for ipv4 or ::0 for ipv6
    #[clap(long, short = 'b', default_value = "[0,0,0,0].into()")]
    pub(crate) ip: IpAddr,
    /// which port to nail. Default is 53 for UDP/TCP
    #[clap(long, short = 'p', default_value = "53")]
    pub(crate) port: u16,
    /// delay between each traffic generator's run in milliseconds. Default is
    /// 1.
    #[clap(long, short = 'd', default_value = "1")]
    pub(crate) delay_ms: u64,
    /// the base record to use as the query for generators. Default is test.com.
    #[clap(long, short = 'r', default_value = "String::from(\"test.com.\")")]
    pub(crate) record: String,
    /// the query type to use for generators. Default is A.
    #[clap(long, short = 'T', default_value = "RecordType::A")]
    pub(crate) qtype: RecordType,
    /// query timeout in seconds. Default is 3.
    #[clap(long, short = 't', default_value = "3")]
    pub(crate) timeout: u64,
    /// protocol to use. Default is udp.
    #[clap(long, short = 'P', default_value = "Protocol::UDP")]
    pub(crate) protocol: Protocol,
    /// rate limit to a maximum queries per second, 0 is unlimited. Default is
    /// 0.
    #[clap(long, short = 'Q', default_value = "0")]
    pub(crate) qps: usize,
    /// number of concurrent traffic generators per process. Default is 10.
    #[clap(long, short = 'c', default_value = "10")]
    pub(crate) tcount: usize,
    /// number of tokio worker threads to spawn
    #[clap(long, short = 'w', default_value = "num_cpus::get()")]
    pub(crate) wcount: usize,
    // TODO:
    /// read records from a file, one per row, QNAME and QTYPE. Used with the file generator.
    #[clap(long, short = 'f')]
    pub(crate) file: Option<usize>,
}

/* TODO: unfinished ones
 * /// internet family. Default is inet.
 * #[clap(option, short = 'F', default = "inet")]
 * family: String */

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Protocol {
    UDP,
    TCP, // DOH
}

impl FromStr for Protocol {
    type Err = Error;

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
