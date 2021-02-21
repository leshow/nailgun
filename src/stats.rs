use std::time::Duration;

use anyhow::Result;
use tokio::{
    sync::mpsc,
    time::{self, Instant},
};
use tracing::trace;

use crate::shutdown::Shutdown;

#[derive(Debug)]
pub(crate) struct Stats {
    pub(crate) shutdown: Shutdown,
    pub(crate) _shutdown_complete: mpsc::Sender<()>,
}

impl Stats {
    pub(crate) async fn run(&mut self) -> Result<()> {
        let sleep = time::sleep(Duration::from_millis(10));
        let mut interval = Instant::now();
        tokio::pin!(sleep);

        while !self.shutdown.is_shutdown() {
            tokio::select! {
                () = &mut sleep => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(interval);
                    interval = now;
                    sleep.as_mut().reset(now + Duration::from_secs(1));
                },
                _ = self.shutdown.recv() => {
                    // kill sender task
                    trace!("stats shutdown received");
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}
