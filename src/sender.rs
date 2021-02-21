use std::{sync::Arc, time::Instant};

use anyhow::Result;
use parking_lot::Mutex;
use tokio::net::UdpSocket;
use trust_dns_proto::{
    op::{Message, MessageType, Query},
    rr::{Name, RecordType},
};

use crate::gen::{Config, QueryInfo, Store};

#[derive(Debug)]
pub(crate) struct UdpSender {
    pub(crate) config: Config,
    pub(crate) s: Arc<UdpSocket>,
    pub(crate) store: Arc<Mutex<Store>>,
}

impl UdpSender {
    pub(crate) async fn run(&self) -> Result<()> {
        loop {
            for _ in 0..self.config.batch_size {
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
                    let msg = QueryGen::gen(next_id, self.config.record.clone(), self.config.qtype);
                    self.s.send_to(&msg.to_vec()?[..], self.config.addr).await?;
                }
            }
            // tokio::task::yield_now().await;
            tokio::time::sleep(self.config.delay_ms).await;
        }
    }
}

#[derive(Debug)]
pub(crate) struct QueryGen;

impl QueryGen {
    pub(crate) fn gen(id: u16, record: Name, qtype: RecordType) -> Message {
        let mut msg = Message::new();
        msg.set_id(id)
            .add_query(Query::query(record, qtype))
            .set_message_type(MessageType::Query);
        msg
    }
}
