use std::{
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::{net::UdpSocket, time};
use tracing::{info, trace};

use crate::{
    gen::{AtomicStore, Config, QueryInfo, Store},
    query,
};
use tokenbucket::AsyncTokenBucket;

// #[derive(Debug)]
pub struct UdpSender {
    pub config: Config,
    pub s: Arc<UdpSocket>,
    pub store: Arc<Mutex<Store>>,
    pub atomic_store: Arc<AtomicStore>,
    pub bucket: AsyncTokenBucket,
}

impl UdpSender {
    pub async fn run(&mut self) -> Result<()> {
        let sleep = time::sleep(Duration::from_secs(1));
        let mut interval = tokio::time::Instant::now();
        let mut total_duration = tokio::time::Duration::from_millis(0);
        tokio::pin!(sleep);
        let mut sent = 0;
        loop {
            tokio::select! {
                Ok(()) = self.bucket.tokens(self.config.batch_size) => {
                    // trace!(batch_size = self.config.batch_size, "acquiring tokens");
                    // self.bucket.tokens(self.config.batch_size).await?;
                    let ids = {
                        let mut store = self.store.lock();
                        // TODO: decide whether to use vec or just lock more often?
                        (0..self.config.batch_size)
                            .flat_map(|_| match store.ids.pop() {
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
                    sent += ids.len();
                    for next_id in ids {
                        let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype);
                        self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                        self.atomic_store
                            .count
                            .fetch_add(1, atomic::Ordering::Relaxed);
                    }
                    // TODO: this sleep will affect the QPS
                    // tokio::time::sleep(self.config.delay_ms).await;
                },
                () = &mut sleep => {
                    let now = tokio::time::Instant::now();
                    let elapsed = now.duration_since(interval);
                    total_duration += elapsed;
                    interval = now;
                    info!(
                        "elapsed: {}s sent: {} total_duration: {}s",
                        &elapsed.as_secs_f32().to_string()[0..4],
                        sent,
                        &total_duration.as_secs_f32().to_string()[0..4],

                    );
                    sent = 0;
                    // reset the timer
                    sleep.as_mut().reset(now + Duration::from_secs(1));
                },
            }
        }
    }
}
