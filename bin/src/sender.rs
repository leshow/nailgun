use std::{
    sync::{atomic, Arc},
    time::Instant,
};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::net::UdpSocket;

use crate::{
    gen::{AtomicStore, Config, QueryInfo, Store},
    query,
};
use tokenbucket::TokenBucket;

#[derive(Debug)]
pub(crate) struct UdpSender {
    pub(crate) config: Config,
    pub(crate) s: Arc<UdpSocket>,
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) atomic_store: Arc<AtomicStore>,
    pub(crate) bucket: TokenBucket,
}

impl UdpSender {
    pub(crate) async fn run(&mut self) -> Result<()> {
        loop {
            for _ in 0..self.config.batch_size {
                self.bucket.consume(1, Instant::now());
                // have to structure like this to not hold mutex over await
                let id = {
                    let mut store = self.store.lock();
                    match store.ids.pop() {
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
                    }
                };
                if let Some(next_id) = id {
                    let msg = query::simple(next_id, self.config.record.clone(), self.config.qtype);
                    self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                    self.atomic_store
                        .count
                        .fetch_add(1, atomic::Ordering::Relaxed);
                }
            }
            // task::yield_now().await;
            tokio::time::sleep(self.config.delay_ms).await;
        }
    }
}
