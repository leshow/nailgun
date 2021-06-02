use anyhow::{anyhow, Error};
use clap::Clap;
use trust_dns_proto::rr::RecordType;

use std::{net::IpAddr, path::PathBuf, str::FromStr};

/// nailgun is a small, fast, configurable tool for functional testing,
/// benchmarking, and stress testing DNS servers and networks. It supports IPv4,
/// IPv6, UDP, TCP, and DoT (eventually)
#[derive(Debug, Clap, Clone, PartialEq, Eq)]
#[clap(version = "0.1.0", author = "Evan Cameron <cameron.evan@gmail.com>")]
pub(crate) struct Args {
    /// IP address to bind to
    #[clap(long, short = 'b', default_value = "0.0.0.0")]
    pub(crate) ip: IpAddr,
    /// which port to nail. Default is 53 for UDP/TCP
    #[clap(long, short = 'p', default_value = "53")]
    pub(crate) port: u16,
    /// delay between each traffic generator's run in milliseconds
    #[clap(long, short = 'd', default_value = "1")]
    pub(crate) delay_ms: u64,
    /// the base record to use as the query for generators
    #[clap(long, short = 'r', default_value = "test.com.")]
    pub(crate) record: String,
    /// the query type to use for generators. Default is A.
    #[clap(long, short = 'T', default_value = "A")]
    pub(crate) qtype: RecordType,
    /// query timeout in seconds. Default is 3.
    #[clap(long, short = 't', default_value = "3")]
    pub(crate) timeout: u64,
    /// protocol to use. Default is udp.
    #[clap(long, short = 'P', default_value = "udp")]
    pub(crate) protocol: Protocol,
    /// rate limit to a maximum queries per second, 0 is unlimited
    #[clap(long, short = 'Q', default_value = "0")]
    pub(crate) qps: usize,
    /// number of concurrent traffic generators per process
    #[clap(long, short = 'c', default_value = "10")]
    pub(crate) tcount: usize,
    /// number of tokio worker threads to spawn
    #[clap(long, short = 'w', default_value = "1")]
    pub(crate) wcount: usize,
    /// read records from a file, one per row, QNAME and QTYPE. Used with the
    /// file generator.
    #[clap(long, short = 'f')]
    pub(crate) file: Option<PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Protocol {
    Udp,
    Tcp, // DOH
}

impl FromStr for Protocol {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "udp" => Ok(Protocol::Udp),
            "tcp" => Ok(Protocol::Tcp),
            _ => Err(anyhow!(
                "unknown protocol type: {:?} must be \"udp\" or \"tcp\"",
                s
            )),
        }
    }
}
