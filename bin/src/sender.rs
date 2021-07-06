use std::{
    num::NonZeroU32,
    sync::{atomic, Arc},
    time::Instant,
};

use anyhow::Result;
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
    gen::{AtomicStore, QueryInfo, Store},
    query,
};

#[derive(Debug)]
pub struct UdpSender {
    pub config: Config,
    pub s: Arc<UdpSocket>,
    pub store: Arc<Mutex<Store>>,
    pub atomic_store: Arc<AtomicStore>,
    pub bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl UdpSender {
    pub async fn run(&mut self) -> Result<()> {
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
                    bucket.until_n_ready(NonZeroU32::new(1).unwrap()).await?;
                }
                // TODO: better way to do this that locks less?
                {
                    self.store.lock().in_flight.insert(
                        next_id,
                        QueryInfo {
                            sent: Instant::now(),
                        },
                    );
                }
                let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype);
                self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                self.atomic_store
                    .sent
                    .fetch_add(1, atomic::Ordering::Relaxed);
            }
        }
    }
}

#[derive(Debug)]
pub struct TcpSender {
    pub config: Config,
    pub s: OwnedWriteHalf,
    pub store: Arc<Mutex<Store>>,
    pub atomic_store: Arc<AtomicStore>,
    pub bucket: Option<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}
impl TcpSender {
    pub async fn run(&mut self) -> Result<()> {
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
                    bucket.until_n_ready(NonZeroU32::new(1).unwrap()).await?;
                }
                // TODO: better way to do this that locks less?
                {
                    self.store.lock().in_flight.insert(
                        next_id,
                        QueryInfo {
                            sent: Instant::now(),
                        },
                    );
                }
                let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype)
                    .to_vec()?;
                // write the message out
                self.s.write_u16(msg.len() as u16).await;
                self.s.write_all(&msg).await;

                self.atomic_store
                    .sent
                    .fetch_add(1, atomic::Ordering::Relaxed);
            }
        }
    }
}
