use std::{
    convert::TryFrom,
    mem::{self, MaybeUninit},
    net::{IpAddr, SocketAddr, SocketAddrV4, SocketAddrV6},
    path::PathBuf,
    ptr,
    sync::Arc,
};

use anyhow::{anyhow, Context, Result};
use bytes::Bytes;
use rand::seq::SliceRandom;
use rand::thread_rng;
use rustc_hash::FxHashMap;
use tokio::{
    net::UdpSocket,
    sync::{broadcast, mpsc},
};
use tokio_stream::StreamExt;
use tokio_util::codec::BytesCodec;
use tracing::{error, info, trace};
use trust_dns_proto::rr::{Name, RecordType};
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
        let mut ids = {
            let mut rng = thread_rng();
            let mut data: [MaybeUninit<u16>; u16::max_value() as usize] =
                unsafe { MaybeUninit::uninit().assume_init() };

            for (i, elem) in &mut data[..].iter_mut().enumerate() {
                unsafe {
                    ptr::write(elem.as_mut_ptr(), i as u16);
                }
            }

            let mut data = unsafe {
                mem::transmute::<
                    [MaybeUninit<u16>; u16::max_value() as usize],
                    [u16; u16::max_value() as usize],
                >(data)
            };
            data.shuffle(&mut rng);
            // should we just:
            // let mut data: Vec<u16> = (0..u16::max_value()).collect();
            // data.shuffle(&mut thread_rng());
        };
        let in_flight: FxHashMap<u16, u16> = FxHashMap::default();

        // choose src based on addr
        let src: SocketAddr = match self.config.addr {
            SocketAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            SocketAddr::V6(_) => ("::0".parse::<IpAddr>()?, 0).into(),
        };
        let ns_recv = Arc::new(UdpSocket::bind(src).await?);
        let ns_send = Arc::clone(&ns_recv);
        let (tx, rx) = mpsc::channel(10_000);

        tokio::spawn(async move {
            if let Err(err) = udp_send_chan(rx, ns_send).await {
                error!(?err, "Error in UDP send task");
            }
        });
        let mut recv = UdpFramedRecv::new(ns_recv, BytesCodec::new());
        while !self.shutdown.is_shutdown() {
            // While reading a request frame, also listen for the shutdown
            // signal.
            tokio::select! {
                res = recv.next() => {
                    let frame = match res {
                        Some(frame) => frame,
                        None => return Ok(())
                    };
                    if let Ok((buf, addr)) = frame {
                        let msg= BufMsg::new(buf.freeze(), addr);
                        let id = msg.msg_id();
                        // TODO maybe do something after we remove?
                        // in_flight.remove(&id);
                    }
                },
                _ = self.shutdown.recv() => {
                    // If a shutdown signal is received, return from `run`.
                    // This will result in the task terminating.
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

async fn udp_send_chan(
    mut rx: mpsc::Receiver<(Bytes, SocketAddr)>,
    ns_send: Arc<UdpSocket>,
) -> Result<()> {
    while let Some(msg) = rx.recv().await {
        if let Err(err) = ns_send.send_to(&msg.0, msg.1).await {
            error!(?err, "Error in UDP send");
            continue;
        }
    }
    Ok(())
}
