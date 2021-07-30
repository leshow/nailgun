use std::{
    net::SocketAddr,
    sync::{atomic, Arc},
    time::Instant,
};

use anyhow::{Context, Result};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use parking_lot::Mutex;
use tokio::{
    io::AsyncWriteExt,
    net::{tcp::OwnedWriteHalf, UdpSocket},
    task,
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
                    .context("query gen ran out of msgs")?
                    .to_vec()?;
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
#[derive(Debug)]
pub enum MsgSend {
    Tcp {
        s: OwnedWriteHalf,
    },
    Udp {
        s: Arc<UdpSocket>,
        target: SocketAddr,
    },
}

impl MsgSend {
    async fn send(&mut self, msg: &[u8]) -> Result<()> {
        match self {
            MsgSend::Tcp { s } => {
                // write the message out
                s.write_u16(msg.len() as u16).await?;
                s.write_all(&msg).await?;
            }
            MsgSend::Udp { s, target } => {
                s.send_to(&msg, *target).await?;
            }
        }
        Ok(())
    }
}
