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
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

pub mod error;
use crossbeam_channel::{Receiver, Sender};
use error::Error;
use tokio::sync::Semaphore;

pub type Result<T> = std::result::Result<T, Error>;

const DEFAULT_CAP: usize = 120;
const DEFAULT_RATE: usize = 1;
const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct Builder {
    capacity: usize,
    rate: usize,
    interval: Duration,
    initial: usize,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_CAP,
            initial: DEFAULT_CAP,
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
    ///     bucket.tokens(1).await
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

    pub fn initial(&mut self, initial: usize) -> &mut Self {
        self.initial = initial;
        self
    }

    pub fn interval_secs(&mut self, interval: u64) -> &mut Self {
        self.interval(Duration::from_secs(interval))
    }

    pub fn interval_millis(&mut self, interval: u64) -> &mut Self {
        self.interval(Duration::from_millis(interval))
    }

    pub fn build(&mut self) -> TokenBucket {
        let (tx, rx) = crossbeam_channel::bounded(self.initial);
        TokenBucket {
            capacity: self.capacity,
            rate: self.rate,
            interval: self.interval,
            tx: Some(tx),
            rx,
        }
    }

    pub fn build_async(&mut self) -> AsyncTokenBucket {
        let semaphore = Arc::new(Semaphore::new(self.initial));
        AsyncTokenBucket {
            semaphore,
            capacity: self.capacity,
            rate: self.rate,
            interval: self.interval,
        }
    }
}

#[derive(Debug)]
pub struct AsyncTokenBucket {
    capacity: usize,
    rate: usize,
    interval: Duration,
    semaphore: Arc<Semaphore>,
}

#[derive(Debug)]
pub struct AsyncTokenRunner {
    semaphore: Arc<Semaphore>,
    capacity: usize,
    rate: usize,
    interval: Duration,
}

impl AsyncTokenRunner {
    /// start runner, will add `rate` tokens every `interval`
    pub async fn run(self) -> Result<()> {
        let mut interval = tokio::time::interval(self.interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            let permits = self.semaphore.available_permits();
            if permits + self.rate > self.capacity {
                // top up the permits
                self.semaphore.add_permits(self.capacity - permits);
            } else {
                self.semaphore.add_permits(self.rate);
            }
        }
    }
}

impl AsyncTokenBucket {
    /// Create a new async runner
    pub fn runner(&mut self) -> AsyncTokenRunner {
        AsyncTokenRunner {
            semaphore: self.semaphore.clone(),
            capacity: self.capacity,
            rate: self.rate,
            interval: self.interval,
        }
    }

    /// Acquire `n` tokens
    pub async fn tokens(&self, n: u32) -> Result<()> {
        // acquire `n` tokens and forget to reduce the permit count
        // permits will be added back in `AsyncTokenRunner`
        self.semaphore.acquire_many(n).await?.forget();
        Ok(())
    }

    /// Acquire a token
    pub async fn token(&self) -> Result<()> {
        self.semaphore.acquire().await?.forget();
        Ok(())
    }
}

/// A simple blocking token bucket implementation that will
/// fill up with a `rate` number of tokens every `interval`.
/// If the bucket overfills, the unspent tokens are *not*
/// saved.
#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: usize,
    rate: usize,
    interval: Duration,
    rx: Receiver<()>,
    tx: Option<Sender<()>>,
}

#[derive(Debug)]
pub struct TokenRunner {
    capacity: usize,
    rate: usize,
    interval: Duration,
    start: Instant,
    tx: Sender<()>,
}

impl TokenRunner {
    pub fn run(mut self) -> Result<()> {
        let mut diff = Duration::from_secs(0);
        loop {
            // subtract previous extra time to adjust interval
            if self.interval > diff {
                thread::sleep(self.interval - diff);
            } // TODO: else if the diff is larger we probably want to add more tokens?
            self.start = Instant::now();
            // fill queue with next rate
            for _ in 0..self.rate {
                self.tx.send(())?;
            }
            let t1 = Instant::now();
            diff = t1.duration_since(self.start);
        }
    }
}

impl TokenBucket {
    /// ```rust
    /// use std::thread;
    /// use tokenbucket::Builder;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let mut bucket = Builder::new()
    ///         .capacity(10)
    ///         .rate(10)
    ///         .interval_secs(1)
    ///         .build();
    ///     let runner = bucket.runner()?;
    ///     thread::spawn(move || { runner.run().expect("token bucket task failed") });
    ///     // first will be available immediately
    ///     bucket.tokens(10);
    ///     // the next will take a second
    ///     bucket.tokens(10);
    /// # Ok(())
    /// }
    /// ```
    pub fn runner(&mut self) -> Result<TokenRunner> {
        if self.tx.is_none() {
            return Err(Error::AlreadyRunning);
        }
        let tx = self
            .tx
            .take()
            .unwrap(/*we just checked is_none-- unreachable*/);
        // fill channel capacity
        for _ in 0..self.capacity {
            tx.send(())?;
        }

        Ok(TokenRunner {
            rate: self.rate,
            capacity: self.capacity,
            interval: self.interval,
            start: Instant::now(), // dummy value will be overrided on run()
            tx,
        })
    }

    pub fn tokens(&self, n: usize) -> Result<()> {
        for _ in 0..n {
            self.token()?;
        }
        Ok(())
    }

    pub fn token(&self) -> Result<()> {
        self.rx.recv()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_interval() -> Result<()> {
        let mut bucket = Builder::new()
            .capacity(10)
            .rate(10)
            .interval_secs(1)
            .build();
        let runner = bucket.runner()?;
        thread::spawn(move || runner.run().expect("token bucket task failed"));
        // we start with 10 prefilled
        let t0 = Instant::now();
        bucket.tokens(10)?;
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(0).as_secs()
        );
        // the next 10 must be filled
        let t0 = Instant::now();
        bucket.tokens(10)?;
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(1).as_secs()
        );
        // the next 10 must be filled
        let t0 = Instant::now();
        bucket.tokens(10)?;
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(1).as_secs()
        );
        Ok(())
    }

    #[tokio::test]
    async fn async_test_interval() -> Result<()> {
        let mut bucket = Builder::new()
            .capacity(10)
            .rate(10)
            .interval_secs(1)
            .build_async();
        let runner = bucket.runner();
        tokio::spawn(async move { runner.run().await });
        // we start with 10 prefilled
        let t0 = Instant::now();
        bucket.tokens(10).await?;
        dbg!("a");
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(0).as_secs()
        );
        // the next 10 must be filled
        let t0 = Instant::now();
        bucket.tokens(10).await?;
        dbg!("b");
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(1).as_secs()
        );
        // the next 10 must be filled
        let t0 = Instant::now();
        bucket.tokens(10).await?;
        dbg!("c");
        assert_eq!(
            Instant::now().duration_since(t0).as_secs(),
            Duration::from_secs(1).as_secs()
        );
        Ok(())
    }
}
