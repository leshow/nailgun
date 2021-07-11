#![warn(
    missing_debug_implementations,
    // missing_docs, // TODO
    rust_2018_idioms,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use std::{convert::TryFrom, time::Duration};

use anyhow::{anyhow, Result};
use clap::Clap;
use tokio::{
    runtime::Builder,
    signal,
    sync::{broadcast, mpsc},
    time,
};
use tracing::{error, info, trace};
use tracing_subscriber::{
    fmt::{self, format::Pretty},
    prelude::__tracing_subscriber_SubscriberExt,
    util::SubscriberInitExt,
    EnvFilter,
};

mod args;
mod config;
mod gen;
mod msg;
mod query;
mod sender;
mod shutdown;
mod stats;

use crate::{
    args::{Args, LogStructure},
    config::Config,
    gen::Generator,
    shutdown::Shutdown,
    stats::StatsInterval,
};

// TODO: custom logging output format?
// struct Output;
// impl<'writer> FormatFields<'writer> for Output {
//     fn format_fields<R: tracing_subscriber::prelude::__tracing_subscriber_field_RecordFields>(
//         &self,
//         writer: &'writer mut dyn std::fmt::Write,
//         fields: R,
//     ) -> std::fmt::Result {
//         todo!()
//     }
// }

fn main() -> Result<()> {
    let args = Args::parse();
    // tracing_subscriber::fmt::init();

    match args.logs {
        LogStructure::Pretty => {
            let fmt_layer = fmt::layer()
                .fmt_fields(Pretty::with_source_location(Pretty::default(), false))
                .with_target(false);
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
        LogStructure::Debug => {
            let fmt_layer = fmt::layer();
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
        LogStructure::Json => {
            let fmt_layer = fmt::layer().json();
            let filter_layer = EnvFilter::try_from_default_env()
                .or_else(|_| EnvFilter::try_new("info"))
                .unwrap();

            tracing_subscriber::registry()
                .with(filter_layer)
                .with(fmt_layer)
                .init();
        }
    }

    trace!("{:?}", args);
    let rt = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(args.wcount)
        .build()?;
    trace!(?rt, "tokio runtime created");

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
        // Extract the `shutdown_complete` receiver and transmitter
        // explicitly drop `shutdown_transmitter`. This is important, as the
        // `.await` below would otherwise never complete.
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

        let (tx, rx) = mpsc::channel(len);
        let mut stats = StatsRunner { rx, len };
        tokio::spawn(async move { stats.run().await });

        for i in 0..len {
            let mut gen = Generator {
                config: Config::try_from(&self.args)?,
                // Receive shutdown notifications.
                shutdown: Shutdown::new(self.notify_shutdown.subscribe()),
                // Notifies the receiver half once all clones are
                // dropped.
                _shutdown_complete: self.shutdown_complete_tx.clone(),
            };
            trace!(
                "spawning generator {} with QPS {}",
                i,
                gen.config.rate_per_gen()
            );
            let tx = tx.clone();
            let handle = tokio::spawn(async move {
                match gen.run().await {
                    Err(err) => {
                        error!(?err, "generator exited with error");
                        Err(err)
                    }
                    Ok(agg) => {
                        // last chance to run on exit
                        tx.send(agg).await?;
                        Ok(())
                    }
                }
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await??;
        }
        Ok(())
    }
}

// TODO: perhaps we want this to listen to generator
// stats rather than just aggregating them after shutdown

#[derive(Debug)]
struct StatsRunner {
    rx: mpsc::Receiver<StatsInterval>,
    len: usize,
}

impl StatsRunner {
    pub async fn run(&mut self) -> Result<()> {
        let mut summary = StatsInterval::default();
        let mut n = 0;
        while n < self.len {
            n += 1;
            if let Some(interval) = self.rx.recv().await {
                trace!("received stats");
                summary.update_totals(interval, n);
            }
        }
        summary.summary();
        Ok(())
    }
}
