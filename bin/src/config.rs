use std::{convert::TryFrom, net::SocketAddr, num::NonZeroU32, time::Duration};

use anyhow::anyhow;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use trust_dns_proto::rr::Name;

use crate::{
    args::{Args, Protocol},
    query::Source,
};

#[derive(Debug, Clone)]
pub struct Config {
    pub protocol: Protocol,
    pub addr: SocketAddr,
    qps: u32,
    pub timeout: Duration,
    pub generators: usize,
    pub src: Source,
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

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
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

impl TryFrom<&Args> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &Args) -> Result<Self, Self::Error> {
        let msg_src = if let Some(f) = &args.file {
            Source::File(f.clone())
        } else {
            Source::Static {
                name: Name::from_ascii(&args.record).map_err(|err| {
                    anyhow!(
                        "failed to parse record: {:?}. with error: {:?}",
                        args.record,
                        err
                    )
                })?,
                qtype: args.qtype,
            }
        };

        Ok(Self {
            protocol: args.protocol,
            addr: (
                args.ip
                    .expect("This is a bug, IP always has a value at this point"),
                args.port,
            )
                .into(),
            src: msg_src,
            qps: args.qps,
            timeout: Duration::from_secs(args.timeout),
            generators: args.tcount * args.wcount,
        })
    }
}
