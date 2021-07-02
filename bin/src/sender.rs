use std::{
    num::NonZeroU32,
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use anyhow::Result;
use governor::{
    clock::{DefaultClock, QuantaClock},
    state::{InMemoryState, NotKeyed},
    RateLimiter,
};
use parking_lot::Mutex;
use tokio::{net::UdpSocket, task, time};
use tracing::{info, trace};

use crate::{
    config::Config,
    gen::{AtomicStore, QueryInfo, Store},
    query,
};
use tokenbucket::AsyncTokenBucket;

#[derive(Debug)]
pub struct UdpSender {
    pub config: Config,
    pub s: Arc<UdpSocket>,
    pub store: Arc<Mutex<Store>>,
    pub atomic_store: Arc<AtomicStore>,
    // pub bucket: AsyncTokenBucket,
    pub bucket: RateLimiter<NotKeyed, InMemoryState, DefaultClock>,
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
                        Some(id) if !store.in_flight.contains_key(&id) => {
                            store.in_flight.insert(
                                id,
                                QueryInfo {
                                    sent: Instant::now(),
                                },
                            );
                            Some(id)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            for next_id in ids {
                self.bucket
                    .until_n_ready(NonZeroU32::new(1).unwrap())
                    .await?;
                let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype);
                self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                self.atomic_store
                    .count
                    .fetch_add(1, atomic::Ordering::Relaxed);
            }
        }
    }
}
