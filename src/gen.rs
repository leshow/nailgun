use std::{
    convert::TryFrom,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use rand::{seq::SliceRandom, thread_rng};
use rustc_hash::FxHashMap;
use tokio::{
    net::UdpSocket,
    sync::mpsc,
    task::yield_now,
    time::{self, Instant},
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
    sender::UdpSender,
    shutdown::Shutdown,
};

#[derive(Debug)]
pub(crate) struct Generator {
    pub(crate) config: Config,
    pub(crate) shutdown: Shutdown,
    pub(crate) _shutdown_complete: mpsc::Sender<()>,
}

#[derive(Debug, Clone)]
pub(crate) struct Config {
    pub(crate) protocol: Protocol,
    pub(crate) addr: SocketAddr,
    pub(crate) record: Name,
    pub(crate) qtype: RecordType,
    pub(crate) qps: usize,
    pub(crate) delay_ms: Duration,
    pub(crate) timeout: Duration,
    pub(crate) batch_size: usize,
    pub(crate) file: Option<PathBuf>,
}

#[derive(Debug)]
pub(crate) struct Info {
    elapsed: Duration,
    in_flight: usize,
    timeouts: usize,
}

#[derive(Debug)]
pub(crate) struct QueryInfo {
    pub(crate) sent: StdInstant,
}

#[derive(Debug)]
pub(crate) struct Store {
    pub(crate) ids: Vec<u16>,
    pub(crate) in_flight: FxHashMap<u16, QueryInfo>,
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
        let store = Arc::new(Mutex::new(Store {
            in_flight: FxHashMap::default(),
            ids: create_and_shuffle(),
        }));

        let sender = UdpSender {
            config: self.config.clone(),
            s,
            store: store.clone(),
        };
        let sender_handle = tokio::spawn(async move {
            if let Err(err) = sender.run().await {
                error!(?err, "Error in UDP send task");
            }
        });

        let mut recv = UdpFramedRecv::new(r, BytesCodec::new());
        let sleep = time::sleep(Duration::from_secs(1));
        let mut interval = Instant::now();
        tokio::pin!(sleep);
        let mut stats = StatsTracker {
            recv: 0,
            latency: Duration::from_millis(0),
        };

        while !self.shutdown.is_shutdown() {
            tokio::select! {
                res = recv.next() => {
                    let frame = match res {
                        Some(frame) => frame,
                        None => return Ok(())
                    };
                    if let Ok((buf, addr)) = frame {
                        let msg = BufMsg::new(buf.freeze(), addr);
                        let id = msg.msg_id();
                        let mut store = store.lock();
                        if let Some(qinfo) = store.in_flight.remove(&id) {
                            store.ids.push(id);
                            stats.recv += 1;
                            stats.latency += StdInstant::now().duration_since(qinfo.sent);
                        }
                    }
                },
                () = &mut sleep => {
                    let now = Instant::now();
                    if stats.recv != 0 {
                        let elapsed = now.duration_since(interval);
                        interval = now;
                        info!("elapsed {}s recv {} avg latency {}", &elapsed.as_secs_f32().to_string()[0..4], stats.recv, stats.latency.as_millis() / stats.recv);
                        stats.recv = 0;
                        stats.latency = Duration::from_millis(0);
                    }
                    sleep.as_mut().reset(now + Duration::from_secs(1));
                },
                _ = self.shutdown.recv() => {
                    // kill sender task
                    trace!("shutdown received");
                    sender_handle.abort();
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct StatsTracker {
    recv: u128,
    latency: Duration,
}

// create a stack array of random u16's
fn create_and_shuffle() -> Vec<u16> {
    let mut data: Vec<u16> = (0..u16::max_value()).collect();
    data.shuffle(&mut thread_rng());
    data
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
