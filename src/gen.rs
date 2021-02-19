use std::{
    convert::TryFrom,
    mem::{self, MaybeUninit},
    net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6},
    path::PathBuf,
    ptr,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rustc_hash::FxHashMap;
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc},
    task::JoinHandle,
};
use tokio_stream::StreamExt;
use tokio_util::codec::BytesCodec;
use tracing::{error, info, trace};
use trust_dns_proto::{
    op::{Message, MessageType, Query},
    rr::{Name, RecordType},
};
// this one is my very own:
use tokio_udp_framed::UdpFramedRecv;

use crate::{
    args::{Args, Protocol},
    msg::BufMsg,
    shutdown::Shutdown,
};

#[derive(Debug)]
pub(crate) struct Generator {
    // pub(crate) random_ids: Vec<u16>,
    pub(crate) config: Config,
    pub(crate) shutdown: Shutdown,
    pub(crate) _shutdown_complete: mpsc::Sender<()>,
}

#[derive(Debug, Clone)]
pub(crate) struct Config {
    protocol: Protocol,
    addr: SocketAddr,
    record: Name,
    qtype: RecordType,
    qps: usize,
    delay_ms: Duration,
    timeout: Duration,
    batch_size: usize,
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
            delay_ms: Duration::from_millis(args.delay_ms),
            timeout: Duration::from_secs(args.timeout),
            // TODO: in flamethrower batch is decided by protocol
            batch_size: 1000,
            file: args.file.clone(),
        })
    }
}

impl Generator {
    pub(crate) async fn run(&mut self) -> Result<()> {
        // choose src based on addr
        let src: SocketAddr = match self.config.addr {
            SocketAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            SocketAddr::V6(_) => ("::0".parse::<IpAddr>()?, 0).into(),
        };
        let r = Arc::new(UdpSocket::bind(src).await?);
        let s = Arc::clone(&r);
        let (tx, mut rx) = mpsc::channel(10_000);

        let mut sender = UdpSender {
            config: self.config.clone(),
            s,
            tx,
            ids: create_and_shuffle(),
        };
        let sender_handle = tokio::spawn(async move {
            if let Err(err) = sender.run().await {
                error!(?err, "Error in UDP send task");
            }
        });

        let mut in_flight: FxHashMap<u16, ()> = FxHashMap::default();
        let mut recv = UdpFramedRecv::new(r, BytesCodec::new());
        while !self.shutdown.is_shutdown() {
            // While reading a request frame, also listen for the shutdown
            // signal.
            tokio::select! {
                Some(id) = rx.recv() => {
                    in_flight.insert(id, ());
                },
                res = recv.next() => {
                    let frame = match res {
                        Some(frame) => frame,
                        None => return Ok(())
                    };
                    if let Ok((buf, addr)) = frame {
                        let msg= BufMsg::new(buf.freeze(), addr);
                        let id = msg.msg_id();
                        in_flight.remove(&id);
                    }
                },
                _ = self.shutdown.recv() => {
                    // kill sender task
                    sender_handle.abort();
                    // If a shutdown signal is received, return from `run`.
                    // This will result in the task terminating.
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

pub(crate) struct UdpSender {
    config: Config,
    s: Arc<UdpSocket>,
    tx: mpsc::Sender<u16>,
    ids: Vec<u16>,
}

impl UdpSender {
    async fn run(&mut self) -> Result<()> {
        loop {
            for _ in 0..self.config.batch_size {
                if let Some(next_id) = self.ids.pop() {
                    // TODO: make message generation better
                    let msg = QueryGen::gen(next_id, self.config.record.clone(), self.config.qtype);
                    self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                    // should this be try_send?
                    self.tx.send(next_id).await?;
                }
            }
            tokio::time::sleep(self.config.delay_ms).await;
        }
    }
}

// create a stack array of random u16's
fn create_and_shuffle() -> Vec<u16> {
    let mut data: Vec<u16> = (0..u16::max_value()).collect();
    data.shuffle(&mut thread_rng());

    data
}

pub(crate) struct QueryGen;

impl QueryGen {
    pub(crate) fn gen(id: u16, record: Name, qtype: RecordType) -> Message {
        let mut msg = Message::new();
        msg.set_id(id)
            .add_query(Query::query(record, qtype))
            .set_message_type(MessageType::Query);
        msg
    }
}
