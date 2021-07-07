use std::{
    borrow::Cow,
    fmt::Display,
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use rustc_hash::FxHashMap;
use tracing::info;

use crate::{gen::AtomicStore, msg::BufMsg};

#[derive(Debug)]
pub struct StatsTracker {
    intervals: usize,
    pub recv: u64,
    ok_recv: u64,
    pub atomic_store: Arc<AtomicStore>,
    latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
    rcodes: FxHashMap<u8, u64>,
    buf_size: usize,
    totals: StatsInterval,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(0),
            max_latency: Duration::from_micros(0),
            recv: 0,
            ok_recv: 0,
            atomic_store: Arc::new(AtomicStore::default()),
            totals: StatsInterval::default(),
            rcodes: FxHashMap::default(),
            buf_size: 0,
            intervals: 0,
        }
    }
}
impl StatsTracker {
    pub fn reset(&mut self) {
        self.min_latency = Duration::from_micros(u64::max_value());
        self.max_latency = Duration::from_micros(0);
        self.latency = Duration::from_micros(0);
        self.recv = 0;
        self.ok_recv = 0;
        self.atomic_store.reset();
    }

    fn avg_latency(&self, avg_latency: f32) -> Cow<'static, str> {
        if self.ok_recv != 0 {
            Cow::Owned(avg_latency.to_string())
        } else {
            Cow::Borrowed("-")
        }
    }

    pub fn update(&mut self, sent: Instant, msg: &BufMsg) {
        if let Some(latency) = Instant::now().checked_duration_since(sent) {
            self.latency += latency;
            self.min_latency = self.min_latency.min(latency);
            self.max_latency = self.max_latency.max(latency);
        }
        *self.rcodes.entry(msg.rcode()).or_insert(0) += 1;
        self.buf_size += msg.bytes().len();
        self.ok_recv += 1;
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

    pub fn totals(self) -> StatsInterval {
        let StatsTracker { totals, .. } = self;
        totals
    }

    pub fn log_stats(
        &mut self,
        elapsed: Duration,
        total_duration: Duration,
        in_flight: usize,
        ids: usize,
    ) {
        let timeouts = self.atomic_store.timed_out.load(atomic::Ordering::Relaxed);
        let sent = self.atomic_store.sent.load(atomic::Ordering::Relaxed);
        let recv = self.recv;
        let min = self.min_latency();
        let avg = (self.latency.as_micros() / self.ok_recv as u128) as f32 / 1_000.;
        let avg_latency = self.avg_latency(avg);
        let max = self.max_latency();
        let avg_size = self.buf_size / self.ok_recv as usize;
        let ok_recv = self.ok_recv;
        self.intervals += 1;
        let interval = StatsInterval {
            duration: total_duration,
            elapsed,
            sent,
            recv,
            ok_recv,
            timeouts,
            min_latency: min,
            avg_latency: avg,
            max_latency: max,
            avg_size,
            rcodes: std::mem::take(&mut self.rcodes),
        };
        self.totals.update_totals(interval, self.intervals);
        info!(
            // elapsed = &PrettyDuration(elapsed),
            duration = %PrettyDuration(total_duration),
            send = sent,
            recv,
            to = timeouts,
            min = %format!("{}ms", min),
            avg = %format!("{}ms", avg_latency),
            max = %format!("{}ms", max),
            in_flight,
            ids
        );
        self.reset();
    }
}

#[derive(Debug, Default)]
pub struct StatsInterval {
    duration: Duration,
    elapsed: Duration,
    sent: u64,
    recv: u64,
    ok_recv: u64,
    timeouts: u64,
    min_latency: f32,
    avg_latency: f32,
    max_latency: f32,
    rcodes: FxHashMap<u8, u64>,
    avg_size: usize,
}

impl StatsInterval {
    fn update_totals(&mut self, interval: StatsInterval, intervals: usize) {
        self.duration = interval.duration;
        self.elapsed += interval.elapsed;
        self.sent += interval.sent;
        self.recv += interval.recv;
        self.ok_recv += interval.ok_recv;
        self.timeouts += interval.timeouts;
        self.min_latency = interval.min_latency.min(self.min_latency);
        self.max_latency = interval.max_latency.min(self.max_latency);
        if interval.avg_latency != 0. {
            self.avg_latency = (self.avg_latency * (intervals - 1) as f32 + interval.avg_latency)
                / intervals as f32;
        }
        if interval.avg_size != 0 {
            self.avg_size = (self.avg_size * (intervals - 1) + interval.avg_size) / intervals;
        }
        self.rcodes.extend(interval.rcodes.into_iter());
    }
    pub fn summary(&self) {
        info!(
            runtime = %PrettyDuration(self.duration),
            total_sent = self.sent,
            total_rcvd = self.recv,
            min = %format!("{}ms", self.min_latency),
            avg = %format!("{}ms", self.avg_latency),
            max = %format!("{}ms", self.max_latency),
            avg_size = %format!("{} bytes", self.avg_size),
            timeouts = %format!("{} ({}%)", self.timeouts, self.timeouts / self.recv)
        );
    }
}

struct PrettyDuration(Duration);

impl Display for PrettyDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}s", &self.0.as_secs_f32().to_string()[0..=4])
    }
}
