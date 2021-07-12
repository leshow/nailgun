use std::{net::SocketAddr, num::NonZeroU32, path::PathBuf, time::Duration};

use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use trust_dns_proto::rr::{Name, RecordType};

use crate::args::Protocol;

#[derive(Debug, Clone)]
pub struct Config {
    pub protocol: Protocol,
    pub addr: SocketAddr,
    pub record: Name,
    pub qtype: RecordType,
    pub qps: u32,
    pub timeout: Duration,
    pub generators: usize,
}

impl Config {
    pub const fn batch_size(&self) -> u32 {
        1_000
    }

    pub const fn rate_per_gen(&self) -> u32 {
        self.qps / self.generators as u32
    }

    pub const fn qps(&self) -> Qps {
        if self.qps == 0 {
            Qps::Unlimited
        } else {
            Qps::Limited(self.qps)
        }
    }

    pub fn rate_limiter(&self) -> Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>> {
        if self.qps().is_limited() {
            Some(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(self.rate_per_gen()).expect("QPS is non-zero"),
            )))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub enum Qps {
    Unlimited,
    Limited(u32),
}

impl Qps {
    /// Returns `true` if the qps is [`Unlimited`].
    pub fn is_unlimited(&self) -> bool {
        matches!(self, Self::Unlimited)
    }

    /// Returns `true` if the qps is [`Limited`].
    pub fn is_limited(&self) -> bool {
        matches!(self, Self::Limited(..))
    }
}
