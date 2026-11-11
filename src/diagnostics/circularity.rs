#[derive(Debug, Clone, Copy, Default)]
pub struct CircularityMetrics {
    pub avg_error: f32,
    pub max_error: f32,
    pub samples: u32,
    pub quadrant_error: [f32; 4],
}

#[derive(Debug, Default)]
pub struct CircularityTracker {
    samples: u32,
    error_sum: f32,
    max_error: f32,
    quadrant_sum: [f32; 4],
    quadrant_count: [u32; 4],
}

impl CircularityTracker {
    pub fn register(&mut self, x: f32, y: f32) {
        let radius = (x * x + y * y).sqrt();
        if radius < 0.80 {
            return;
        }
        let error = (1.0 - radius).abs();
        self.samples = self.samples.saturating_add(1);
        self.error_sum += error;
        self.max_error = self.max_error.max(error);

        let quadrant = match (x >= 0.0, y >= 0.0) {
            (true, true) => 0,
            (false, true) => 1,
            (false, false) => 2,
            (true, false) => 3,
        };
        self.quadrant_sum[quadrant] += error;
        self.quadrant_count[quadrant] = self.quadrant_count[quadrant].saturating_add(1);
    }

    pub fn metrics(&self) -> CircularityMetrics {
        let mut q = [0.0f32; 4];
        for i in 0..4 {
            if self.quadrant_count[i] > 0 {
                q[i] = self.quadrant_sum[i] / self.quadrant_count[i] as f32 * 100.0;
            }
        }
        CircularityMetrics {
            avg_error: if self.samples == 0 { 0.0 } else { self.error_sum / self.samples as f32 * 100.0 },
            max_error: self.max_error * 100.0,
            samples: self.samples,
            quadrant_error: q,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
