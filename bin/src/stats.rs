use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct StatsTracker {
    pub recv: u128,
    latency: Duration,
    min_latency: Duration,
    max_latency: Duration,
    total_timeouts: u128,
}

impl Default for StatsTracker {
    fn default() -> Self {
        Self {
            recv: 0,
            total_timeouts: 0,
            latency: Duration::from_micros(0),
            min_latency: Duration::from_micros(0),
            max_latency: Duration::from_micros(0),
        }
    }
}
impl StatsTracker {
    pub fn reset(&mut self) {
        *self = StatsTracker {
            min_latency: Duration::from_micros(u64::max_value()),
            ..Self::default()
        };
    }

    fn avg_latency(&self) -> Cow<'static, str> {
        if self.recv != 0 {
            Cow::Owned(((self.latency.as_micros() / self.recv) as f32 / 1_000.).to_string())
        } else {
            Cow::Borrowed("-")
        }
    }

    pub fn update_latencies(&mut self, sent: Instant) {
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

    pub fn stats_string(
        &self,
        elapsed: Duration,
        total_duration: Duration,
        in_flight: usize,
        ids: usize,
    ) -> String {
        format!(
            "elapsed: {}s recv: {} min/avg/max: {}ms/{}ms/{}ms duration: {}s in_flight: {} ids: {}",
            &elapsed.as_secs_f32().to_string()[0..4],
            self.recv,
            self.min_latency(),
            self.avg_latency(),
            self.max_latency(),
            &total_duration.as_secs_f32().to_string()[0..4],
            in_flight,
            ids
        )
    }
}
