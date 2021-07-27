use std::{
    io,
    net::{IpAddr, SocketAddr},
    pin::Pin,
    sync::{
        atomic::{self},
        Arc,
    },
    time::{Duration, Instant as StdInstant},
};

use anyhow::Result;
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
use tokio_stream::{Stream, StreamExt};
use tokio_util::{
    codec::{BytesCodec, FramedRead},
    udp::UdpFramed,
};
use tracing::{error, trace};

use crate::{
    args::Protocol,
    config::Config,
    msg::{BufMsg, TcpDecoder},
    query::{FileGen, QueryGen, Source, StaticGen},
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
        let query_gen: Box<dyn QueryGen + Send> = match &self.config.query_src {
            Source::File(p) => {
                trace!("using file gen on path {:#?}", p);
                Box::new(FileGen::new(p)?)
            }
            Source::Static { name, qtype } => {
                trace!("using static gen with {} {:?}", name, qtype);
                Box::new(StaticGen::new(name.clone(), *qtype))
            }
        };
        let (mut reader, mut sender) = match self.config.protocol {
            // TODO: Probably not realistic that each generator maintains one TcpStream.
            // Would need to drop this abstraction in order to have tcp gen open/close multiple
            // connections?
            Protocol::Tcp => {
                TcpGen::build(&store, &stats.atomic_store, &self.config, query_gen, bucket).await?
            }
            Protocol::Udp => {
                UdpGen::build(&store, &stats.atomic_store, &self.config, query_gen, bucket).await?
            }
        };
        let mut sender = tokio::spawn(async move {
            if let Err(err) = sender.run().await {
                error!(?err, "Error in send task");
            }
        });
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
                            // never recv nothing is removed, so there is no
                            // sent buf_size
                            //
                            // two ideas to make this better & fix a few other things:
                            // 1 - make a Arc<Mutex<Stats>> and share with all
                            // generators
                            // 2 - make a Stats actor and use mpsc chans.
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
                    stats.log_stats(elapsed, total_duration, in_flight, ids);
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
                    self.wait_in_flight(&store).await;
                    // final log
                    let (_, elapsed, in_flight, ids) = log(interval);
                    total_duration += elapsed;
                    stats.log_stats(elapsed, total_duration, in_flight, ids);
                    // abort cleanup
                    cleanup.abort();
                    self.stats_tx.send(stats.totals()).await?;
                    return Ok(());
                }
            }
        }

        // we should only ever exit via shutdown.recv()
        Ok(())
    }

    async fn wait_in_flight(&self, store: &Arc<Mutex<Store>>) {
        let timeout = self.config.timeout;
        let mut interval = time::interval(Duration::from_millis(10));
        let start_wait = StdInstant::now();
        loop {
            interval.tick().await;
            let store = store.lock();
            let empty = store.in_flight.is_empty();
            drop(store);
            let now = StdInstant::now();
            if empty || (now - start_wait > timeout) {
                trace!("waited {:?} for cleanup", now - start_wait);
                return;
            }
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

struct TcpGen;
struct UdpGen;

#[async_trait]
trait BuildGen {
    async fn build(
        store: &Arc<Mutex<Store>>,
        atomic_store: &Arc<AtomicStore>,
        config: &Config,
        query_gen: Box<dyn QueryGen + Send>,
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
        query_gen: Box<dyn QueryGen + Send>,
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
            config: config.clone(),
            query_gen,
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
        query_gen: Box<dyn QueryGen + Send>,
        bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ) -> Result<(BoxStream<io::Result<(BytesMut, SocketAddr)>>, Sender)> {
        trace!("building UDP generator");
        // bind to specified addr (will be 0.0.0.0 in most cases)
        let r = Arc::new(UdpSocket::bind((config.bind, 0)).await?);
        let s = Arc::clone(&r);

        let recv = UdpFramed::new(r, BytesCodec::new());
        let sender = Sender {
            query_gen,
            config: config.clone(),
            s: MsgSend::Udp {
                s,
                target: config.target,
            },
            store: Arc::clone(&store),
            atomic_store: Arc::clone(&atomic_store),
            bucket,
        };
        Ok((Box::pin(recv), sender))
    }
}
