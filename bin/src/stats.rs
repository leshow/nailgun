use std::{
    borrow::Cow,
    fmt::Display,
    io,
    sync::{atomic, Arc},
    time::{Duration, Instant},
};

use anyhow::Result;
use rustc_hash::FxHashMap;
use tokio::sync::mpsc;
use tracing::info;
use trust_dns_proto::op::ResponseCode;

use crate::{
    msg::BufMsg,
    store::{AtomicStore, QueryInfo},
    util::MovingAvg,
};

#[derive(Debug)]
pub struct StatsTracker {
    intervals: usize,
    pub recv: u64,
    ok_recv: u64,
    pub atomic_store: Arc<AtomicStore>,
    total_latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
    rcodes: FxHashMap<u8, u64>,
    recv_buf_size: usize,
    buf_size: usize,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            total_latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(u64::MAX),
            max_latency: Duration::from_micros(0),
            recv: 0,
            ok_recv: 0,
            atomic_store: Arc::new(AtomicStore::default()),
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
        self.total_latency = Duration::from_micros(0);
        self.recv = 0;
        self.ok_recv = 0;
        self.recv_buf_size = 0;
        self.buf_size = 0;
        self.atomic_store.reset();
    }

    pub fn update(&mut self, qinfo: QueryInfo, msg: &BufMsg) {
        if let Some(latency) = Instant::now().checked_duration_since(qinfo.sent) {
            self.total_latency += latency;
            self.min_latency = self.min_latency.min(latency);
            self.max_latency = self.max_latency.max(latency);
        }
        *self.rcodes.entry(msg.rcode()).or_insert(0) += 1;
        self.buf_size += qinfo.len;
        self.recv_buf_size += msg.bytes().len();
        self.ok_recv += 1;
    }

    pub fn interval(
        &mut self,
        elapsed: Duration,
        total_duration: Duration,
        in_flight: usize,
        ids_len: usize,
    ) -> StatsInterval {
        let timeouts = self.atomic_store.timed_out.load(atomic::Ordering::Relaxed);
        let sent = self.atomic_store.sent.load(atomic::Ordering::Relaxed);
        let interval = StatsInterval {
            interval: self.intervals,
            duration: total_duration,
            elapsed,
            sent,
            recv: self.recv,
            ok_recv: self.ok_recv,
            timeouts,
            min_latency: self.min_latency,
            total_latency: self.total_latency,
            max_latency: self.max_latency,
            recv_buf_size: self.recv_buf_size,
            buf_size: self.buf_size,
            rcodes: std::mem::take(&mut self.rcodes),
            in_flight,
        };
        self.intervals += 1;
        self.reset();

        interval
    }
}

#[derive(Debug)]
pub struct StatsInterval {
    duration: Duration,
    elapsed: Duration,
    interval: usize,
    sent: u64,
    recv: u64,
    ok_recv: u64,
    timeouts: u64,
    min_latency: Duration,
    total_latency: Duration,
    max_latency: Duration,
    rcodes: FxHashMap<u8, u64>,
    recv_buf_size: usize,
    buf_size: usize,
    in_flight: usize,
}

impl Default for StatsInterval {
    fn default() -> Self {
        Self {
            duration: Duration::from_micros(0),
            total_latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(u64::MAX),
            max_latency: Duration::from_micros(0),
            interval: 0,
            sent: 0,
            recv: 0,
            ok_recv: 0,
            rcodes: FxHashMap::default(),
            buf_size: 0,
            recv_buf_size: 0,
            elapsed: Duration::from_micros(0),
            timeouts: 0,
            in_flight: 0,
        }
    }
}

impl StatsInterval {
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
    fn avg_latency(&self) -> f32 {
        if self.ok_recv != 0 {
            (self.total_latency.as_micros() / self.ok_recv as u128) as f32 / 1_000.
        } else {
            self.total_latency.as_micros() as f32 / 1_000.
        }
    }
    fn avg_latency_str(&self) -> Cow<'_, str> {
        if self.ok_recv != 0 {
            Cow::Owned(self.avg_latency().to_string())
        } else {
            Cow::Borrowed("-")
        }
    }
    fn recv_avg_size(&self) -> usize {
        if self.ok_recv != 0 {
            self.recv_buf_size / self.ok_recv as usize
        } else {
            self.recv_buf_size
        }
    }
    fn avg_size(&self) -> usize {
        if self.sent != 0 {
            self.buf_size / self.sent as usize
        } else {
            self.buf_size
        }
    }

    pub fn aggregate(&mut self, agg: StatsInterval) {
        // destructuring like this makes sure we didn't miss any
        let Self {
            duration,
            elapsed,
            interval,
            sent,
            recv,
            ok_recv,
            timeouts,
            min_latency,
            total_latency,
            max_latency,
            rcodes,
            recv_buf_size,
            buf_size,
            in_flight,
        } = self;
        *interval = agg.interval;
        *duration = agg.duration;
        *elapsed += agg.elapsed;
        *sent += agg.sent;
        *recv += agg.recv;
        *ok_recv += agg.ok_recv;
        *timeouts += agg.timeouts;
        *min_latency = agg.min_latency.min(*min_latency);
        *max_latency = agg.max_latency.max(*max_latency);
        *total_latency += agg.total_latency;
        for (rcode, count) in agg.rcodes.into_iter() {
            *rcodes.entry(rcode).or_insert(0) += count;
        }
        *recv_buf_size += agg.recv_buf_size;
        *buf_size += agg.buf_size;
        *in_flight += agg.in_flight;
    }
    pub fn reset(&mut self) {
        self.min_latency = Duration::from_micros(u64::MAX);
        self.max_latency = Duration::from_micros(0);
        self.total_latency = Duration::from_micros(0);
        self.recv = 0;
        self.ok_recv = 0;
        self.recv_buf_size = 0;
        self.buf_size = 0;
        self.sent = 0;
        self.in_flight = 0;
        self.timeouts = 0;
    }

    pub fn log(&mut self) -> StatsInterval {
        info!(
            // elapsed = &PrettyDuration(elapsed),
            duration = %PrettyDuration(self.duration),
            send = self.sent,
            recv = self.recv,
            to = self.timeouts,
            min = %format!("{}ms", self.min_latency()),
            avg = %format!("{}ms", self.avg_latency_str()),
            max = %format!("{}ms", self.max_latency()),
            avg_size = self.avg_size(),
            recv_avg_size = self.recv_avg_size(),
            in_flight = self.in_flight,
        );
        let interval = StatsInterval {
            interval: self.interval,
            duration: self.duration,
            elapsed: self.elapsed,
            sent: self.sent,
            recv: self.recv,
            ok_recv: self.ok_recv,
            timeouts: self.timeouts,
            min_latency: self.min_latency,
            total_latency: self.total_latency,
            max_latency: self.max_latency,
            recv_buf_size: self.recv_buf_size,
            buf_size: self.buf_size,
            rcodes: std::mem::take(&mut self.rcodes),
            in_flight: self.in_flight,
        };
        self.reset();
        interval
    }
}

#[derive(Debug)]
pub struct StatsTotals {
    duration: Duration,
    elapsed: Duration,
    interval: usize,
    sent: u64,
    recv: u64,
    ok_recv: u64,
    timeouts: u64,
    min_latency: f32,
    avg_latency: MovingAvg<f32>,
    max_latency: f32,
    rcodes: FxHashMap<u8, u64>,
    avg_recv_buf_size: MovingAvg<usize>,
    avg_buf_size: MovingAvg<usize>,
    in_flight: usize,
}

impl Default for StatsTotals {
    fn default() -> Self {
        Self {
            duration: Duration::from_micros(0),
            elapsed: Duration::from_micros(0),
            avg_latency: MovingAvg::new(0.),
            min_latency: f32::MAX,
            max_latency: 0.,
            interval: 0,
            sent: 0,
            recv: 0,
            ok_recv: 0,
            rcodes: FxHashMap::default(),
            avg_buf_size: MovingAvg::new(0),
            avg_recv_buf_size: MovingAvg::new(0),
            timeouts: 0,
            in_flight: 0,
        }
    }
}

impl StatsTotals {
    pub fn update(&mut self, interval: StatsInterval) {
        self.duration = interval.duration;
        self.elapsed += interval.elapsed;
        self.sent += interval.sent;
        self.recv += interval.recv;
        self.ok_recv += interval.ok_recv;
        self.timeouts += interval.timeouts;
        self.in_flight += interval.in_flight;
        self.min_latency = if interval.min_latency() != 0. {
            interval.min_latency().min(self.min_latency)
        } else {
            self.min_latency
        };
        self.max_latency = interval.max_latency().max(self.max_latency);
        if interval.avg_latency() != 0. {
            self.avg_latency.next(interval.avg_latency());
        }
        if interval.buf_size != 0 && interval.sent != 0 {
            let avg = interval.buf_size / interval.sent as usize;
            self.avg_buf_size.next(avg);
        }
        if interval.recv_buf_size != 0 && interval.ok_recv != 0 {
            let avg = interval.recv_buf_size / interval.ok_recv as usize;
            self.avg_recv_buf_size.next(avg);
        }
        for (rcode, count) in interval.rcodes.into_iter() {
            *self.rcodes.entry(rcode).or_insert(0) += count;
        }
    }
    pub fn summary(&self) -> Result<()> {
        use std::io::Write;
        let runtime = PrettyDuration(self.duration);
        let to_percent = if self.sent == 0 || self.timeouts == 0 {
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
            sent_avg_size = %format!("{} bytes", self.avg_buf_size),
            rcvd_avg_size = %format!("{} bytes", self.avg_recv_buf_size),
            timeouts = %format!("{} ({}%)", self.timeouts, to_percent)
        );
        // TODO: print this better?
        for (rcode, count) in self.rcodes.iter() {
            info!(responses = %format!("{:?}: {}", ResponseCode::from(0, *rcode), *count));
        }
        // write it to stdout nicely
        let stdout = io::stdout();
        let mut f = stdout.lock();
        writeln!(f, "-----RESULTS-----")?;
        writeln!(f, "runtime        {}", runtime)?;
        writeln!(f, "total sent     {}", self.sent)?;
        writeln!(f, "total rcvd     {}", self.recv)?;
        writeln!(f, "min latency    {}ms", self.min_latency)?;
        writeln!(f, "avg latency    {}ms", self.avg_latency)?;
        writeln!(f, "max latency    {}ms", self.max_latency)?;
        writeln!(f, "sent avg size  {} bytes", self.avg_buf_size)?;
        writeln!(f, "rcvd avg size  {} bytes", self.avg_recv_buf_size)?;
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

#[derive(Debug)]
pub struct StatsRunner {
    rx: mpsc::Receiver<StatsInterval>,
    len: u32,
    summary: StatsInterval,
    totals: StatsTotals,
}

impl StatsRunner {
    pub fn new(rx: mpsc::Receiver<StatsInterval>, len: usize) -> Self {
        Self {
            rx,
            len: len as u32,
            summary: StatsInterval::default(),
            totals: StatsTotals::default(),
        }
    }
    pub async fn run(&mut self) -> Result<()> {
        let mut generators = 0;
        while let Some(interval) = self.rx.recv().await {
            generators += 1;
            self.summary.aggregate(interval);
            // we've gathered all the generators' stats for this interval, so log
            if generators == self.len {
                // add this interval to the totals
                self.totals.update(self.summary.log());
                generators = 0;
            }
            // recv returns None when all senders are dropped,
            // meaning our run is done
        }
        self.totals.summary()?;
        Ok(())
    }
}
