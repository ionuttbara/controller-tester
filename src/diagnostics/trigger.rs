#[derive(Debug, Clone, Copy, Default)]
pub struct TriggerMetrics {
    pub min: f32,
    pub max: f32,
    pub travel: f32,
    pub samples: u64,
}

#[derive(Debug, Default)]
pub struct TriggerDiagnostics {
    min: f32,
    max: f32,
    samples: u64,
}

impl TriggerDiagnostics {
    pub fn register(&mut self, value: f32) {
        let value = value.clamp(0.0, 1.0);
        if self.samples == 0 {
            self.min = value;
            self.max = value;
        } else {
            self.min = self.min.min(value);
            self.max = self.max.max(value);
        }
        self.samples = self.samples.saturating_add(1);
    }

    pub fn metrics(&self) -> TriggerMetrics {
        if self.samples == 0 {
            return TriggerMetrics::default();
        }
        TriggerMetrics {
            min: self.min,
            max: self.max,
            travel: (self.max - self.min).max(0.0),
            samples: self.samples,
        }
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}
