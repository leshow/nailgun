use std::{
    collections::VecDeque,
    convert::TryFrom,
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{self, AtomicU64},
        Arc,
    },
    time::{Duration, Instant as StdInstant},
};

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use bytes::BytesMut;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use parking_lot::Mutex;
use rand::{seq::SliceRandom, thread_rng};
use rustc_hash::FxHashMap;
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::mpsc,
    task::JoinHandle,
    time::{self, Instant},
};
use tokio_stream::{Stream, StreamExt};
use tokio_util::{
    codec::{BytesCodec, FramedRead},
    udp::UdpFramed,
};
use tracing::{error, trace};
use trust_dns_proto::rr::Name;

use crate::{
    args::{Args, Protocol},
    config::Config,
    msg::{BufMsg, TcpDecoder},
    sender::{MsgSend, Sender},
    shutdown::Shutdown,
    stats::{StatsInterval, StatsTracker},
};

#[derive(Debug)]
pub struct Generator {
    pub config: Config,
    pub shutdown: Shutdown,
    pub _shutdown_complete: mpsc::Sender<()>,
}

#[derive(Debug)]
pub struct QueryInfo {
    pub sent: StdInstant,
}

#[derive(Debug)]
pub struct Store {
    pub ids: VecDeque<u16>,
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

#[derive(Debug, Default)]
pub struct AtomicStore {
    pub sent: AtomicU64,
    pub timed_out: AtomicU64,
}

impl AtomicStore {
    pub fn reset(&self) {
        self.sent.store(0, atomic::Ordering::Relaxed);
        self.timed_out.store(0, atomic::Ordering::Relaxed);
    }
}

impl Generator {
    pub async fn run(&mut self) -> Result<StatsInterval> {
        let store = Arc::new(Mutex::new(Store::new()));
        let mut stats = StatsTracker::default();

        let bucket = self.config.rate_limiter();
        let (mut reader, mut sender) = match self.config.protocol {
            // TODO: perhaps we shouldn't do this abstraction because we should be opening
            // more TcpStreams instead of one per generator
            Protocol::Tcp => {
                TcpGen::build(&store, &stats.atomic_store, &self.config, bucket).await?
            }
            Protocol::Udp => {
                UdpGen::build(&store, &stats.atomic_store, &self.config, bucket).await?
            }
        };
        let sender_handle = tokio::spawn(async move {
            if let Err(err) = sender.run().await {
                error!(?err, "Error in send task");
            }
        });
        // cleanup
        let cleanup_handle = self.cleanup_task(Arc::clone(&store), Arc::clone(&stats.atomic_store));

        // timers
        let sleep = time::sleep(Duration::from_secs(1));
        tokio::pin!(sleep);
        let mut interval = Instant::now();
        let mut total_duration = Duration::from_millis(0);

        let log = |interval| {
            let now = Instant::now();
            let elapsed = now.duration_since(interval);
            let store = store.lock();
            let in_flight = store.in_flight.len();
            let ids = store.ids.len();
            drop(store);
            (now, elapsed, in_flight, ids)
        };
        while !self.shutdown.is_shutdown() {
            tokio::select! {
                res = reader.next() => {
                    let frame = match res {
                        Some(frame) => frame,
                        None => return Ok(stats.totals())
                    };
                    if let Ok((buf, addr)) = frame {
                        let msg = BufMsg::new(buf.freeze(), addr);
                        let id = msg.msg_id();
                        let mut store = store.lock();
                        store.ids.push_back(id);
                        if let Some(qinfo) = store.in_flight.remove(&id) {
                            stats.update(qinfo.sent, &msg);
                        }
                        drop(store);
                    }
                    stats.recv += 1;
                },
                () = &mut sleep => {
                    let (now, elapsed, in_flight, ids) = log(interval);
                    total_duration += elapsed;
                    interval = now;
                    stats.log_stats(elapsed, total_duration, in_flight, ids);
                    // reset the timer
                    sleep.as_mut().reset(now + Duration::from_secs(1));
                },
                _ = self.shutdown.recv() => {
                    trace!("shutdown received");
                    // abort sender
                    sender_handle.abort();
                    // final log
                    let (_, elapsed, in_flight, ids) = log(interval);
                    total_duration += elapsed;
                    stats.log_stats(elapsed, total_duration, in_flight, ids);
                    // wait for remaining in flight messages or timeouts
                    self.wait_in_flight(store).await;
                    // abort cleanup
                    cleanup_handle.abort();
                    return Ok(stats.totals());
                }
            }
        }

        Ok(stats.totals())
    }
    async fn wait_in_flight(&self, store: Arc<Mutex<Store>>) {
        let mut interval = time::interval(Duration::from_millis(10));
        let start_wait = StdInstant::now();
        loop {
            interval.tick().await;
            let store = store.lock();
            let empty = store.in_flight.is_empty();
            drop(store);
            let now = StdInstant::now();
            if empty || (now - start_wait > self.config.timeout) {
                trace!("waited {:?} for cleanup", now - start_wait);
                return;
            }
        }
    }
    fn cleanup_task(
        &self,
        store: Arc<Mutex<Store>>,
        atomic_store: Arc<AtomicStore>,
    ) -> JoinHandle<()> {
        let timeout = self.config.timeout;
        tokio::spawn(async move {
            let mut sleep = time::interval(timeout);
            sleep.tick().await;
            loop {
                sleep.tick().await;
                let mut store = store.lock();
                let now = StdInstant::now();
                let mut ids = Vec::new();
                // remove all timed out ids from in_flight
                store.in_flight.retain(|id, info| {
                    if now - info.sent >= timeout {
                        ids.push(*id);
                        false
                    } else {
                        true
                    }
                });
                atomic_store
                    .timed_out
                    .fetch_add(ids.len() as u64, atomic::Ordering::Relaxed);
                // add back the ids so they can be used
                store.ids.extend(ids);
                drop(store);
            }
        })
    }
}

// create a stack array of random u16's
fn create_and_shuffle() -> VecDeque<u16> {
    let mut data: Vec<u16> = (0..u16::max_value()).collect();
    data.shuffle(&mut thread_rng());
    VecDeque::from(data)
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
            timeout: Duration::from_secs(args.timeout),
            generators: args.tcount * args.wcount,
        })
    }
}

struct TcpGen;
struct UdpGen;

#[async_trait]
trait BuildGen {
    async fn build(
        store: &Arc<Mutex<Store>>,
        atomic_store: &Arc<AtomicStore>,
        config: &Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(
        Pin<Box<dyn Stream<Item = io::Result<(BytesMut, SocketAddr)>> + Send>>,
        Sender,
    )>;
}

type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

#[async_trait]
impl BuildGen for TcpGen {
    async fn build(
        store: &Arc<Mutex<Store>>,
        atomic_store: &Arc<AtomicStore>,
        config: &Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        trace!("building TCP generator");
        // TODO: shutdown ?
        let (r, s) = TcpStream::connect(config.addr).await?.into_split();
        let reader = FramedRead::new(r, TcpDecoder { addr: config.addr });
        let sender = Sender {
            config: config.clone(),
            s: MsgSend::Tcp { s },
            store: Arc::clone(&store),
            atomic_store: Arc::clone(&atomic_store),
            bucket,
        };
        Ok((Box::pin(reader), sender))
    }
}

#[async_trait]
impl BuildGen for UdpGen {
    async fn build(
        store: &Arc<Mutex<Store>>,
        atomic_store: &Arc<AtomicStore>,
        config: &Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        trace!("building UDP generator");
        let src: SocketAddr = match config.addr {
            SocketAddr::V4(_) => ([0, 0, 0, 0], 0).into(),
            SocketAddr::V6(_) => ("::0".parse::<IpAddr>()?, 0).into(),
        };
        let r = Arc::new(UdpSocket::bind(src).await?);
        let s = Arc::clone(&r);

        let recv = UdpFramed::new(r, BytesCodec::new());
        let sender = Sender {
            config: config.clone(),
            s: MsgSend::Udp {
                s,
                addr: config.addr,
            },
            store: Arc::clone(&store),
            atomic_store: Arc::clone(&atomic_store),
            bucket,
        };
        Ok((Box::pin(recv), sender))
    }
}
