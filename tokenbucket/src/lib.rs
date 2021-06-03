#![warn(
    missing_debug_implementations,
    // missing_docs, // TODO
    rust_2018_idioms,
    non_snake_case,
    non_upper_case_globals
)]
#![deny(broken_intra_doc_links)]
#![allow(clippy::cognitive_complexity)]

use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

pub mod error;
use error::Error;

pub type Result<T> = std::result::Result<T, Error>;

const DEFAULT_CAP: usize = 120;
const DEFAULT_RATE: usize = 1;
const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Builder {
    capacity: usize,
    rate: usize,
    interval: Duration,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CAP,
            rate: DEFAULT_RATE,
            interval: DEFAULT_INTERVAL,
        }
    }
}

impl Builder {
    /// ```rust
    /// #[tokio::main]
    /// async fn main() {
    ///     let mut bucket = Builder::new()
    ///         .capacity(100)
    ///         .rate(1)
    ///         .interval_secs(1)
    ///         .build();
    ///     tokio::spawn(async move { bucket.run().await.expect("token bucket task failed") });
    ///     bucket.wait()
    /// }
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    pub fn capacity(&mut self, capacity: usize) -> &mut Self {
        self.capacity = capacity;
        self
    }

    pub fn rate(&mut self, rate: usize) -> &mut Self {
        self.rate = rate;
        self
    }

    pub fn interval(&mut self, interval: Duration) -> &mut Self {
        self.interval = interval;
        self
    }

    pub fn interval_secs(&mut self, interval: u64) -> &mut Self {
        self.interval(Duration::from_secs(interval))
    }

    pub fn interval_millis(&mut self, interval: u64) -> &mut Self {
        self.interval(Duration::from_millis(interval))
    }

    pub fn build(&mut self) -> TokenBucket {
        let (tx, rx) = mpsc::sync_channel(self.capacity);
        TokenBucket {
            capacity: self.capacity,
            rate: self.rate,
            interval: self.interval,
            tx,
            rx: Some(rx),
        }
    }
}

#[derive(Debug)]
pub struct AsyncTokenBucket {
    capacity: usize,
    rate: usize,
    interval: Duration,
}

// use semaphore instead of channels?
impl AsyncTokenBucket {}

#[derive(Debug)]
pub struct TokenBucket {
    capacity: usize,
    rate: usize,
    interval: Duration,
    tx: mpsc::SyncSender<()>,
    rx: Option<mpsc::Receiver<()>>,
}

#[derive(Debug)]
pub struct TokenRunner {
    rate: usize,
    interval: Duration,
    rx: mpsc::Receiver<()>,
}

impl TokenRunner {
    pub fn run(self) -> Result<()> {
        Ok(())
    }
}

impl TokenBucket {
    /// ```rust
    /// use std::thread;
    /// use tokenbucket::Builder;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let bucket = Builder::new()
    ///         .capacity(100)
    ///         .rate(1)
    ///         .interval_secs(1)
    ///         .build();
    ///     let runner = bucket.runner();
    ///     thread::spawn(move || { runner.run().expect("token bucket task failed") });
    ///     
    ///     bucket.tokens(10);
    /// # Ok(())
    /// }
    /// ```
    pub fn runner(&mut self) -> Result<TokenRunner> {
        if self.rx.is_none() {
            return Err(Error::AlreadyRunning);
        }
        let rx = self
            .rx
            .take()
            .unwrap(/*we just checked is_none, unreachable*/);

        let runner = TokenRunner {
            rate: self.rate,
            interval: self.interval,
            rx,
        };
        // fill channel capacity
        for _ in 0..self.capacity {
            self.tx.send(())?;
        }

        Ok(runner)
    }

    pub fn tokens(&self, count: usize) -> Result<()> {
        for _ in 0..count {
            self.token()?;
        }
        Ok(())
    }

    pub fn token(&self) -> Result<()> {
        Ok(())
    }
}
