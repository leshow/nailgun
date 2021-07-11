use anyhow::{anyhow, Error};
use clap::Clap;
use trust_dns_proto::rr::RecordType;

use std::{net::IpAddr, path::PathBuf, str::FromStr};

/// nailgun is a cli tool for stress testing and benchmarking DNS
#[derive(Debug, Clap, Clone, PartialEq, Eq)]
#[clap(version = "0.1.0", author = "Evan Cameron <cameron.evan@gmail.com>")]
pub struct Args {
    /// IP address to bind to
    #[clap(long, short = 'b', default_value = "0.0.0.0")]
    pub ip: IpAddr,
    /// which port to nail. Default is 53 for UDP/TCP
    #[clap(long, short = 'p', default_value = "53")]
    pub port: u16,
    /// the base record to use as the query for generators
    #[clap(long, short = 'r', default_value = "test.com.")]
    pub record: String,
    /// the query type to use for generators. Default is A.
    #[clap(long, short = 'T', default_value = "A")]
    pub qtype: RecordType,
    /// query timeout in seconds. Default is 2.
    #[clap(long, short = 't', default_value = "2")]
    pub timeout: u64,
    /// protocol to use. Default is udp.
    #[clap(long, short = 'P', default_value = "udp")]
    pub protocol: Protocol,
    /// rate limit to a maximum queries per second, 0 is unlimited
    #[clap(long, short = 'Q', default_value = "0")]
    pub qps: u32,
    /// number of concurrent traffic generators per process
    #[clap(long, short = 'c', default_value = "1")]
    pub tcount: usize,
    /// number of tokio worker threads to spawn
    #[clap(long, short = 'w', default_value = "1")]
    pub wcount: usize,
    /// limits traffic generation to n seconds, 0 is unlimited
    #[clap(long, short = 'l', default_value = "0")]
    pub limit_secs: u64,
    #[clap(long, default_value = "pretty")]
    pub log: LogStructure,
    /// read records from a file, one per row, QNAME and QTYPE. Used with the
    /// file generator.
    #[clap(long, short = 'f')]
    pub file: Option<PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum LogStructure {
    Debug,
    Pretty,
    Json,
}

impl FromStr for LogStructure {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "json" => Ok(LogStructure::Json),
            "pretty" => Ok(LogStructure::Pretty),
            "debug" => Ok(LogStructure::Debug),
            _ => Err(anyhow!(
                "unknown log structure type: {:?} must be \"json\" or \"compact\" or \"pretty\"",
                s
            )),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Protocol {
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
