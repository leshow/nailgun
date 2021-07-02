use std::{net::SocketAddr, path::PathBuf, time::Duration};

use trust_dns_proto::rr::{Name, RecordType};

use crate::args::Protocol;

#[derive(Debug, Clone)]
pub struct Config {
    pub protocol: Protocol,
    pub addr: SocketAddr,
    pub record: Name,
    pub qtype: RecordType,
    pub qps: u32,
    pub delay_ms: Duration,
    pub timeout: Duration,
    pub file: Option<PathBuf>,
    pub generators: usize,
}

impl Config {
    pub fn batch_size(&self) -> u32 {
        1_000
    }

    pub fn rate_per_gen(&self) -> u32 {
        self.qps / self.generators as u32
    }
}
