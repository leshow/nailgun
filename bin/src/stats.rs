use std::{
    borrow::Cow,
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use tracing::info;
use trust_dns_proto::op::ResponseCode;

use crate::gen::AtomicStore;

#[derive(Debug)]
pub struct StatsTracker {
    pub recv: u64,
    pub atomic_store: Arc<AtomicStore>,
    latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
    totals: Totals,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(0),
            max_latency: Duration::from_micros(0),
            recv: 0,
            atomic_store: Arc::new(AtomicStore::default()),
            totals: Totals::default(),
        }
    }
}
impl StatsTracker {
    pub fn reset(&mut self) {
        self.min_latency = Duration::from_micros(u64::max_value());
        self.max_latency = Duration::from_micros(0);
        self.latency = Duration::from_micros(0);
        self.recv = 0;
        self.atomic_store.reset();
    }

    fn avg_latency(&self) -> Cow<'static, str> {
        if self.recv != 0 {
            Cow::Owned(((self.latency.as_micros() / self.recv as u128) as f32 / 1_000.).to_string())
        } else {
            Cow::Borrowed("-")
        }
    }

    pub fn update(&mut self, sent: Instant, rcode: u8) {
        if let Some(latency) = Instant::now().checked_duration_since(sent) {
            self.latency += latency;
            self.min_latency = self.min_latency.min(latency);
            self.max_latency = self.max_latency.max(latency);
        }
    }

    fn min_latency(&self) -> f32 {
        if self.min_latency.as_micros() == u64::max_value() as u128 {
            0.
        } else {
            self.min_latency.as_micros() as f32 / 1_000.
        }
    }

    fn max_latency(&self) -> f32 {
        self.max_latency.as_micros() as f32 / 1_000.
    }

    pub fn log_stats(
        &self,
        elapsed: Duration,
        total_duration: Duration,
        in_flight: usize,
        ids: usize,
    ) {
        info!(
            // elapsed = &elapsed.as_secs_f32().to_string()[0..4],
            duration = &total_duration.as_secs_f32().to_string()[0..4],
            sent = self.atomic_store.sent.load(atomic::Ordering::Relaxed),
            recv = %self.recv,
            timeouts = self.atomic_store.timed_out.load(atomic::Ordering::Relaxed),
            min = %self.min_latency(),
            avg = %self.avg_latency(),
            max = %self.max_latency(),
            in_flight,
            ids
        )
    }
}

struct StatsInterval {
    duration: Duration,
    elapsed: Duration,
    in_flight: usize,
    ids: usize,
    sent: u64,
    recv: u64,
    timeouts: u64,
    min_latency: f32,
    avg_latency: f32,
    max_latency: f32,
}

#[derive(Debug, Default)]
struct Totals {
    timeouts: u64,
    sent: u64,
}
