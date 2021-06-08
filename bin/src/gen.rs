use std::{
    borrow::Cow,
    convert::TryFrom,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{atomic::AtomicUsize, Arc},
    time::{Duration, Instant as StdInstant, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use rand::{seq::SliceRandom, thread_rng};
use rustc_hash::FxHashMap;
use tokio::{
    net::UdpSocket,
    sync::mpsc,
    task,
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
use tokenbucket::{AsyncTokenBucket, Builder};

#[derive(Debug)]
pub struct Generator {
    pub config: Config,
    pub shutdown: Shutdown,
    pub _shutdown_complete: mpsc::Sender<()>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub protocol: Protocol,
    pub addr: SocketAddr,
    pub record: Name,
    pub qtype: RecordType,
    pub qps: usize,
    pub delay_ms: Duration,
    pub timeout: Duration,
    pub batch_size: u32,
    pub file: Option<PathBuf>,
}

#[derive(Debug)]
pub struct Info {
    elapsed: Duration,
    in_flight: usize,
    timeouts: usize,
}

#[derive(Debug)]
pub struct QueryInfo {
    pub sent: StdInstant,
}

#[derive(Debug)]
pub struct Store {
    pub ids: Vec<u16>,
    pub in_flight: FxHashMap<u16, QueryInfo>,
}
impl Store {
    pub fn new() -> Self {
        Self {
            in_flight: FxHashMap::default(),
            ids: create_and_shuffle(),
        }
    }
}
#[derive(Debug)]
pub struct AtomicStore {
    pub count: AtomicUsize,
}

impl AtomicStore {
    pub fn new() -> Self {
        Self {
            count: AtomicUsize::new(0),
        }
    }
}

impl Generator {
    pub async fn run(&mut self) -> Result<()> {
        // choose src based on addr
        let src: SocketAddr = match self.config.addr {
            SocketAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            SocketAddr::V6(_) => ("::0".parse::<IpAddr>()?, 0).into(),
        };
        let r = Arc::new(UdpSocket::bind(src).await?);
        let s = Arc::clone(&r);
        let store = Arc::new(Mutex::new(Store::new()));
        let atomic_store = Arc::new(AtomicStore::new());

        // start token bucket
        let mut bucket = Builder::new()
            .capacity(self.config.qps)
            .rate(self.config.qps)
            .interval_secs(1)
            .build_async();
        let runner = bucket.runner();
        tokio::spawn(async move { runner.run().await });

        let mut sender = UdpSender {
            config: self.config.clone(),
            s,
            store: store.clone(),
            atomic_store,
            bucket,
        };
        let sender_handle = tokio::spawn(async move {
            if let Err(err) = sender.run().await {
                error!(?err, "Error in UDP send task");
            }
        });

        let mut recv = UdpFramedRecv::new(r, BytesCodec::new());
        let sleep = time::sleep(Duration::from_secs(1));
        let mut interval = Instant::now();
        let mut total_duration = Duration::from_millis(0);
        tokio::pin!(sleep);
        let mut stats = StatsTracker {
            recv: 0,
            latency: Duration::from_millis(0),
            min_latency: Duration::from_millis(0),
            max_latency: Duration::from_millis(0),
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
                            stats.update(qinfo.sent);
                        }
                    }
                },
                () = &mut sleep => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(interval);
                    total_duration += elapsed;
                    interval = now;
                    info!(
                        "elapsed: {}s recv: {} avg_latency: {}ms min_latency: {}ms max_latency: {}ms total_duration: {}s",
                        &elapsed.as_secs_f32().to_string()[0..4],
                        stats.recv,
                        stats.avg_latency(),
                        stats.min_latency.as_millis(),
                        stats.max_latency.as_millis(),
                        &total_duration.as_secs_f32().to_string()[0..4],

                    );
                    stats.reset();
                    // reset the timer
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
        // TODO: do something with this
        // total_duration

        Ok(())
    }
}

#[derive(Debug)]
struct StatsTracker {
    recv: u128,
    latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
}

impl StatsTracker {
    fn reset(&mut self) {
        *self = StatsTracker {
            recv: 0,
            latency: Duration::from_millis(0),
            min_latency: Duration::from_millis(u64::max_value()),
            max_latency: Duration::from_millis(0),
        };
    }

    fn avg_latency(&self) -> Cow<'static, str> {
        if self.recv != 0 {
            Cow::Owned((self.latency.as_millis() / self.recv).to_string())
        } else {
            Cow::Borrowed("-")
        }
    }

    fn update(&mut self, sent: StdInstant) {
        self.recv += 1;
        let recv = StdInstant::now().duration_since(sent);
        self.latency += recv;
        self.min_latency = self.min_latency.min(recv);
        self.max_latency = self.max_latency.max(recv);
    }
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
