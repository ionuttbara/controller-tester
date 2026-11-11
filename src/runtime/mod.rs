use anyhow::Result;
use crossbeam_channel::{unbounded, Receiver, Sender};
use gilrs::ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Replay, Ticks};
use gilrs::{EventType, Gilrs, GilrsBuilder};
use parking_lot::RwLock;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::calibrate::{CalibrationManager, CalibrationState};
use crate::config::models::{CalibrationProfile, WheelCalibration};
use crate::config::registry;
use crate::device_manager::{
    self,
    model::DeviceDescriptor,
    semantic_hid::{SemanticAxisValue, SemanticHidReader},
    model::DeviceClass,
    vendor::{
        AdaptiveTriggerMode, AdaptiveTriggerParams, AdaptiveTriggerSide,
        NintendoRumbleParams, NintendoRumbleSide, VendorTelemetry,
    },
};
use crate::diagnostics::{
    CircularityMetrics, CircularityTracker, PollingMetrics, PollingTracker, StickDiagnostics, StickMetrics,
    TriggerDiagnostics, TriggerMetrics,
};
use crate::input::{raw_input::{RawInputMonitor, RawInputPacket}, InputSnapshot};

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub devices: Vec<DeviceDescriptor>,
    pub devices_revision: u64,
    pub active_id: i32,
    pub input: InputSnapshot,
    pub semantic_axes: Vec<SemanticAxisValue>,
    pub polling_hz: u32,
    pub polling_metrics: PollingMetrics,
    pub poll_history: Vec<f32>,
    pub raw_capture_paused: bool,
    pub left_stick: StickMetrics,
    pub right_stick: StickMetrics,
    pub left_trigger: TriggerMetrics,
    pub right_trigger: TriggerMetrics,
    pub left_circularity: CircularityMetrics,
    pub right_circularity: CircularityMetrics,
    pub calibration_state: CalibrationState,
    pub calibration_saved: bool,
    pub calibration_summary: String,
    pub joykey_enabled: bool,
    pub backend_status: String,
    pub raw_hid_report: String,
    pub raw_input_status: String,
    pub raw_input_report: String,
    pub vendor_protocol: String,
    pub battery_text: String,
    pub firmware_detail: String,
    pub gyro: [i32; 3],
    pub accel: [i32; 3],
    pub gyro_dps: [f32; 3],
    pub accel_g: [f32; 3],
    pub motion_units: String,
    pub power_source: String,
    pub motion_available: bool,
    pub touch1_text: String,
    pub touch2_text: String,
    pub touch_active: bool,
    pub wheel_available: bool,
    pub wheel_raw: f32,
    pub wheel_angle_deg: f32,
    pub wheel_rotation_deg: f32,
    pub wheel_observed_min: f32,
    pub wheel_observed_max: f32,
    pub wheel_source: String,
    pub adaptive_trigger_available: bool,
    pub nintendo_rumble_available: bool,
    pub vendor_control_status: String,
}

impl Default for RuntimeSnapshot {
    fn default() -> Self {
        Self {
            devices: Vec::new(),
            devices_revision: 0,
            active_id: -1,
            input: InputSnapshot::default(),
            semantic_axes: Vec::new(),
            polling_hz: 0,
            polling_metrics: PollingMetrics::default(),
            poll_history: Vec::new(),
            raw_capture_paused: false,
            left_stick: StickMetrics::default(),
            right_stick: StickMetrics::default(),
            left_trigger: TriggerMetrics::default(),
            right_trigger: TriggerMetrics::default(),
            left_circularity: CircularityMetrics::default(),
            right_circularity: CircularityMetrics::default(),
            calibration_state: CalibrationState::Connect,
            calibration_saved: false,
            calibration_summary: "No calibration samples yet".to_string(),
            joykey_enabled: false,
            backend_status: "Initializing".to_string(),
            raw_hid_report: String::new(),
            raw_input_status: "Starting WM_INPUT transport".to_string(),
            raw_input_report: String::new(),
            vendor_protocol: String::new(),
            battery_text: "Not exposed".to_string(),
            firmware_detail: String::new(),
            gyro: [0; 3],
            accel: [0; 3],
            gyro_dps: [0.0; 3],
            accel_g: [0.0; 3],
            motion_units: "Raw units only".to_string(),
            power_source: "Not exposed".to_string(),
            motion_available: false,
            touch1_text: "Inactive".to_string(),
            touch2_text: "Inactive".to_string(),
            touch_active: false,
            wheel_available: false,
            wheel_raw: 0.0,
            wheel_angle_deg: 0.0,
            wheel_rotation_deg: 900.0,
            wheel_observed_min: 0.0,
            wheel_observed_max: 0.0,
            wheel_source: "No steering axis".to_string(),
            adaptive_trigger_available: false,
            nintendo_rumble_available: false,
            vendor_control_status: "No vendor output protocol active".to_string(),
        }
    }
}

#[derive(Debug)]
pub enum RuntimeCommand {
    SelectDevice(i32),
    RefreshDevices,
    Vibrate { strong: f32, weak: f32, duration_ms: u32 },
    StopVibration,
    SetAdaptiveTrigger {
        side: AdaptiveTriggerSide,
        mode: AdaptiveTriggerMode,
        start: f32,
        end: f32,
        strength: f32,
        frequency: u8,
    },
    StopAdaptiveTriggers,
    SetNintendoRumble {
        side: NintendoRumbleSide,
        low_frequency_hz: f32,
        high_frequency_hz: f32,
        amplitude: f32,
    },
    StopNintendoRumble,
    ClearCircularity,
    ResetDiagnostics,
    SetRawCapturePaused(bool),
    CalibrationNext,
    CalibrationReset,
    SetJoyKeyEnabled(bool),
    SetWheelRotation(f32),
    CenterWheel,
    ResetWheelRange,
    Shutdown,
}

pub struct AppRuntime {
    state: Arc<RwLock<RuntimeSnapshot>>,
    tx: Sender<RuntimeCommand>,
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AppRuntime {
    pub fn start() -> Result<Self> {
        let gilrs = GilrsBuilder::new()
            .with_default_filters(false)
            .build()
            .map_err(|e| anyhow::anyhow!("Gilrs initialization failed: {e}"))?;
        let state = Arc::new(RwLock::new(RuntimeSnapshot::default()));
        let (tx, rx) = unbounded();
        let running = Arc::new(AtomicBool::new(true));
        let state_worker = state.clone();
        let running_worker = running.clone();

        let worker = thread::Builder::new()
            .name("ibbe-controller-runtime".to_string())
            .spawn(move || runtime_thread(gilrs, rx, state_worker, running_worker))?;

        Ok(Self { state, tx, running, worker: Some(worker) })
    }

    pub fn snapshot(&self) -> RuntimeSnapshot {
        self.state.read().clone()
    }

    pub fn command(&self, cmd: RuntimeCommand) {
        let _ = self.tx.send(cmd);
    }
}

impl Drop for AppRuntime {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Release);
        let _ = self.tx.send(RuntimeCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn runtime_thread(
    mut gilrs: Gilrs,
    rx: Receiver<RuntimeCommand>,
    state: Arc<RwLock<RuntimeSnapshot>>,
    running: Arc<AtomicBool>,
) {
    let settings = registry::load_settings();
    let mut polling = PollingTracker::default();
    let mut left_diag = StickDiagnostics::default();
    let mut right_diag = StickDiagnostics::default();
    let mut left_trigger_diag = TriggerDiagnostics::default();
    let mut right_trigger_diag = TriggerDiagnostics::default();
    let mut left_circ = CircularityTracker::default();
    let mut right_circ = CircularityTracker::default();
    let mut calibration = CalibrationManager::new();
    let mut active_calibration: Option<CalibrationProfile> = None;
    let mut active_effect: Option<Effect> = None;
    let mut semantic_reader: Option<SemanticHidReader> = None;
    let mut semantic_input_cache = InputSnapshot::default();
    let mut semantic_axes_cache = Vec::<SemanticAxisValue>::new();
    let mut raw_hid_report = String::new();
    let mut vendor_telemetry = VendorTelemetry::default();
    let mut vendor_control_status = "No vendor output protocol active".to_string();
    let mut wheel_profile = WheelCalibration::default();
    let mut backend_status = "Controller backend running".to_string();
    let raw_input_monitor = RawInputMonitor::start();
    let mut raw_input_status = match &raw_input_monitor {
        Ok(_) => "WM_INPUT registered • background HID capture".to_string(),
        Err(err) => format!("WM_INPUT unavailable: {err}"),
    };
    let mut raw_input_report = String::new();
    let mut force_rescan = true;
    let mut devices = Vec::<DeviceDescriptor>::new();
    let mut devices_revision = 0u64;
    let mut active_id = -1i32;
    let mut preferred_stable_key: Option<String> = None;
    let mut raw_capture_paused = false;
    let mut calibration_saved = false;
    let mut last_metadata_scan = Instant::now() - Duration::from_secs(10);

    {
        let mut s = state.write();
        s.joykey_enabled = settings.joykey_enabled;
        s.backend_status = backend_status.clone();
    }

    while running.load(Ordering::Acquire) {
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                RuntimeCommand::SelectDevice(id) => {
                    preferred_stable_key = devices
                        .iter()
                        .find(|d| d.id == id)
                        .map(|d| d.stable_key.clone());
                    stop_active_vibration(&mut active_effect);
                    stop_vendor_outputs(&mut semantic_reader);
                    active_id = id;
                    reset_analysis(
                        &mut polling,
                        &mut left_diag,
                        &mut right_diag,
                        &mut left_trigger_diag,
                        &mut right_trigger_diag,
                        &mut left_circ,
                        &mut right_circ,
                        &mut calibration,
                    );
                    calibration_saved = false;
                    active_calibration = devices
                        .iter()
                        .find(|d| d.id == id)
                        .and_then(|d| registry::load_calibration(&d.stable_key));
                    wheel_profile = devices
                        .iter()
                        .find(|d| d.id == id)
                        .and_then(|d| registry::load_wheel_calibration(&d.stable_key))
                        .unwrap_or_default();
                    vendor_telemetry = VendorTelemetry::default();
                    semantic_input_cache = InputSnapshot::default();
                    semantic_axes_cache.clear();
                    let opened = open_semantic_hid_reader(&mut devices, id);
                    semantic_reader = opened.reader;
                    vendor_control_status = semantic_reader
                        .as_ref()
                        .map(|reader| reader.control_status().to_string())
                        .unwrap_or_else(|| "No vendor output protocol active".to_string());
                    if !raw_capture_paused {
                        raw_hid_report = opened.message.clone();
                    }
                    backend_status = opened.message;
                    if opened.device_changed {
                        devices_revision = devices_revision.wrapping_add(1);
                    }
                }
                RuntimeCommand::RefreshDevices => force_rescan = true,
                RuntimeCommand::Vibrate { strong, weak, duration_ms } => {
                    stop_active_vibration(&mut active_effect);
                    active_effect = play_vibration(&mut gilrs, active_id, strong, weak, duration_ms);
                }
                RuntimeCommand::StopVibration => {
                    stop_active_vibration(&mut active_effect);
                }
                RuntimeCommand::SetAdaptiveTrigger { side, mode, start, end, strength, frequency } => {
                    let params = AdaptiveTriggerParams { start, end, strength, frequency };
                    vendor_control_status = match semantic_reader.as_mut() {
                        Some(reader) if reader.adaptive_trigger_available() => match reader.set_adaptive_triggers(side, mode, params) {
                            Ok(()) => reader.control_status().to_string(),
                            Err(err) => format!("Adaptive trigger output failed: {err}"),
                        },
                        _ => "Adaptive triggers require a selected DualSense / DualSense Edge HID endpoint".to_string(),
                    };
                }
                RuntimeCommand::StopAdaptiveTriggers => {
                    vendor_control_status = match semantic_reader.as_mut() {
                        Some(reader) if reader.adaptive_trigger_available() => match reader.clear_adaptive_triggers() {
                            Ok(()) => reader.control_status().to_string(),
                            Err(err) => format!("Adaptive trigger stop failed: {err}"),
                        },
                        _ => "Adaptive triggers are not active for the selected device".to_string(),
                    };
                }
                RuntimeCommand::SetNintendoRumble { side, low_frequency_hz, high_frequency_hz, amplitude } => {
                    let params = NintendoRumbleParams {
                        low_frequency_hz: low_frequency_hz.clamp(40.87, 626.0),
                        high_frequency_hz: high_frequency_hz.clamp(81.75, 1252.0),
                        amplitude: amplitude.clamp(0.0, 1.0),
                    };
                    vendor_control_status = match semantic_reader.as_mut() {
                        Some(reader) if reader.nintendo_rumble_available() => match reader.set_nintendo_hd_rumble(side, params) {
                            Ok(()) => reader.control_status().to_string(),
                            Err(err) => format!("Nintendo HD rumble failed: {err}"),
                        },
                        _ => "Nintendo HD rumble requires a selected Switch Pro / Joy-Con HID endpoint".to_string(),
                    };
                }
                RuntimeCommand::StopNintendoRumble => {
                    vendor_control_status = match semantic_reader.as_mut() {
                        Some(reader) if reader.nintendo_rumble_available() => match reader.stop_nintendo_hd_rumble() {
                            Ok(()) => reader.control_status().to_string(),
                            Err(err) => format!("Nintendo HD rumble stop failed: {err}"),
                        },
                        _ => "Nintendo HD rumble is not active for the selected device".to_string(),
                    };
                }
                RuntimeCommand::ClearCircularity => {
                    left_circ.reset();
                    right_circ.reset();
                }
                RuntimeCommand::ResetDiagnostics => {
                    left_diag.reset();
                    right_diag.reset();
                    left_trigger_diag.reset();
                    right_trigger_diag.reset();
                    left_circ.reset();
                    right_circ.reset();
                    polling.reset();
                }
                RuntimeCommand::SetRawCapturePaused(paused) => {
                    raw_capture_paused = paused;
                    state.write().raw_capture_paused = paused;
                }
                RuntimeCommand::CalibrationNext => {
                    if let Some(profile) = calibration.advance() {
                        if let Some(device) = devices.iter().find(|d| d.id == active_id) {
                            calibration_saved =
                                registry::save_calibration(&device.stable_key, &profile).is_ok();
                            active_calibration = Some(profile);
                        }
                    }
                }
                RuntimeCommand::CalibrationReset => {
                    calibration.reset();
                    calibration_saved = false;
                }
                RuntimeCommand::SetJoyKeyEnabled(enabled) => {
                    let saved = registry::save_joykey_enabled(enabled).is_ok();
                    if enabled && saved {
                        let _ = registry::launch_joykey_agent();
                    }
                    state.write().joykey_enabled = enabled && saved;
                }
                RuntimeCommand::SetWheelRotation(degrees) => {
                    wheel_profile.rotation_degrees = degrees.clamp(90.0, 2520.0);
                    if let Some(device) = devices.iter().find(|d| d.id == active_id) {
                        let _ = registry::save_wheel_calibration(&device.stable_key, &wheel_profile);
                    }
                }
                RuntimeCommand::CenterWheel => {
                    let raw = state.read().wheel_raw;
                    wheel_profile.center = raw;
                    if let Some(device) = devices.iter().find(|d| d.id == active_id) {
                        let _ = registry::save_wheel_calibration(&device.stable_key, &wheel_profile);
                    }
                }
                RuntimeCommand::ResetWheelRange => {
                    let raw = state.read().wheel_raw;
                    wheel_profile.reset_observed(raw);
                    if let Some(device) = devices.iter().find(|d| d.id == active_id) {
                        let _ = registry::save_wheel_calibration(&device.stable_key, &wheel_profile);
                    }
                }
                RuntimeCommand::Shutdown => {
                    stop_active_vibration(&mut active_effect);
                    stop_vendor_outputs(&mut semantic_reader);
                    running.store(false, Ordering::Release);
                    break;
                }
            }
        }

        let mut connected_changed = false;
        let mut active_event = false;
        let mut active_axis_event = false;
        while let Some(event) = gilrs.next_event() {
            if usize::from(event.id) as i32 == active_id {
                polling.register_event();
                active_event = true;
                if matches!(
                    &event.event,
                    EventType::AxisChanged(_, _, _) | EventType::ButtonChanged(_, _, _)
                ) {
                    active_axis_event = true;
                }
            }
            if matches!(event.event, EventType::Connected | EventType::Disconnected) {
                connected_changed = true;
            }
        }

        if force_rescan || connected_changed || last_metadata_scan.elapsed() >= Duration::from_secs(5) {
            let reopen_active_hid = force_rescan || connected_changed;
            let mut scanned = device_manager::scan_controllers(&gilrs);

            // Preserve descriptor-based class refinement across routine metadata
            // scans. Otherwise a generic multi-axis wheel could briefly revert to
            // "Generic HID" every five seconds even while its reader knows it is a
            // Racing Wheel/HOTAS/Pedals device.
            if semantic_reader.is_some() && active_id >= 0 {
                if let Some(old) = devices.iter().find(|d| d.id == active_id) {
                    if let Some(new) = scanned.iter_mut().find(|d| d.stable_key == old.stable_key) {
                        new.class = old.class;
                        new.display_name = old.display_name.clone();
                        new.backend = old.backend.clone();
                    }
                }
            }

            if devices_changed(&devices, &scanned) || force_rescan || connected_changed {
                devices = scanned;
                devices_revision = devices_revision.wrapping_add(1);
                let active_missing = active_id < 0 || !devices.iter().any(|d| d.id == active_id);
                let preferred_index = preferred_stable_key
                    .as_ref()
                    .and_then(|key| devices.iter().position(|d| &d.stable_key == key));
                let restore_preferred = preferred_index
                    .and_then(|index| devices.get(index))
                    .map(|device| device.id != active_id)
                    .unwrap_or(false);

                if active_missing || restore_preferred {
                    stop_active_vibration(&mut active_effect);
                    stop_vendor_outputs(&mut semantic_reader);
                    let chosen_index = preferred_index.or_else(|| if devices.is_empty() { None } else { Some(0) });
                    active_id = chosen_index.and_then(|index| devices.get(index)).map(|d| d.id).unwrap_or(-1);
                    if preferred_stable_key.is_none() {
                        preferred_stable_key = chosen_index
                            .and_then(|index| devices.get(index))
                            .map(|d| d.stable_key.clone());
                    }
                    reset_analysis(
                        &mut polling,
                        &mut left_diag,
                        &mut right_diag,
                        &mut left_trigger_diag,
                        &mut right_trigger_diag,
                        &mut left_circ,
                        &mut right_circ,
                        &mut calibration,
                    );
                    calibration_saved = false;
                    active_calibration = chosen_index
                        .and_then(|index| devices.get(index))
                        .and_then(|d| registry::load_calibration(&d.stable_key));
                    wheel_profile = chosen_index
                        .and_then(|index| devices.get(index))
                        .and_then(|d| registry::load_wheel_calibration(&d.stable_key))
                        .unwrap_or_default();
                    vendor_telemetry = VendorTelemetry::default();
                    semantic_input_cache = InputSnapshot::default();
                    semantic_axes_cache.clear();
                    let opened = open_semantic_hid_reader(&mut devices, active_id);
                    semantic_reader = opened.reader;
                    vendor_control_status = semantic_reader
                        .as_ref()
                        .map(|reader| reader.control_status().to_string())
                        .unwrap_or_else(|| "No vendor output protocol active".to_string());
                    if !raw_capture_paused {
                        raw_hid_report = opened.message.clone();
                    }
                    backend_status = opened.message;
                    if opened.device_changed {
                        devices_revision = devices_revision.wrapping_add(1);
                    }
                } else if reopen_active_hid {
                    // Device notifications/manual refresh can invalidate the old HID
                    // handle even when VID/PID/serial produce the same stable id.
                    stop_vendor_outputs(&mut semantic_reader);
                    let opened = open_semantic_hid_reader(&mut devices, active_id);
                    semantic_reader = opened.reader;
                    vendor_control_status = semantic_reader
                        .as_ref()
                        .map(|reader| reader.control_status().to_string())
                        .unwrap_or_else(|| "No vendor output protocol active".to_string());
                    semantic_input_cache = InputSnapshot::default();
                    semantic_axes_cache.clear();
                    if !raw_capture_paused {
                        raw_hid_report = opened.message.clone();
                    }
                    backend_status = opened.message;
                    if opened.device_changed {
                        devices_revision = devices_revision.wrapping_add(1);
                    }
                }
            } else {
                devices = scanned;
            }
            force_rescan = false;
            last_metadata_scan = Instant::now();
        }

        let mut input = InputSnapshot::default();
        let mut semantic_axes = Vec::<SemanticAxisValue>::new();
        let mut have_gilrs_input = false;

        if active_id >= 0 {
            if let Some((_, gamepad)) = gilrs
                .gamepads()
                .find(|(id, _)| usize::from(*id) as i32 == active_id)
            {
                let raw = InputSnapshot::from_gamepad(gamepad);
                calibration.record(raw.lx, raw.ly, raw.rx, raw.ry);
                if active_axis_event {
                    register_stick_analysis(
                        &raw,
                        &mut left_diag,
                        &mut right_diag,
                        &mut left_circ,
                        &mut right_circ,
                    );
                }
                input = apply_calibration(raw, active_calibration.as_ref());
                have_gilrs_input = true;
                backend_status = "gilrs normalized input + HID metadata".to_string();
            }
        }

        if let Some(reader) = &mut semantic_reader {
            if let Some(decoded) = reader.read_latest() {
                polling.register_event();
                active_event = true;
                semantic_input_cache = decoded.input;
                semantic_axes_cache = decoded.axes;
                if !raw_capture_paused {
                    raw_hid_report = format_hid_report(&decoded.raw_report);
                }
                vendor_telemetry = decoded.vendor;
                vendor_control_status = reader.control_status().to_string();
                if have_gilrs_input {
                    backend_status = "gilrs input + direct HID semantic telemetry".to_string();
                } else {
                    backend_status = "Direct HID semantic input • background-capable".to_string();
                    calibration.record(
                        semantic_input_cache.lx,
                        semantic_input_cache.ly,
                        semantic_input_cache.rx,
                        semantic_input_cache.ry,
                    );
                    register_stick_analysis(
                        &semantic_input_cache,
                        &mut left_diag,
                        &mut right_diag,
                        &mut left_circ,
                        &mut right_circ,
                    );
                }
            }
            semantic_axes = semantic_axes_cache.clone();
            if !have_gilrs_input {
                input = apply_calibration(semantic_input_cache.clone(), active_calibration.as_ref());
            }
        }

        // Drain the Win32 WM_INPUT transport as an independent background path.
        // We keep this distinct from direct HID: WM_INPUT is the fallback for
        // drivers that deliver reports to Windows but refuse a second shared HID open.
        if let Ok(monitor) = &raw_input_monitor {
            if let Some(packet) = monitor.latest() {
                if let Some(selected) = devices.iter().find(|d| d.id == active_id) {
                    if raw_packet_matches_device(&packet, selected) {
                        if !raw_capture_paused {
                            raw_input_report = format_raw_input_packet(&packet);
                        }
                        raw_input_status = if raw_capture_paused {
                            "WM_INPUT capture display paused • input remains active".to_string()
                        } else {
                            format!(
                                "WM_INPUT live • {} byte report × {}",
                                packet.report_size, packet.report_count
                            )
                        };
                        if semantic_reader.is_none() && !have_gilrs_input {
                            raw_hid_report = raw_input_report.clone();
                            backend_status = "WM_INPUT raw HID fallback • semantic descriptor not available".to_string();
                        }
                    }
                }
            }
        }

        if active_event {
            left_trigger_diag.register(input.lt);
            right_trigger_diag.register(input.rt);
        }

        // Wheels keep their semantic steering axis even when gilrs is also active.
        // Degrees are derived from the per-device configured lock-to-lock range;
        // HID does not expose the driver's active steering lock universally.
        let active_device = devices.iter().find(|d| d.id == active_id);
        let is_wheel = active_device.map(|d| d.class == DeviceClass::RacingWheel).unwrap_or(false);
        let steering_semantic = semantic_axes
            .iter()
            .find(|a| a.name == "Steering")
            .or_else(|| if is_wheel { semantic_axes.iter().find(|a| a.name == "X") } else { None });
        let (wheel_available, wheel_raw, wheel_source) = if let Some(axis) = steering_semantic {
            (true, axis.value, format!("HID {}", axis.name))
        } else if is_wheel && have_gilrs_input {
            (true, input.lx, "gilrs LeftStickX fallback".to_string())
        } else {
            (false, 0.0, "No steering axis".to_string())
        };
        if wheel_available {
            wheel_profile.update_observed(wheel_raw);
        }
        let wheel_angle_deg = if wheel_available { wheel_profile.angle_degrees(wheel_raw) } else { 0.0 };

        let polling_metrics = polling.update();
        let hz = polling_metrics.current_hz;
        {
            let mut s = state.write();
            if s.devices_revision != devices_revision {
                s.devices = devices.clone();
                s.devices_revision = devices_revision;
            }
            s.active_id = active_id;
            s.input = input;
            s.semantic_axes = semantic_axes;
            s.polling_hz = if active_event || hz > 0 { hz } else { 0 };
            s.polling_metrics = polling_metrics;
            s.poll_history = polling.history();
            s.raw_capture_paused = raw_capture_paused;
            s.left_stick = left_diag.metrics();
            s.right_stick = right_diag.metrics();
            s.left_trigger = left_trigger_diag.metrics();
            s.right_trigger = right_trigger_diag.metrics();
            s.left_circularity = left_circ.metrics();
            s.right_circularity = right_circ.metrics();
            s.calibration_state = calibration.state;
            s.calibration_saved = calibration_saved;
            let calibration_for_summary = if matches!(
                calibration.state,
                CalibrationState::Center | CalibrationState::Range | CalibrationState::Verify
            ) {
                calibration.current_profile()
            } else {
                active_calibration.as_ref().unwrap_or_else(|| calibration.current_profile())
            };
            s.calibration_summary = calibration_profile_summary(calibration_for_summary);
            s.backend_status = backend_status.clone();
            s.raw_hid_report = raw_hid_report.clone();
            s.raw_input_status = raw_input_status.clone();
            s.raw_input_report = raw_input_report.clone();
            s.vendor_protocol = vendor_telemetry.protocol.clone();
            s.battery_text = vendor_telemetry.power_text().unwrap_or_else(|| {
                active_device.map(|d| d.power_status.clone()).unwrap_or_else(|| "Not exposed".to_string())
            });
            s.firmware_detail = vendor_telemetry.firmware_detail.clone();
            s.gyro = vendor_telemetry.gyro.map(|v| [v[0] as i32, v[1] as i32, v[2] as i32]).unwrap_or([0; 3]);
            s.accel = vendor_telemetry.accel.map(|v| [v[0] as i32, v[1] as i32, v[2] as i32]).unwrap_or([0; 3]);
            s.gyro_dps = vendor_telemetry.gyro_dps.unwrap_or([0.0; 3]);
            s.accel_g = vendor_telemetry.accel_g.unwrap_or([0.0; 3]);
            s.motion_units = if vendor_telemetry.motion_units.is_empty() {
                "Raw units only".to_string()
            } else {
                vendor_telemetry.motion_units.clone()
            };
            s.power_source = if vendor_telemetry.power_source.is_empty() {
                active_device.map(|d| format!("{} / controller backend", d.transport.as_str())).unwrap_or_else(|| "Not exposed".to_string())
            } else {
                vendor_telemetry.power_source.clone()
            };
            s.motion_available = vendor_telemetry.has_motion();
            s.touch1_text = format_touch(&vendor_telemetry.touch[0]);
            s.touch2_text = format_touch(&vendor_telemetry.touch[1]);
            s.touch_active = vendor_telemetry.has_touch();
            s.wheel_available = wheel_available;
            s.wheel_raw = wheel_raw;
            s.wheel_angle_deg = wheel_angle_deg;
            s.wheel_rotation_deg = wheel_profile.rotation_degrees;
            s.wheel_observed_min = wheel_profile.observed_min;
            s.wheel_observed_max = wheel_profile.observed_max;
            s.wheel_source = wheel_source;
            s.adaptive_trigger_available = semantic_reader
                .as_ref()
                .map(|reader| reader.adaptive_trigger_available())
                .unwrap_or(false);
            s.nintendo_rumble_available = semantic_reader
                .as_ref()
                .map(|reader| reader.nintendo_rumble_available())
                .unwrap_or(false);
            s.vendor_control_status = vendor_control_status.clone();
        }

        let _keep_effect_alive = &active_effect;
        thread::sleep(Duration::from_millis(1));
    }
}

fn raw_packet_matches_device(packet: &RawInputPacket, device: &DeviceDescriptor) -> bool {
    if packet.device_path.is_empty() {
        return false;
    }
    let raw = packet.device_path.to_ascii_lowercase();
    let hid = device.hid_path.to_ascii_lowercase();
    if !hid.is_empty() && raw == hid {
        return true;
    }
    match (device.vendor_id, device.product_id) {
        (Some(vid), Some(pid)) => raw.contains(&format!("vid_{vid:04x}&pid_{pid:04x}")),
        _ => false,
    }
}

fn format_raw_input_packet(packet: &RawInputPacket) -> String {
    let shown = packet.report.iter().take(96).map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
    let suffix = if packet.report.len() > 96 { " …" } else { "" };
    format!(
        "WM_INPUT • {} × {} bytes • {}{}",
        packet.report_count, packet.report_size, shown, suffix
    )
}

fn format_touch(point: &crate::device_manager::vendor::TouchPoint) -> String {
    if !point.active {
        return "Inactive".to_string();
    }
    format!("ID {} • X {} / {} • Y {} / {}", point.id, point.x, point.max_x, point.y, point.max_y)
}

fn apply_calibration(raw: InputSnapshot, profile: Option<&CalibrationProfile>) -> InputSnapshot {
    let Some(profile) = profile else { return raw; };
    let mut input = raw.clone();
    let (lx, ly) = profile.left.apply_pair(raw.lx, raw.ly);
    let (rx, ry) = profile.right.apply_pair(raw.rx, raw.ry);
    input.lx = lx;
    input.ly = ly;
    input.rx = rx;
    input.ry = ry;
    input
}

fn register_stick_analysis(
    input: &InputSnapshot,
    left_diag: &mut StickDiagnostics,
    right_diag: &mut StickDiagnostics,
    left_circ: &mut CircularityTracker,
    right_circ: &mut CircularityTracker,
) {
    left_diag.register(input.lx, input.ly);
    right_diag.register(input.rx, input.ry);
    left_circ.register(input.lx, input.ly);
    right_circ.register(input.rx, input.ry);
}


fn calibration_profile_summary(profile: &CalibrationProfile) -> String {
    format!(
        "Left center {:+.3}/{:+.3} • DZ {:.3}   |   Right center {:+.3}/{:+.3} • DZ {:.3}",
        profile.left.center_x,
        profile.left.center_y,
        profile.left.deadzone,
        profile.right.center_x,
        profile.right.center_y,
        profile.right.deadzone,
    )
}

fn reset_analysis(
    polling: &mut PollingTracker,
    left_diag: &mut StickDiagnostics,
    right_diag: &mut StickDiagnostics,
    left_trigger_diag: &mut TriggerDiagnostics,
    right_trigger_diag: &mut TriggerDiagnostics,
    left_circ: &mut CircularityTracker,
    right_circ: &mut CircularityTracker,
    calibration: &mut CalibrationManager,
) {
    polling.reset();
    left_diag.reset();
    right_diag.reset();
    left_trigger_diag.reset();
    right_trigger_diag.reset();
    left_circ.reset();
    right_circ.reset();
    calibration.reset();
}

fn devices_changed(old: &[DeviceDescriptor], new: &[DeviceDescriptor]) -> bool {
    old.len() != new.len()
        || old.iter().zip(new).any(|(a, b)| {
            a.id != b.id
                || a.stable_key != b.stable_key
                || a.power_status != b.power_status
                || a.transport != b.transport
                || a.class != b.class
        })
}

struct OpenSemanticResult {
    reader: Option<SemanticHidReader>,
    message: String,
    device_changed: bool,
}

fn open_semantic_hid_reader(devices: &mut [DeviceDescriptor], active_id: i32) -> OpenSemanticResult {
    let Some(index) = devices.iter().position(|d| d.id == active_id) else {
        return OpenSemanticResult { reader: None, message: "No device selected".to_string(), device_changed: false };
    };
    let device = &devices[index];
    if device.hid_path.is_empty() {
        return OpenSemanticResult {
            reader: None,
            message: "No readable HID endpoint for this device • WM_INPUT remains available".to_string(),
            device_changed: false,
        };
    }

    let had_gilrs = device.backend.starts_with("gilrs");
    let path = device.hid_path.clone();
    let family = device.family;
    let class = device.class;
    match SemanticHidReader::open(&path, family, class) {
        Ok(reader) => {
            let refined = reader.refined_class();
            let names = reader.semantic_names().iter().take(12).cloned().collect::<Vec<_>>();
            let backend_name = if had_gilrs { "gilrs + HID semantic" } else { "HID semantic" };
            let changed = refined != devices[index].class || devices[index].backend != backend_name;
            if refined != devices[index].class {
                devices[index].class = refined;
                devices[index].display_name = format!("{} [{}]", devices[index].name, refined.as_str());
            }
            devices[index].backend = backend_name.to_string();
            let message = if names.is_empty() {
                "HID semantic backend active • descriptor has buttons/vendor fields only".to_string()
            } else {
                format!("HID semantic backend active • {}", names.join(", "))
            };
            OpenSemanticResult { reader: Some(reader), message, device_changed: changed }
        }
        Err(err) => OpenSemanticResult {
            reader: None,
            message: format!("HID semantic open failed: {err}"),
            device_changed: false,
        },
    }
}

fn format_hid_report(report: &[u8]) -> String {
    let shown = report
        .iter()
        .take(96)
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if report.len() > 96 {
        format!("{} bytes: {} …", report.len(), shown)
    } else {
        format!("{} bytes: {}", report.len(), shown)
    }
}


fn stop_vendor_outputs(reader: &mut Option<SemanticHidReader>) {
    if let Some(reader) = reader.as_mut() {
        let _ = reader.clear_adaptive_triggers();
        let _ = reader.stop_nintendo_hd_rumble();
    }
}

fn stop_active_vibration(active_effect: &mut Option<Effect>) {
    if let Some(effect) = active_effect.take() {
        let _ = effect.stop();
    }
}

fn play_vibration(
    gilrs: &mut Gilrs,
    active_id: i32,
    strong: f32,
    weak: f32,
    duration_ms: u32,
) -> Option<Effect> {
    if active_id < 0 {
        return None;
    }
    let id = gilrs
        .gamepads()
        .find(|(id, _)| usize::from(*id) as i32 == active_id)
        .map(|(id, _)| id)?;

    let strong = (strong.clamp(0.0, 1.0) * u16::MAX as f32) as u16;
    let weak = (weak.clamp(0.0, 1.0) * u16::MAX as f32) as u16;
    let duration_ms = duration_ms.clamp(20, 10_000);
    let replay_strong = Replay {
        play_for: Ticks::from_ms(duration_ms),
        with_delay: Ticks::from_ms(0),
        after: Ticks::from_ms(0),
    };
    let replay_weak = Replay {
        play_for: Ticks::from_ms(duration_ms),
        with_delay: Ticks::from_ms(0),
        after: Ticks::from_ms(0),
    };

    let mut builder = EffectBuilder::new();
    builder
        .add_effect(BaseEffect {
            kind: BaseEffectType::Strong { magnitude: strong },
            scheduling: replay_strong,
            envelope: Default::default(),
        })
        .add_effect(BaseEffect {
            kind: BaseEffectType::Weak { magnitude: weak },
            scheduling: replay_weak,
            envelope: Default::default(),
        })
        .gamepads(&[id]);

    let effect = builder.finish(gilrs).ok()?;
    effect.play().ok()?;
    Some(effect)
}
