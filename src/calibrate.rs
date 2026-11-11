use crate::config::models::{CalibrationProfile, StickCalibration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CalibrationState {
    #[default]
    Connect,
    Center,
    Range,
    Verify,
    Complete,
}

#[derive(Debug, Default)]
struct CenterAccumulator {
    count: u32,
    lx: f64,
    ly: f64,
    rx: f64,
    ry: f64,
}

#[derive(Debug)]
pub struct CalibrationManager {
    pub state: CalibrationState,
    center: CenterAccumulator,
    profile: CalibrationProfile,
}

impl Default for CalibrationManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CalibrationManager {
    pub fn new() -> Self {
        Self { state: CalibrationState::Connect, center: CenterAccumulator::default(), profile: CalibrationProfile::default() }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn record(&mut self, lx: f32, ly: f32, rx: f32, ry: f32) {
        match self.state {
            CalibrationState::Center => {
                self.center.count = self.center.count.saturating_add(1);
                self.center.lx += lx as f64;
                self.center.ly += ly as f64;
                self.center.rx += rx as f64;
                self.center.ry += ry as f64;
            }
            CalibrationState::Range => {
                record_range(&mut self.profile.left, lx, ly);
                record_range(&mut self.profile.right, rx, ry);
            }
            _ => {}
        }
    }

    /// Advances the wizard. Returns a completed software calibration profile when
    /// Verify -> Complete is crossed. No firmware/NVS writes are performed.
    pub fn advance(&mut self) -> Option<CalibrationProfile> {
        match self.state {
            CalibrationState::Connect => {
                self.state = CalibrationState::Center;
                None
            }
            CalibrationState::Center => {
                self.finalize_center();
                self.profile.left.min_x = 0.0;
                self.profile.left.max_x = 0.0;
                self.profile.left.min_y = 0.0;
                self.profile.left.max_y = 0.0;
                self.profile.right.min_x = 0.0;
                self.profile.right.max_x = 0.0;
                self.profile.right.min_y = 0.0;
                self.profile.right.max_y = 0.0;
                self.state = CalibrationState::Range;
                None
            }
            CalibrationState::Range => {
                normalize_ranges(&mut self.profile.left);
                normalize_ranges(&mut self.profile.right);
                self.state = CalibrationState::Verify;
                None
            }
            CalibrationState::Verify => {
                self.state = CalibrationState::Complete;
                Some(self.profile.clone())
            }
            CalibrationState::Complete => Some(self.profile.clone()),
        }
    }

    pub fn current_profile(&self) -> &CalibrationProfile {
        &self.profile
    }

    fn finalize_center(&mut self) {
        if self.center.count == 0 {
            return;
        }
        let n = self.center.count as f64;
        self.profile.left.center_x = (self.center.lx / n) as f32;
        self.profile.left.center_y = (self.center.ly / n) as f32;
        self.profile.right.center_x = (self.center.rx / n) as f32;
        self.profile.right.center_y = (self.center.ry / n) as f32;

        // Add a small guard above measured center offset. User can later expose this
        // as an editable advanced setting.
        self.profile.left.deadzone = ((self.profile.left.center_x.powi(2) + self.profile.left.center_y.powi(2)).sqrt() + 0.02).clamp(0.02, 0.20);
        self.profile.right.deadzone = ((self.profile.right.center_x.powi(2) + self.profile.right.center_y.powi(2)).sqrt() + 0.02).clamp(0.02, 0.20);
    }
}

fn record_range(stick: &mut StickCalibration, x: f32, y: f32) {
    stick.max_x = stick.max_x.max(x);
    stick.min_x = stick.min_x.min(x);
    stick.max_y = stick.max_y.max(y);
    stick.min_y = stick.min_y.min(y);
}

fn normalize_ranges(stick: &mut StickCalibration) {
    if stick.max_x <= stick.center_x + 0.1 { stick.max_x = 1.0; }
    if stick.min_x >= stick.center_x - 0.1 { stick.min_x = -1.0; }
    if stick.max_y <= stick.center_y + 0.1 { stick.max_y = 1.0; }
    if stick.min_y >= stick.center_y - 0.1 { stick.min_y = -1.0; }
}
