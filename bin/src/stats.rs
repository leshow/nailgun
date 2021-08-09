use std::{
    borrow::Cow,
    fmt::Display,
    io,
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use anyhow::Result;
use rustc_hash::FxHashMap;
use tracing::info;
use trust_dns_proto::op::ResponseCode;

use crate::{
    msg::BufMsg,
    store::{AtomicStore, QueryInfo},
};

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
    recv_buf_size: usize,
    buf_size: usize,
    totals: StatsInterval,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(u64::MAX),
            max_latency: Duration::from_micros(0),
            recv: 0,
            ok_recv: 0,
            atomic_store: Arc::new(AtomicStore::default()),
            totals: StatsInterval::default(),
            rcodes: FxHashMap::default(),
            recv_buf_size: 0,
            buf_size: 0,
            intervals: 0,
        }
    }
}

impl StatsTracker {
    pub fn reset(&mut self) {
        self.min_latency = Duration::from_micros(u64::MAX);
        self.max_latency = Duration::from_micros(0);
        self.latency = Duration::from_micros(0);
        self.recv = 0;
        self.ok_recv = 0;
        self.recv_buf_size = 0;
        self.buf_size = 0;
        self.atomic_store.reset();
    }

    fn avg_latency(&self, avg_latency: f32) -> Cow<'static, str> {
        if self.ok_recv != 0 {
            Cow::Owned(avg_latency.to_string())
        } else {
            Cow::Borrowed("-")
        }
    }

    pub fn update(&mut self, qinfo: QueryInfo, msg: &BufMsg) {
        if let Some(latency) = Instant::now().checked_duration_since(qinfo.sent) {
            self.latency += latency;
            self.min_latency = self.min_latency.min(latency);
            self.max_latency = self.max_latency.max(latency);
        }
        *self.rcodes.entry(msg.rcode()).or_insert(0) += 1;
        self.buf_size += qinfo.len;
        self.recv_buf_size += msg.bytes().len();
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
        // TODO: handle div by 0 better
        let avg = if self.ok_recv != 0 {
            (self.latency.as_micros() / self.ok_recv as u128) as f32 / 1_000.
        } else {
            // ?
            self.latency.as_micros() as f32 / 1_000.
        };
        let recv_avg_size = if self.ok_recv != 0 {
            self.recv_buf_size / self.ok_recv as usize
        } else {
            self.recv_buf_size
        };
        // TODO: I think there is a potential bug here
        // where we count sent via the atomic_store but
        // buf_size is counted only on recv
        let avg_size = if sent != 0 {
            self.buf_size / sent as usize
        } else {
            self.buf_size
        };
        let max = self.max_latency();
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
            recv_avg_size,
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
            avg = %format!("{}ms", self.avg_latency(avg)),
            max = %format!("{}ms", max),
            avg_size,
            recv_avg_size,
            in_flight,
            ids
        );
        self.reset();
    }
}

#[derive(Debug)]
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
    recv_avg_size: usize,
    avg_size: usize,
}

impl Default for StatsInterval {
    fn default() -> Self {
        Self {
            duration: Duration::from_micros(0),
            min_latency: f32::MAX,
            avg_latency: 0.,
            max_latency: 0.,
            sent: 0,
            recv: 0,
            ok_recv: 0,
            rcodes: FxHashMap::default(),
            avg_size: 0,
            recv_avg_size: 0,
            elapsed: Duration::from_micros(0),
            timeouts: 0,
        }
    }
}

impl StatsInterval {
    fn min_latency(&self) -> f32 {
        if self.min_latency == f32::MAX {
            0.
        } else {
            self.min_latency
        }
    }
    pub fn update_totals(&mut self, interval: StatsInterval, intervals: usize) {
        self.duration = interval.duration;
        self.elapsed += interval.elapsed;
        self.sent += interval.sent;
        self.recv += interval.recv;
        self.ok_recv += interval.ok_recv;
        self.timeouts += interval.timeouts;
        self.min_latency = if interval.min_latency != 0. {
            interval.min_latency.min(self.min_latency)
        } else {
            self.min_latency()
        };
        self.max_latency = interval.max_latency.max(self.max_latency);
        if interval.avg_latency != 0. {
            self.avg_latency = (self.avg_latency * (intervals - 1) as f32 + interval.avg_latency)
                / intervals as f32;
        }
        if interval.avg_size != 0 {
            self.avg_size = (self.avg_size * (intervals - 1) + interval.avg_size) / intervals;
        }
        if interval.recv_avg_size != 0 {
            self.recv_avg_size =
                (self.recv_avg_size * (intervals - 1) + interval.recv_avg_size) / intervals;
        }
        for (rcode, count) in interval.rcodes.into_iter() {
            *self.rcodes.entry(rcode).or_insert(0) += count;
        }
    }

    pub fn summary(&self) -> Result<()> {
        use std::io::Write;
        let runtime = PrettyDuration(self.duration);
        let to_percent = if self.sent == 0 {
            0.
        } else {
            (self.timeouts as f32 / self.sent as f32) * 100.
        };
        info!(
            runtime = %runtime,
            total_sent = self.sent,
            total_rcvd = self.recv,
            min = %format!("{}ms", self.min_latency),
            avg = %format!("{}ms", self.avg_latency),
            max = %format!("{}ms", self.max_latency),
            sent_avg_size = %format!("{} bytes", self.avg_size),
            rcvd_avg_size = %format!("{} bytes", self.recv_avg_size),
            timeouts = %format!("{} ({}%)", self.timeouts, to_percent)
        );
        // TODO: print this better?
        for (rcode, count) in self.rcodes.iter() {
            info!(responses = %format!("{:?}: {}", ResponseCode::from(0, *rcode), *count));
        }
        // write it to stdout nicely
        let stdout = io::stdout();
        let mut f = stdout.lock();
        writeln!(f, "-----")?;
        writeln!(f, "runtime        {}", runtime)?;
        writeln!(f, "total sent     {}", self.sent)?;
        writeln!(f, "total rcvd     {}", self.recv)?;
        writeln!(f, "min latency    {}ms", self.min_latency)?;
        writeln!(f, "avg latency    {}ms", self.avg_latency)?;
        writeln!(f, "max latency    {}ms", self.max_latency)?;
        writeln!(f, "sent avg size  {} bytes", self.avg_size)?;
        writeln!(f, "rcvd avg size  {} bytes", self.recv_avg_size)?;
        writeln!(f, "timeouts       {} ({}%)", self.timeouts, to_percent)?;
        writeln!(f, "responses")?;
        for (rcode, count) in self.rcodes.iter() {
            writeln!(f, "   {:?} {}", ResponseCode::from(0, *rcode), *count)?;
        }
        Ok(())
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
struct PrettyDuration(Duration);

impl Display for PrettyDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}s", &self.0.as_secs_f32().to_string()[0..=4])
    }
}
