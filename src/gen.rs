use std::{convert::TryFrom, net::SocketAddr, path::PathBuf};

use anyhow::{anyhow, Context, Result};
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, trace};
use trust_dns_proto::rr::{Name, RecordType};

use crate::{
    args::{Args, Protocol},
    shutdown::Shutdown,
};

#[derive(Debug)]
pub(crate) struct Generator {
    pub(crate) config: Config,
    pub(crate) shutdown: Shutdown,
    pub(crate) _shutdown_complete: mpsc::Sender<()>,
}

#[derive(Debug)]
pub(crate) struct Config {
    protocol: Protocol,
    addr: SocketAddr,
    record: Name,
    qtype: RecordType,
    qps: usize,
    file: Option<PathBuf>,
}

impl TryFrom<&Args> for Config {
    type Error = anyhow::Error;

    fn try_from(args: &Args) -> Result<Self, Self::Error> {
        Ok(Self {
            protocol: args.protocol,
            addr: (args.ip, args.port).into(),
            record: Name::from_ascii(&args.record).map_err(|err| {
                anyhow!(
                    "failed to parse record: {:?}. with error: {:?}",
                    args.record,
                    err
                )
            })?,
            qtype: args.qtype,
            qps: args.qps,
            file: args.file.clone(),
        })
    }
}

impl Generator {
    pub(crate) async fn run(&mut self) -> Result<()> {
        self.shutdown.recv().await;
        trace!("received shutdown signal-- exiting");
        Ok(())
    }
}
