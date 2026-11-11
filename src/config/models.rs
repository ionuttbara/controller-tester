use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickCalibration {
    pub center_x: f32,
    pub center_y: f32,
    pub min_x: f32,
    pub max_x: f32,
    pub min_y: f32,
    pub max_y: f32,
    pub deadzone: f32,
}

impl Default for StickCalibration {
    fn default() -> Self {
        Self { center_x: 0.0, center_y: 0.0, min_x: -1.0, max_x: 1.0, min_y: -1.0, max_y: 1.0, deadzone: 0.05 }
    }
}

impl StickCalibration {
    pub fn apply_pair(&self, x: f32, y: f32) -> (f32, f32) {
        (self.apply_axis(x, true), self.apply_axis(y, false))
    }

    fn apply_axis(&self, raw: f32, x_axis: bool) -> f32 {
        let (center, min, max) = if x_axis {
            (self.center_x, self.min_x, self.max_x)
        } else {
            (self.center_y, self.min_y, self.max_y)
        };
        let shifted = raw - center;
        if shifted.abs() <= self.deadzone {
            return 0.0;
        }
        let denom = if shifted >= 0.0 { (max - center).max(0.001) } else { (center - min).max(0.001) };
        (shifted / denom).clamp(-1.0, 1.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CalibrationProfile {
    pub left: StickCalibration,
    pub right: StickCalibration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoyKeyProfile {
    pub process: String,
    pub enabled: bool,
    pub mappings: BTreeMap<String, String>,
}

impl JoyKeyProfile {
    pub fn new(process: impl Into<String>) -> Self {
        Self { process: process.into(), enabled: true, mappings: BTreeMap::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WheelCalibration {
    /// Raw normalized steering value which represents the physical center.
    pub center: f32,
    /// Driver/user-selected total lock-to-lock rotation in degrees.
    /// HID reports do not universally expose the currently configured steering lock,
    /// so this value is deliberately persisted per device.
    pub rotation_degrees: f32,
    /// Furthest normalized positions observed by the tester. These are diagnostics,
    /// not firmware calibration values.
    pub observed_min: f32,
    pub observed_max: f32,
}

impl Default for WheelCalibration {
    fn default() -> Self {
        Self {
            center: 0.0,
            rotation_degrees: 900.0,
            observed_min: 0.0,
            observed_max: 0.0,
        }
    }
}

impl WheelCalibration {
    pub fn angle_degrees(&self, raw: f32) -> f32 {
        let centered = (raw - self.center).clamp(-1.0, 1.0);
        centered * self.rotation_degrees.max(1.0) * 0.5
    }

    pub fn update_observed(&mut self, raw: f32) {
        self.observed_min = self.observed_min.min(raw);
        self.observed_max = self.observed_max.max(raw);
    }

    pub fn reset_observed(&mut self, raw: f32) {
        self.observed_min = raw;
        self.observed_max = raw;
    }
}
