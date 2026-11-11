use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default)]
pub struct PollingMetrics {
    pub current_hz: u32,
    pub average_hz: f32,
    pub min_hz: u32,
    pub max_hz: u32,
    pub average_interval_ms: f32,
    pub jitter_ms: f32,
    pub total_events: u64,
}

#[derive(Debug)]
pub struct PollingTracker {
    window_start: Instant,
    events: u32,
    current_hz: u32,
    hz_samples: u64,
    hz_sum: u64,
    min_hz: u32,
    max_hz: u32,
    total_events: u64,
    last_event: Option<Instant>,
    interval_samples: u64,
    interval_mean_ms: f64,
    interval_m2: f64,
    history: VecDeque<u32>,
}

impl Default for PollingTracker {
    fn default() -> Self {
        Self {
            window_start: Instant::now(),
            events: 0,
            current_hz: 0,
            hz_samples: 0,
            hz_sum: 0,
            min_hz: 0,
            max_hz: 0,
            total_events: 0,
            last_event: None,
            interval_samples: 0,
            interval_mean_ms: 0.0,
            interval_m2: 0.0,
            history: VecDeque::with_capacity(60),
        }
    }
}

impl PollingTracker {
    pub fn register_event(&mut self) {
        let now = Instant::now();
        self.events = self.events.saturating_add(1);
        self.total_events = self.total_events.saturating_add(1);

        if let Some(previous) = self.last_event.replace(now) {
            let interval_ms = now.duration_since(previous).as_secs_f64() * 1000.0;
            self.interval_samples = self.interval_samples.saturating_add(1);
            let n = self.interval_samples as f64;
            let delta = interval_ms - self.interval_mean_ms;
            self.interval_mean_ms += delta / n;
            let delta2 = interval_ms - self.interval_mean_ms;
            self.interval_m2 += delta * delta2;
        }
    }

    pub fn update(&mut self) -> PollingMetrics {
        let elapsed = self.window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let secs = elapsed.as_secs_f64().max(0.001);
            self.current_hz = (self.events as f64 / secs).round() as u32;
            self.events = 0;
            self.window_start = Instant::now();

            if self.current_hz > 0 {
                self.hz_samples = self.hz_samples.saturating_add(1);
                self.hz_sum = self.hz_sum.saturating_add(self.current_hz as u64);
                self.min_hz = if self.min_hz == 0 {
                    self.current_hz
                } else {
                    self.min_hz.min(self.current_hz)
                };
                self.max_hz = self.max_hz.max(self.current_hz);
            }
            if self.history.len() == 60 {
                self.history.pop_front();
            }
            self.history.push_back(self.current_hz);
        }
        self.metrics()
    }

    pub fn metrics(&self) -> PollingMetrics {
        let jitter_ms = if self.interval_samples > 1 {
            (self.interval_m2 / (self.interval_samples - 1) as f64).sqrt()
        } else {
            0.0
        };
        PollingMetrics {
            current_hz: self.current_hz,
            average_hz: if self.hz_samples > 0 {
                self.hz_sum as f32 / self.hz_samples as f32
            } else {
                0.0
            },
            min_hz: self.min_hz,
            max_hz: self.max_hz,
            average_interval_ms: self.interval_mean_ms as f32,
            jitter_ms: jitter_ms as f32,
            total_events: self.total_events,
        }
    }


    pub fn history(&self) -> Vec<f32> {
        self.history.iter().map(|value| *value as f32).collect()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
