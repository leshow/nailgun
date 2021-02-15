#![warn(
    missing_debug_implementations,
    // missing_docs, // we shall remove thee, someday!
    rust_2018_idioms,
    unreachable_pub,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use anyhow::{anyhow, Result};
use clap::Clap;
use tokio::{runtime::Builder, signal};
use tracing::{error, info, trace};

mod args;

use args::Args;

fn main() -> Result<()> {
    trace!("parsing cli args");
    let args = Args::parse();
    let rt = Builder::new_multi_thread()
        .enable_all()
        .worker_threads(1)
        .build()?;
    info!(?rt, "tokio runtime created");

    rt.block_on(async {
        tokio::select! {
           res = start() => {
                if let Err(err) = res {
                    error!(?err, "nailgun exited with failure");
                }
           },
           res = sig() => {
            error!(?res, "nailgun exited with failure")
           }
        }
    });
    Ok(())
}

async fn start() -> Result<()> {
    // TODO: stuff
    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    Ok(())
}
async fn sig() -> Result<()> {
    signal::ctrl_c().await.map_err(|err| anyhow!(err))
}
