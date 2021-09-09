#![warn(
    missing_debug_implementations,
    // missing_docs, // TODO
    rust_2018_idioms,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(rustdoc::broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use anyhow::{anyhow, bail, Result};
use clap::Clap;
use tokio::{
    runtime::Builder,
    signal,
    sync::{broadcast, mpsc},
    time,
};
use tracing::{error, info, trace};

mod args;
mod config;
mod gen;
mod logs;
mod msg;
mod query;
mod sender;
mod shutdown;
mod stats;
mod store;
mod util;

use crate::{
    args::{Args, Family},
    gen::Generator,
    shutdown::Shutdown,
    stats::StatsRunner,
};

fn main() -> Result<()> {
    let mut args = Args::parse();
    // set default address for family if none provided
    if args.bind_ip.is_none() {
        match args.family {
            Family::INet6 => {
                args.bind_ip = Some(IpAddr::V6(Ipv6Addr::UNSPECIFIED));
            }
            Family::INet => {
                args.bind_ip = Some(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
            }
        }
    } else if let Some(ip) = args.bind_ip {
        if ip.is_ipv4() && args.family == Family::INet6 {
            bail!("can't bind to ipv4 while in ipv6 mode");
        } else if ip.is_ipv6() && args.family == Family::INet {
            bail!("can't bind to ipv6 while in ipv4 mode");
        }
    }

    let _guard = logs::setup(&args);

    trace!("{:?}", args);
    let rt = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(args.wcount)
        .build()?;
    trace!(?rt, "tokio runtime created");

    // shutdown mechanism courtesy of https://github.com/tokio-rs/mini-redis
    rt.block_on(async move {
        // When the provided `shutdown` future completes, we must send a shutdown
        // message to all active connections. We use a broadcast channel for this
        // purpose. The call below ignores the receiver of the broadcast pair, and when
        // a receiver is needed, the subscribe() method on the sender is used to create
        // one.
        let (notify_shutdown, _) = broadcast::channel(1);
        let (shutdown_complete_tx, shutdown_complete_rx) = mpsc::channel(1);

        let limit_secs = args.limit_secs;
        let mut runner = Runner {
            args,
            notify_shutdown,
            shutdown_complete_rx,
            shutdown_complete_tx,
        };
        tokio::select! {
            res = runner.run() => {
                if let Err(err) = res {
                    error!(?err, "nailgun exited with an error");
                }
            },
            res = sig() => {
                info!("caught signal handler-- exiting");
                if let Err(err) = res {
                    error!(?err);
                }
            },
            _ = time::sleep(Duration::from_secs(limit_secs)), if limit_secs != 0 => {
                trace!("limit reached-- exiting");
            }
        }
        let Runner {
            mut shutdown_complete_rx,
            shutdown_complete_tx,
            notify_shutdown,
            ..
        } = runner;
        trace!("sending shutdown signal");
        // When `notify_shutdown` is dropped, all tasks which have `subscribe`d will
        // receive the shutdown signal and can exit
        drop(notify_shutdown);
        // Drop final `Sender` so the `Receiver` below can complete
        drop(shutdown_complete_tx);

        // Wait for all active connections to finish processing. As the `Sender`
        // handle held by the listener has been dropped above, the only remaining
        // `Sender` instances are held by connection handler tasks. When those drop,
        // the `mpsc` channel will close and `recv()` will return `None`.
        let _ = shutdown_complete_rx.recv().await;

        Ok::<(), anyhow::Error>(())
    })?;

    Ok(())
}

async fn sig() -> Result<()> {
    signal::ctrl_c().await.map_err(|err| anyhow!(err))
}

#[derive(Debug)]
pub struct Runner {
    args: Args,
    notify_shutdown: broadcast::Sender<()>,
    shutdown_complete_rx: mpsc::Receiver<()>,
    shutdown_complete_tx: mpsc::Sender<()>,
}

impl Runner {
    pub async fn run(&mut self) -> Result<()> {
        let len = self.args.wcount * self.args.tcount;
        let mut handles = Vec::with_capacity(len);

        let (stats_tx, rx) = mpsc::channel(len);
        let mut stats = StatsRunner::new(rx, len);
        tokio::spawn(async move { stats.run().await });

        for i in 0..len {
            let mut gen = Generator {
                config: self.args.to_config().await?,
                // Receive shutdown notifications.
                shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
                // Notifies the receiver half once all clones are
                // dropped.
                _shutdown_complete: self.shutdown_complete_tx.clone(),
                // stats sender
                stats_tx: stats_tx.clone(),
            };
            trace!(
                "spawning generator {} with QPS {}",
                i,
                gen.config.rate_per_gen()
            );
            let handle = tokio::spawn(async move {
                if let Err(err) = gen.run().await {
                    error!(?err, "generator exited with error");
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await?;
        }
        Ok(())
    }
}
