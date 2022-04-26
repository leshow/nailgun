use std::{
    io,
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
    sync::mpsc,
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
                self.s.send(msg).await?;
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
        s: mpsc::Sender<Vec<u8>>,
    },
}

impl MsgSend {
    async fn send(&mut self, msg: Vec<u8>) -> Result<()> {
        match self {
            MsgSend::Tcp { s } => {
                // write the message out
                s.write_u16(msg.len() as u16).await?;
                s.write_all(&msg[..]).await?;
            }
            MsgSend::Udp { s, target } => {
                s.send_to(&msg[..], *target).await?;
            }
            MsgSend::Doh { s } => {
                s.send(msg).await?;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "doh")]
pub mod doh {
    use super::*;

    use bytes::Bytes;
    use rustls::{ClientConfig, KeyLogFile, OwnedTrustAnchor, RootCertStore};
    use tokio::net::TcpStream as TokioTcpStream;
    use trust_dns_proto::iocompat::AsyncIoTokioAsStd;
    use trust_dns_proto::{
        https::{HttpsClientStream, HttpsClientStreamBuilder},
        xfer::FirstAnswer,
    };

    fn client_config_tls12_webpki_roots() -> ClientConfig {
        let mut root_store = RootCertStore::empty();
        root_store.add_server_trust_anchors(webpki_roots::TLS_SERVER_ROOTS.0.iter().map(|ta| {
            OwnedTrustAnchor::from_subject_spki_name_constraints(
                ta.subject,
                ta.spki,
                ta.name_constraints,
            )
        }));

        let mut client_config = ClientConfig::builder()
            .with_safe_default_cipher_suites()
            .with_safe_default_kx_groups()
            .with_protocol_versions(&[&rustls::version::TLS12])
            .unwrap()
            .with_root_certificates(root_store)
            .with_no_client_auth();

        client_config.alpn_protocols = vec![b"h2".to_vec()];
        client_config
    }

    pub struct DohSender {
        rx: mpsc::Receiver<Vec<u8>>,
        stream: HttpsClientStream,
        addr: SocketAddr,
        resp_tx: mpsc::Sender<io::Result<(Bytes, SocketAddr)>>,
    }

    impl DohSender {
        pub async fn new(
            target: SocketAddr,
            name_server: impl Into<String>,
        ) -> Result<(
            Self,
            mpsc::Sender<Vec<u8>>,
            mpsc::Receiver<io::Result<(Bytes, SocketAddr)>>,
        )> {
            let (tx, rx) = mpsc::channel(100_000);
            let (resp_tx, resp_rx) = mpsc::channel(100_000);

            let mut client_config = client_config_tls12_webpki_roots();
            client_config.key_log = Arc::new(KeyLogFile::new());

            let https_builder =
                HttpsClientStreamBuilder::with_client_config(Arc::new(client_config));
            let stream = https_builder
                .build::<AsyncIoTokioAsStd<TokioTcpStream>>(target, name_server.into())
                .await?;

            Ok((
                Self {
                    rx,
                    resp_tx,
                    stream,
                    addr: target,
                },
                tx,
                resp_rx,
            ))
        }
        pub async fn run(&mut self) -> Result<()> {
            while let Some(msg) = self.rx.recv().await {
                let stream = self.stream.send_bytes(msg);
                let tx = self.resp_tx.clone();
                // TODO: shutdown?
                let addr = self.addr;
                tokio::spawn(async move {
                    let resp = stream.first_answer().await;
                    if let Ok(resp) = resp {
                        tx.send(Ok((resp.freeze(), addr)))
                            .await
                            .context("failed to send DOH response to gen")?;
                    }
                    Ok::<_, anyhow::Error>(())
                });
            }
            Ok(())
        }
    }
}
