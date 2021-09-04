use std::{
    net::SocketAddr,
    sync::{atomic, Arc},
    time::Instant,
};

use anyhow::{Context, Result};
use bytes::{Bytes, BytesMut};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use parking_lot::Mutex;
use tokio::{
    io::AsyncWriteExt,
    net::{tcp::OwnedWriteHalf, UdpSocket},
    sync::mpsc,
    task,
};
use trust_dns_proto::{
    error::ProtoError,
    https::HttpsClientStream,
    op::Message,
    xfer::{DnsRequest, DnsRequestSender, DnsResponse, FirstAnswer},
};

use crate::{
    config::Config,
    query::QueryGen,
    store::{AtomicStore, QueryInfo, Store},
};

pub struct Sender {
    pub config: Config,
    pub s: MsgSend,
    pub store: Arc<Mutex<Store>>,
    pub atomic_store: Arc<AtomicStore>,
    pub bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl Sender {
    pub async fn run<T: QueryGen>(&mut self, mut query_gen: T) -> Result<()> {
        loop {
            let num = self.config.batch_size();
            task::yield_now().await;
            let ids = {
                let mut store = self.store.lock();
                (0..num)
                    .flat_map(|_| match store.ids.pop_front() {
                        Some(id) if !store.in_flight.contains_key(&id) => Some(id),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            for next_id in ids {
                if let Some(bucket) = &self.bucket {
                    bucket.until_ready().await;
                }
                let msg = query_gen
                    .next_msg(next_id)
                    .context("query gen ran out of msgs")?;
                // let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype)
                // .to_vec()?;
                // TODO: better way to do this that locks less?
                {
                    self.store.lock().in_flight.insert(
                        next_id,
                        QueryInfo {
                            sent: Instant::now(),
                            len: msg.len(),
                        },
                    );
                }
                // could be done with an async trait and Sender<S: MsgSender>
                // but this just seems easier for now
                self.s.send(&msg[..]).await?;
                self.atomic_store
                    .sent
                    .fetch_add(1, atomic::Ordering::Relaxed);
            }
        }
    }
}

// a very simple DNS message sender
pub enum MsgSend {
    Tcp {
        s: OwnedWriteHalf,
    },
    Udp {
        s: Arc<UdpSocket>,
        target: SocketAddr,
    },
    Doh {
        s: mpsc::Sender<Message>,
    },
}

impl MsgSend {
    async fn send(&mut self, msg: &[u8]) -> Result<()> {
        match self {
            MsgSend::Tcp { s } => {
                // write the message out
                s.write_u16(msg.len() as u16).await?;
                s.write_all(msg).await?;
            }
            MsgSend::Udp { s, target } => {
                s.send_to(msg, *target).await?;
            }
            MsgSend::Doh { s } => {
                s.send(todo!()).await?;
            }
        }
        Ok(())
    }
}

pub struct DohSender {
    rx: mpsc::Receiver<Message>,
    stream: HttpsClientStream,
    resp_tx: mpsc::Sender<(Bytes, SocketAddr)>,
}

impl DohSender {
    pub async fn new(
        target: SocketAddr,
    ) -> (
        Self,
        mpsc::Sender<Message>,
        mpsc::Receiver<(Bytes, SocketAddr)>,
    ) {
        let (tx, rx) = mpsc::channel(100_000);
        let (resp_tx, resp_rx) = mpsc::channel(100_000);
        (
            Self {
                rx,
                resp_tx,
                stream: todo!(),
            },
            tx,
            resp_rx,
        )
    }
    pub async fn run(&mut self) -> Result<()> {
        while let Some(msg) = self.rx.recv().await {
            let req = DnsRequest::new(todo!(), Default::default());
            let stream = self.stream.send_message(req);
            let tx = self.resp_tx.clone();
            tokio::spawn(async move {
                let resp = stream.first_answer().await;
                // TODO: unfortunately (for us) trustdns is deserializing the message
                // so we have to convert it back to a vec in order to use it..
                if let Ok(resp) = resp {
                    // tx.send(resp).await?;
                    todo!();
                }
            });
        }
        Ok(())
    }
}
