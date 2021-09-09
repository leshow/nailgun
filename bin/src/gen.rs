use std::{
    io,
    net::SocketAddr,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant as StdInstant},
};

use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::BytesMut;
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use parking_lot::Mutex;
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::mpsc,
    task::JoinHandle,
    time::{self, Instant},
};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tokio_util::{
    codec::{BytesCodec, FramedRead},
    udp::UdpFramed,
};
use tracing::{error, info, trace};

#[cfg(feature = "doh")]
use crate::sender::doh::DohSender;
use crate::{
    args::Protocol,
    config::Config,
    msg::{BufMsg, TcpDecoder},
    query::{FileGen, RandomPkt, RandomQName, Source, StaticGen},
    sender::{MsgSend, Sender},
    shutdown::Shutdown,
    stats::{StatsInterval, StatsTracker},
    store::{AtomicStore, Store},
};

#[derive(Debug)]
pub struct Generator {
    pub config: Config,
    pub shutdown: Shutdown,
    pub stats_tx: mpsc::Sender<StatsInterval>,
    pub _shutdown_complete: mpsc::Sender<()>,
}

impl Generator {
    pub async fn run(&mut self) -> Result<()> {
        let store = Arc::new(Mutex::new(Store::new()));
        let mut stats = StatsTracker::default();

        let bucket = self.config.rate_limiter();

        let (mut reader, sender) = {
            let store = Arc::clone(&store);
            let atomic_store = Arc::clone(&stats.atomic_store);
            let config = self.config.clone();
            match self.config.protocol {
                // TODO: Probably not realistic that each generator maintains one TcpStream.
                // Would need to drop this abstraction in order to have tcp gen open/close multiple
                // connections?
                Protocol::Tcp => TcpGen::build(store, atomic_store, config, bucket).await?,
                Protocol::Udp => UdpGen::build(store, atomic_store, config, bucket).await?,
                #[cfg(feature = "doh")]
                Protocol::DoH => DohGen::build(store, atomic_store, config, bucket).await?,
            }
        };
        let mut sender = self.sender_task(sender)?;
        let mut cleanup = self.cleanup_task(Arc::clone(&store), Arc::clone(&stats.atomic_store));

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
                        None => return Ok(())
                    };
                    if let Ok((buf, addr)) = frame {
                        let msg = BufMsg::new(buf.freeze(), addr);
                        let id = msg.msg_id();
                        let mut store = store.lock();
                        store.ids.push_back(id);
                        if let Some(qinfo) = store.in_flight.remove(&id) {
                            // TODO: there is a bug here were if we
                            // never recv then nothing is removed, so there is no
                            // sent buf_size
                            stats.update(qinfo, &msg);
                        }
                        drop(store);
                    }
                    stats.recv += 1;
                },
                () = &mut sleep => {
                    let (now, elapsed, in_flight, ids) = log(interval);
                    total_duration += elapsed;
                    interval = now;
                    // should complete immediately
                    self.stats_tx.send(stats.interval(elapsed, total_duration, in_flight, ids)).await?;
                    // reset the timer
                    sleep.as_mut().reset(now + Duration::from_secs(1));
                },
                // if any handle returns or we recv shutdown-- exit generator
                // TODO: kinda gnarly. we could pass in `shutdown`
                // separately in `run` so we can have unique access.
                _ = async {
                    tokio::select! {
                        _ = self.shutdown.recv() => {},
                        _ = &mut sender => {},
                        _ = &mut cleanup => {}
                    }
                } => {
                    trace!("shutdown received");
                    // abort sender
                    sender.abort();
                    // wait for remaining in flight messages or timeouts
                    self.wait_in_flight(&store, &stats.atomic_store).await;
                    // final log
                    let (_, elapsed, in_flight, ids) = log(interval);
                    total_duration += elapsed;
                    self.stats_tx.send(stats.interval(elapsed, total_duration, in_flight, ids)).await?;
                    // abort cleanup
                    cleanup.abort();
                    // self.stats_tx.send(stats.totals()).await?;
                    return Ok(());
                }
            }
        }

        // we should only ever exit via shutdown.recv()
        Ok(())
    }

    async fn wait_in_flight(&self, store: &Arc<Mutex<Store>>, atomic_store: &Arc<AtomicStore>) {
        // add a little extra time to timeout any in flight queries
        let timeout = self.config.timeout;
        let mut interval = time::interval(Duration::from_millis(10));
        let start_wait = StdInstant::now();
        loop {
            interval.tick().await;
            let mut store = store.lock();
            // could rely on cleanup_task?
            store.clear_timeouts(timeout, atomic_store);
            let empty = store.in_flight.is_empty();
            let now = StdInstant::now();
            if empty || (now - start_wait > timeout) {
                trace!("waited {:?} for cleanup", now - start_wait);
                return;
            }
            drop(store);
        }
    }

    // cleans up timed out message, returning them to in_flight queue
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
                store.clear_timeouts(timeout, &atomic_store);
                drop(store);
            }
        })
    }
    fn sender_task(&self, mut sender: Sender) -> Result<JoinHandle<()>> {
        // TODO: can we reduce the code duplication here without using
        // dynamic types?
        Ok(match &self.config.query_src {
            Source::File(path) => {
                info!(path = %path.display(), "using file gen");
                let query_gen = FileGen::new(path)?;

                tokio::spawn(async move {
                    if let Err(err) = sender.run(query_gen).await {
                        error!(?err, "error in send task");
                    }
                })
            }
            Source::Static { name, qtype, class } => {
                info!(%name, ?qtype, %class, "using static gen",);
                let query_gen = StaticGen::new(name.clone(), *qtype, *class);
                tokio::spawn(async move {
                    if let Err(err) = sender.run(query_gen).await {
                        error!(?err, "Error in send task");
                    }
                })
            }
            Source::RandomPkt { size } => {
                info!(size, "using randompkt gen");
                let query_gen = RandomPkt::new(*size);
                tokio::spawn(async move {
                    if let Err(err) = sender.run(query_gen).await {
                        error!(?err, "Error in send task");
                    }
                })
            }
            Source::RandomQName { size, qtype } => {
                info!(size, ?qtype, "using randomqname gen");
                let query_gen = RandomQName::new(*qtype, *size);
                tokio::spawn(async move {
                    if let Err(err) = sender.run(query_gen).await {
                        error!(?err, "Error in send task");
                    }
                })
            }
        })
    }
}

struct TcpGen;
struct UdpGen;
#[cfg(feature = "doh")]
struct DohGen;

#[async_trait]
trait BuildGen {
    async fn build(
        store: Arc<Mutex<Store>>,
        atomic_store: Arc<AtomicStore>,
        config: Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)>;
}

type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

#[async_trait]
impl BuildGen for TcpGen {
    async fn build(
        store: Arc<Mutex<Store>>,
        atomic_store: Arc<AtomicStore>,
        config: Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        trace!("building TCP generator with target {}", config.target);
        // TODO: shutdown ?
        let (r, s) = TcpStream::connect(config.target).await?.into_split();
        let reader = FramedRead::new(
            r,
            TcpDecoder {
                target: config.target,
            },
        );
        let sender = Sender {
            s: MsgSend::Tcp { s },
            config,
            store,
            atomic_store,
            bucket,
        };
        Ok((Box::pin(reader), sender))
    }
}

#[async_trait]
impl BuildGen for UdpGen {
    async fn build(
        store: Arc<Mutex<Store>>,
        atomic_store: Arc<AtomicStore>,
        config: Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        trace!("building UDP generator");
        // bind to specified addr (will be 0.0.0.0 in most cases)
        let r = Arc::new(UdpSocket::bind((config.bind, 0)).await?);
        let s = Arc::clone(&r);

        let recv = UdpFramed::new(r, BytesCodec::new());
        let sender = Sender {
            s: MsgSend::Udp {
                s,
                target: config.target,
            },
            config,
            store,
            atomic_store,
            bucket,
        };
        Ok((Box::pin(recv), sender))
    }
}

#[cfg(feature = "doh")]
#[async_trait]
impl BuildGen for DohGen {
    async fn build(
        store: Arc<Mutex<Store>>,
        atomic_store: Arc<AtomicStore>,
        config: Config,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        let (mut doh, tx, resp_rx) = DohSender::new(
            // FIXME: need to get the correct address for this, then it looks to be working
            ([1, 1, 1, 1], 443).into(),
            config
                .name_server
                .clone()
                .context("doh must always have a dns name")?,
        )
        .await?;
        // TODO: joinhandle & shutdown?
        tokio::spawn(async move {
            if let Err(err) = doh.run().await {
                error!(%err, "error in doh task");
            }
        });
        let sender = Sender {
            s: MsgSend::Doh { s: tx },
            config,
            store,
            atomic_store,
            bucket,
        };

        Ok((Box::pin(ReceiverStream::new(resp_rx)), sender))
    }
}
