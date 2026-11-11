use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default)]
pub struct StickMetrics {
    pub center_x: f32,
    pub center_y: f32,
    pub center_offset: f32,
    pub noise: f32,
    pub peak_drift: f32,
    pub recommended_deadzone: f32,
    pub samples: u32,
}

#[derive(Debug)]
pub struct StickDiagnostics {
    samples: VecDeque<(f32, f32)>,
    capacity: usize,
}

impl Default for StickDiagnostics {
    fn default() -> Self {
        Self::new(1200)
    }
}

impl StickDiagnostics {
    pub fn new(capacity: usize) -> Self {
        Self { samples: VecDeque::with_capacity(capacity), capacity }
    }

    pub fn register(&mut self, x: f32, y: f32) {
        let radius = (x * x + y * y).sqrt();
        // Only analyze center behavior. Intentional stick movement should not be classified as drift.
        if radius <= 0.25 {
            if self.samples.len() == self.capacity {
                self.samples.pop_front();
            }
            self.samples.push_back((x, y));
        }
    }

    pub fn metrics(&self) -> StickMetrics {
        if self.samples.is_empty() {
            return StickMetrics::default();
        }

        let n = self.samples.len() as f32;
        let (sum_x, sum_y) = self.samples.iter().fold((0.0f32, 0.0f32), |acc, &(x, y)| (acc.0 + x, acc.1 + y));
        let mean_x = sum_x / n;
        let mean_y = sum_y / n;
        let center_offset = (mean_x * mean_x + mean_y * mean_y).sqrt();

        let mut variance = 0.0f32;
        let mut peak = 0.0f32;
        for &(x, y) in &self.samples {
            let dx = x - mean_x;
            let dy = y - mean_y;
            variance += dx * dx + dy * dy;
            peak = peak.max((x * x + y * y).sqrt());
        }
        let noise = (variance / n).sqrt();
        let recommended = (center_offset + noise * 3.0).clamp(0.0, 0.25);

        StickMetrics {
            center_x: mean_x,
            center_y: mean_y,
            center_offset,
            noise,
            peak_drift: peak,
            recommended_deadzone: recommended,
            samples: self.samples.len() as u32,
        }
    }

    pub fn reset(&mut self) {
        self.samples.clear();
    }
}
