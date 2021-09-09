use std::{
    convert::TryFrom,
    net::{IpAddr, SocketAddr},
    num::NonZeroU32,
    time::Duration,
};

use anyhow::{anyhow, bail, Context};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use trust_dns_proto::rr::Name;

use crate::{
    args::{self, Args, GenType, Protocol},
    query::Source,
};

#[derive(Debug, Clone)]
pub struct Config {
    pub target: SocketAddr,
    pub name_server: Option<String>,
    pub protocol: Protocol,
    pub bind: IpAddr,
    qps: u32,
    pub timeout: Duration,
    pub generators: usize,
    pub query_src: Source,
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
    #[allow(dead_code)]
    pub fn is_unlimited(&self) -> bool {
        !self.is_limited()
    }

    /// Returns `true` if the qps is [`Limited`].
    pub fn is_limited(&self) -> bool {
        matches!(self, Self::Limited(..))
    }
}

impl TryFrom<&Args> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &Args) -> Result<Self, Self::Error> {
        use trust_dns_resolver::{config::*, Resolver};

        let query_src = match &args.generator {
            Some(GenType::File { path }) => Source::File(path.clone()),
            Some(GenType::Static) | None => Source::Static {
                name: Name::from_ascii(&args.record).map_err(|err| {
                    anyhow!(
                        "failed to parse record: {:?}. with error: {:?}",
                        args.record,
                        err
                    )
                })?,
                qtype: args.qtype,
                class: args.class,
            },
            Some(GenType::RandomPkt { size }) => Source::RandomPkt { size: *size },
            Some(GenType::RandomQName { size }) => Source::RandomQName {
                size: *size,
                qtype: args.qtype,
            },
        };

        let (target, name_server) = match args.target.parse::<IpAddr>() {
            #[cfg(feature = "doh")]
            Ok(target) if args.protocol == args::Protocol::DoH => {
                bail!("found {}: need to use a domain name for DoH", target)
            }
            Ok(target) => (target, None),
            Err(_) => {
                // is not an IP, so see if we can resolve it over dns
                let resolver =
                    Resolver::new(ResolverConfig::default(), ResolverOpts::default()).unwrap();
                let response = resolver.lookup_ip(args.target.clone()).unwrap();

                (
                    response
                        .iter()
                        .next()
                        .context("Resolver failed to return an addr")?,
                    Some(args.target.clone()),
                )
            }
        };
        Ok(Self {
            target: (target, args.port).into(),
            name_server,
            protocol: args.protocol,
            bind: args.bind_ip.context("Args::ip always has a value")?,
            query_src,
            qps: args.qps,
            timeout: Duration::from_secs(args.timeout),
            generators: args.tcount * args.wcount,
        })
    }
}
