use anyhow::{anyhow, Error};
use clap::Clap;
use trust_dns_proto::rr::{DNSClass, RecordType};

use std::{net::IpAddr, path::PathBuf, str::FromStr};

/// nailgun is a cli tool for stress testing and benchmarking DNS
#[derive(Debug, Clap, Clone, PartialEq, Eq)]
#[clap(author, about, version)]
pub struct Args {
    /// IP or domain send traffic to
    pub target: String,
    /// IP address to bind to. If family not set will use
    /// [default: 0.0.0.0]
    #[clap(long, short = 'b')]
    pub ip: Option<IpAddr>,
    /// which port to nail
    #[clap(long, short = 'p', default_value = "53")]
    pub port: u16,
    /// which internet family to use, (inet/inet6)
    #[clap(long, short = 'F', default_value = "inet")]
    pub family: Family,
    /// the base record to use as the query for generators
    #[clap(long, short = 'r', default_value = "test.com.")]
    pub record: String,
    /// the query type to use for generators
    #[clap(long, short = 'T', default_value = "A")]
    pub qtype: RecordType,
    /// the query class to use
    #[clap(long, default_value = "IN")]
    pub class: DNSClass,
    /// query timeout in seconds
    #[clap(long, short = 't', default_value = "2")]
    pub timeout: u64,
    /// protocol to use (udp/tcp)
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
    /// log output format (pretty/json/debug)
    #[clap(long, default_value = "pretty")]
    pub output: LogStructure,
    /// query generator type (static/file/randompkt/randomqname)
    #[clap(subcommand)]
    pub generator: Option<GenType>,
    /// output file for logs/metrics
    #[clap(short = 'o')]
    pub log_file: Option<PathBuf>,
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

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    INet,
    INet6,
}

impl FromStr for Family {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_ascii_lowercase()[..] {
            "inet" => Ok(Family::INet),
            "inet6" => Ok(Family::INet6),
            _ => Err(anyhow!(
                "unknown family type: {:?} must be \"inet\" or \"inet6\"",
                s
            )),
        }
    }
}

#[derive(Clap, Clone, PartialEq, Eq, Hash, Debug)]
pub enum GenType {
    Static,
    #[clap(name = "randompkt")]
    RandomPkt {
        #[clap(default_value = "600")]
        size: usize,
    },
    #[clap(name = "randomqname")]
    RandomQName {
        #[clap(default_value = "62")]
        size: usize,
    },
    File {
        #[clap(parse(from_os_str))]
        path: PathBuf,
    },
}
