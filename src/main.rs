#![windows_subsystem = "windows"]

mod calibrate;
mod config;
mod device_manager;
mod diagnostics;
mod input;
mod joykey;
mod runtime;
mod rendering;

slint::include_modules!();

use config::models::JoyKeyProfile;
use device_manager::vendor::{AdaptiveTriggerMode, AdaptiveTriggerSide, NintendoRumbleSide};
use runtime::{AppRuntime, RuntimeCommand, RuntimeSnapshot};
use slint::{ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--joykey-agent") {
        return joykey::run_headless_agent();
    }

    // Rendering is configured before the first Slint window is created.
    // Slint's Winit backend can render the complete UI through a GPU 2D
    // renderer or through the software renderer for driver/VM fallback.
    let renderer = rendering::configure(&args);

    let ui = MainWindow::new()?;
    ui.set_renderer_status(SharedString::from(renderer.status.clone()));
    ui.set_renderer_preference(SharedString::from(renderer.preference.as_str()));
    let runtime = Rc::new(AppRuntime::start()?);
    wire_callbacks(&ui, runtime.clone());
    refresh_joy_profiles(&ui);

    let timer = Timer::default();
    let weak = ui.as_weak();
    let runtime_tick = runtime.clone();
    let mut last_devices_revision = u64::MAX;
    let mut last_active_id = i32::MIN;
    let mut ui_frame: u8 = 0;

    timer.start(TimerMode::Repeated, Duration::from_millis(16), move || {
        let Some(ui) = weak.upgrade() else { return; };
        let snapshot = runtime_tick.snapshot();

        if snapshot.devices_revision != last_devices_revision {
            let controllers = snapshot.devices.iter().map(to_ui_controller).collect::<Vec<_>>();
            ui.set_controllers(ModelRc::new(VecModel::from(controllers)));
            last_devices_revision = snapshot.devices_revision;
        }

        if snapshot.active_id != last_active_id {
            ui.set_active_id(snapshot.active_id);
            if let Some(device) = snapshot.devices.iter().find(|d| d.id == snapshot.active_id) {
                ui.set_active_info(to_ui_controller(device));
            }
            last_active_id = snapshot.active_id;
        }

        // Keep live sticks/buttons at ~60 Hz, but update expensive text/model
        // diagnostics at ~30 Hz. The controller worker itself remains independent
        // and high-frequency, so diagnostics sampling is not reduced.
        ui_frame = ui_frame.wrapping_add(1);
        sync_ui(&ui, &snapshot, ui_frame % 2 == 0);
    });

    ui.run()?;
    drop(runtime);
    Ok(())
}

fn wire_callbacks(ui: &MainWindow, runtime: Rc<AppRuntime>) {
    let rt = runtime.clone();
    ui.on_refresh_devices(move || rt.command(RuntimeCommand::RefreshDevices));

    let rt = runtime.clone();
    ui.on_select_device(move |id| rt.command(RuntimeCommand::SelectDevice(id)));

    let rt = runtime.clone();
    ui.on_run_tool(move |tool| {
        let tool = tool.to_string();
        match tool.as_str() {
            "clear_circ" => rt.command(RuntimeCommand::ClearCircularity),
            "reset_diag" => rt.command(RuntimeCommand::ResetDiagnostics),
            "reset_calib" => rt.command(RuntimeCommand::CalibrationReset),
            _ => {}
        }
    });

    let rt = runtime.clone();
    ui.on_next_calibration_step(move || rt.command(RuntimeCommand::CalibrationNext));

    let rt = runtime.clone();
    ui.on_test_vibration(move |kind| {
        let (strong, weak, duration_ms) = match kind {
            0 => (1.0, 1.0, 1500),
            1 => (0.25, 0.25, 1000),
            2 => (1.0, 0.0, 350),
            3 => (0.0, 0.65, 300),
            _ => (0.0, 0.0, 100),
        };
        rt.command(RuntimeCommand::Vibrate { strong, weak, duration_ms });
    });

    let rt = runtime.clone();
    ui.on_test_adv_vibration(move |strong, weak, duration_ms| {
        rt.command(RuntimeCommand::Vibrate {
            strong,
            weak,
            duration_ms: duration_ms.clamp(20.0, 10_000.0) as u32,
        });
    });

    let rt = runtime.clone();
    ui.on_stop_vibration(move || rt.command(RuntimeCommand::StopVibration));

    let rt = runtime.clone();
    ui.on_set_adaptive_trigger(move |side, mode, start, end, strength, frequency| {
        let side = match side {
            0 => AdaptiveTriggerSide::Left,
            1 => AdaptiveTriggerSide::Right,
            _ => AdaptiveTriggerSide::Both,
        };
        let mode = match mode {
            1 => AdaptiveTriggerMode::Feedback,
            2 => AdaptiveTriggerMode::Weapon,
            3 => AdaptiveTriggerMode::Vibration,
            _ => AdaptiveTriggerMode::Off,
        };
        rt.command(RuntimeCommand::SetAdaptiveTrigger {
            side,
            mode,
            start: start.clamp(0.0, 1.0),
            end: end.clamp(0.0, 1.0),
            strength: strength.clamp(0.0, 1.0),
            frequency: frequency.clamp(1.0, 255.0) as u8,
        });
    });

    let rt = runtime.clone();
    ui.on_stop_adaptive_triggers(move || rt.command(RuntimeCommand::StopAdaptiveTriggers));

    let rt = runtime.clone();
    ui.on_set_nintendo_rumble(move |side, low_hz, high_hz, amplitude| {
        let side = match side {
            0 => NintendoRumbleSide::Left,
            1 => NintendoRumbleSide::Right,
            _ => NintendoRumbleSide::Both,
        };
        rt.command(RuntimeCommand::SetNintendoRumble {
            side,
            low_frequency_hz: low_hz,
            high_frequency_hz: high_hz,
            amplitude: (amplitude / 100.0).clamp(0.0, 1.0),
        });
    });

    let rt = runtime.clone();
    ui.on_stop_nintendo_rumble(move || rt.command(RuntimeCommand::StopNintendoRumble));

    let rt = runtime.clone();
    ui.on_set_wheel_rotation(move |degrees| rt.command(RuntimeCommand::SetWheelRotation(degrees)));

    let rt = runtime.clone();
    ui.on_center_wheel(move || rt.command(RuntimeCommand::CenterWheel));

    let rt = runtime.clone();
    ui.on_reset_wheel_range(move || rt.command(RuntimeCommand::ResetWheelRange));

    let rt = runtime.clone();
    ui.on_set_joykey_enabled(move |enabled| rt.command(RuntimeCommand::SetJoyKeyEnabled(enabled)));

    let rt = runtime.clone();
    ui.on_set_raw_capture_paused(move |paused| {
        rt.command(RuntimeCommand::SetRawCapturePaused(paused));
    });

    let weak = ui.as_weak();
    ui.on_set_renderer_preference(move |preference| {
        let Some(ui) = weak.upgrade() else { return; };
        let value = preference.to_string();
        match config::registry::save_renderer_preference(&value) {
            Ok(()) => {
                ui.set_renderer_preference(SharedString::from(value.clone()));
                ui.set_renderer_status(SharedString::from(format!(
                    "Renderer preference saved: {} • restart to apply",
                    value.to_ascii_uppercase()
                )));
            }
            Err(err) => ui.set_renderer_status(SharedString::from(format!(
                "Could not save renderer preference: {err}"
            ))),
        }
    });

    let rt = runtime.clone();
    let weak = ui.as_weak();
    ui.on_export_diagnostics(move || {
        let Some(ui) = weak.upgrade() else { return; };
        let snapshot = rt.snapshot();
        match export_diagnostics(&snapshot) {
            Ok(path) => ui.set_export_status(SharedString::from(format!(
                "Saved: {}", path.display()
            ))),
            Err(err) => ui.set_export_status(SharedString::from(format!(
                "Export failed: {err}"
            ))),
        }
    });

    let weak = ui.as_weak();
    ui.on_load_joy_profile(move |process| {
        let Some(ui) = weak.upgrade() else { return; };
        let process = process.trim().to_string();
        if process.is_empty() {
            ui.set_joy_status(SharedString::from("Enter a process name, e.g. game.exe"));
            return;
        }
        let profile = config::registry::load_joykey_profile(&process)
            .unwrap_or_else(|| JoyKeyProfile::new(process.clone()));
        ui.set_joy_process(SharedString::from(process));
        set_profile_fields(&ui, &profile);
        ui.set_joy_status(SharedString::from(if profile.mappings.is_empty() {
            "New profile • set mappings and Save"
        } else {
            "Profile loaded from Registry"
        }));
    });

    let weak = ui.as_weak();
    ui.on_save_joy_profile(move |process| {
        let Some(ui) = weak.upgrade() else { return; };
        let process = process.trim().to_string();
        if process.is_empty() {
            ui.set_joy_status(SharedString::from("Enter a process name, e.g. game.exe"));
            return;
        }
        let profile = collect_profile(&ui, process.clone());
        match config::registry::save_joykey_profile(&profile) {
            Ok(()) => {
                ui.set_joy_process(SharedString::from(process));
                refresh_joy_profiles(&ui);
                ui.set_joy_status(SharedString::from(
                    "Saved • background agent reloads mappings on the next press",
                ));
            }
            Err(err) => ui.set_joy_status(SharedString::from(format!("Save failed: {err}"))),
        }
    });

    let weak = ui.as_weak();
    ui.on_delete_joy_profile(move |process| {
        let Some(ui) = weak.upgrade() else { return; };
        let process = process.trim().to_string();
        if process.is_empty() {
            ui.set_joy_status(SharedString::from("Select a profile to delete"));
            return;
        }
        match config::registry::delete_joykey_profile(&process) {
            Ok(()) => {
                let blank = JoyKeyProfile::new(process.clone());
                set_profile_fields(&ui, &blank);
                refresh_joy_profiles(&ui);
                ui.set_joy_status(SharedString::from(format!("Deleted profile: {process}")));
            }
            Err(err) => ui.set_joy_status(SharedString::from(format!("Delete failed: {err}"))),
        }
    });

    let weak = ui.as_weak();
    ui.on_refresh_joy_profiles(move || {
        if let Some(ui) = weak.upgrade() {
            refresh_joy_profiles(&ui);
            ui.set_joy_status(SharedString::from("Profile list refreshed"));
        }
    });
}

fn sync_ui(ui: &MainWindow, s: &RuntimeSnapshot, update_details: bool) {
    ui.set_axis_lx(s.input.lx);
    ui.set_axis_ly(s.input.ly);
    ui.set_axis_rx(s.input.rx);
    ui.set_axis_ry(s.input.ry);
    ui.set_l_deviation((s.input.lx * s.input.lx + s.input.ly * s.input.ly).sqrt());
    ui.set_r_deviation((s.input.rx * s.input.rx + s.input.ry * s.input.ry).sqrt());
    ui.set_trigger_l2(s.input.lt);
    ui.set_trigger_r2(s.input.rt);
    ui.set_raw_axes(ModelRc::new(VecModel::from(s.input.axes.to_vec())));
    ui.set_raw_buttons(ModelRc::new(VecModel::from(s.input.buttons.to_vec())));
    if update_details {
        ui.set_semantic_axis_names(ModelRc::new(VecModel::from(
            s.semantic_axes
                .iter()
                .map(|axis| SharedString::from(axis.name.clone()))
                .collect::<Vec<_>>(),
        )));
        ui.set_semantic_axis_values(ModelRc::new(VecModel::from(
            s.semantic_axes.iter().map(|axis| axis.value).collect::<Vec<_>>(),
        )));
    }
    ui.set_current_hz(SharedString::from(if s.polling_hz == 0 {
        "Idle".to_string()
    } else {
        format!("{} Hz", s.polling_hz)
    }));
    ui.set_poll_avg_hz(s.polling_metrics.average_hz);
    ui.set_poll_min_hz(s.polling_metrics.min_hz.min(i32::MAX as u32) as i32);
    ui.set_poll_max_hz(s.polling_metrics.max_hz.min(i32::MAX as u32) as i32);
    ui.set_poll_interval_ms(s.polling_metrics.average_interval_ms);
    ui.set_poll_jitter_ms(s.polling_metrics.jitter_ms);
    ui.set_poll_total_events(SharedString::from(s.polling_metrics.total_events.to_string()));
    if update_details {
        ui.set_poll_history(ModelRc::new(VecModel::from(s.poll_history.clone())));
    }
    ui.set_trigger_l_min(s.left_trigger.min);
    ui.set_trigger_l_max(s.left_trigger.max);
    ui.set_trigger_l_travel(s.left_trigger.travel);
    ui.set_trigger_r_min(s.right_trigger.min);
    ui.set_trigger_r_max(s.right_trigger.max);
    ui.set_trigger_r_travel(s.right_trigger.travel);
    ui.set_trigger_samples(SharedString::from(
        s.left_trigger.samples.max(s.right_trigger.samples).to_string(),
    ));
    ui.set_raw_capture_paused(s.raw_capture_paused);
    ui.set_input_pressed(SharedString::from(pressed_buttons_text(&s.input.buttons)));
    ui.set_input_axes_summary(SharedString::from(format!(
        "LX {:+.3}   LY {:+.3}   RX {:+.3}   RY {:+.3}   LT {:>5.1}%   RT {:>5.1}%",
        s.input.lx,
        s.input.ly,
        s.input.rx,
        s.input.ry,
        s.input.lt * 100.0,
        s.input.rt * 100.0
    )));

    ui.set_l_circ_error(s.left_circularity.avg_error);
    ui.set_r_circ_error(s.right_circularity.avg_error);
    ui.set_l_circ_max(s.left_circularity.max_error);
    ui.set_r_circ_max(s.right_circularity.max_error);
    ui.set_l_q1(s.left_circularity.quadrant_error[0]);
    ui.set_l_q2(s.left_circularity.quadrant_error[1]);
    ui.set_l_q3(s.left_circularity.quadrant_error[2]);
    ui.set_l_q4(s.left_circularity.quadrant_error[3]);
    ui.set_r_q1(s.right_circularity.quadrant_error[0]);
    ui.set_r_q2(s.right_circularity.quadrant_error[1]);
    ui.set_r_q3(s.right_circularity.quadrant_error[2]);
    ui.set_r_q4(s.right_circularity.quadrant_error[3]);

    ui.set_l_center_offset(s.left_stick.center_offset);
    ui.set_l_noise(s.left_stick.noise);
    ui.set_l_peak_drift(s.left_stick.peak_drift);
    ui.set_l_recommended_dz(s.left_stick.recommended_deadzone);
    ui.set_r_center_offset(s.right_stick.center_offset);
    ui.set_r_noise(s.right_stick.noise);
    ui.set_r_peak_drift(s.right_stick.peak_drift);
    ui.set_r_recommended_dz(s.right_stick.recommended_deadzone);

    ui.set_calib_state(match s.calibration_state {
        calibrate::CalibrationState::Connect => 0,
        calibrate::CalibrationState::Center => 1,
        calibrate::CalibrationState::Range => 2,
        calibrate::CalibrationState::Verify => 3,
        calibrate::CalibrationState::Complete => 4,
    });
    ui.set_calib_saved(s.calibration_saved);
    ui.set_calib_summary(SharedString::from(s.calibration_summary.clone()));
    ui.set_joykey_enabled(s.joykey_enabled);
    if update_details {
        ui.set_backend_status(SharedString::from(s.backend_status.clone()));
        ui.set_raw_hid_report(SharedString::from(s.raw_hid_report.clone()));
        ui.set_raw_input_status(SharedString::from(s.raw_input_status.clone()));
        ui.set_raw_input_report(SharedString::from(s.raw_input_report.clone()));
        ui.set_vendor_protocol(SharedString::from(s.vendor_protocol.clone()));
        ui.set_battery_text(SharedString::from(s.battery_text.clone()));
        ui.set_firmware_detail(SharedString::from(s.firmware_detail.clone()));
    }
    ui.set_gyro_x(s.gyro[0]);
    ui.set_gyro_y(s.gyro[1]);
    ui.set_gyro_z(s.gyro[2]);
    ui.set_accel_x(s.accel[0]);
    ui.set_accel_y(s.accel[1]);
    ui.set_accel_z(s.accel[2]);
    ui.set_gyro_dps_x(s.gyro_dps[0]);
    ui.set_gyro_dps_y(s.gyro_dps[1]);
    ui.set_gyro_dps_z(s.gyro_dps[2]);
    ui.set_accel_g_x(s.accel_g[0]);
    ui.set_accel_g_y(s.accel_g[1]);
    ui.set_accel_g_z(s.accel_g[2]);
    if update_details {
        ui.set_motion_units(SharedString::from(s.motion_units.clone()));
        ui.set_power_source(SharedString::from(s.power_source.clone()));
    }
    ui.set_motion_available(s.motion_available);
    if update_details {
        ui.set_touch1_text(SharedString::from(s.touch1_text.clone()));
        ui.set_touch2_text(SharedString::from(s.touch2_text.clone()));
    }
    ui.set_touch_active(s.touch_active);
    ui.set_wheel_available(s.wheel_available);
    ui.set_wheel_raw(s.wheel_raw);
    ui.set_wheel_angle_deg(s.wheel_angle_deg);
    ui.set_wheel_rotation_deg(s.wheel_rotation_deg);
    ui.set_wheel_observed_min(s.wheel_observed_min);
    ui.set_wheel_observed_max(s.wheel_observed_max);
    if update_details {
        ui.set_wheel_source(SharedString::from(s.wheel_source.clone()));
    }
    ui.set_adaptive_trigger_available(s.adaptive_trigger_available);
    ui.set_nintendo_rumble_available(s.nintendo_rumble_available);
    if update_details {
        ui.set_vendor_control_status(SharedString::from(s.vendor_control_status.clone()));
    }
}

fn pressed_buttons_text(buttons: &[bool]) -> String {
    const LABELS: [&str; 32] = [
        "South/A/Cross", "East/B/Circle", "West/X/Square", "North/Y/Triangle",
        "LB/L1", "RB/R1", "Select/View", "Start/Menu", "L3", "R3",
        "D-Pad Up", "D-Pad Right", "D-Pad Down", "D-Pad Left", "Guide/PS/Home",
        "LT/L2", "RT/R2", "Touchpad", "B1", "B2", "B3", "B4", "B5", "B6",
        "B7", "B8", "B9", "B10", "B11", "B12", "B13", "B14",
    ];
    let pressed = buttons
        .iter()
        .enumerate()
        .filter_map(|(index, pressed)| (*pressed).then(|| LABELS.get(index).copied().unwrap_or("Button")))
        .collect::<Vec<_>>();
    if pressed.is_empty() { "None".to_string() } else { pressed.join("  •  ") }
}

fn export_diagnostics(snapshot: &RuntimeSnapshot) -> anyhow::Result<PathBuf> {
    let active = snapshot.devices.iter().find(|device| device.id == snapshot.active_id);
    let device_json = active.map(|device| serde_json::json!({
        "id": device.id,
        "name": device.display_name,
        "uuid": device.uuid,
        "stable_key": device.stable_key,
        "class": device.class.as_str(),
        "family": device.family.description(),
        "manufacturer": device.manufacturer,
        "vid_pid": device.vid_pid(),
        "transport": device.transport.as_str(),
        "firmware": device.firmware,
        "hid_release_number": device.release_number,
        "hid_interface_number": device.interface_number,
        "serial": device.serial,
        "hid_usage": device.hid_usage_text(),
        "hid_path": device.hid_path,
        "backend": device.backend,
        "artwork": device.artwork,
        "force_feedback": device.ff_supported,
        "power_status": device.power_status,
    })).unwrap_or(serde_json::Value::Null);

    let report = serde_json::json!({
        "application": {
            "name": "IBBE Controller Manager",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "device": device_json,
        "polling": {
            "current_hz": snapshot.polling_metrics.current_hz,
            "average_hz": snapshot.polling_metrics.average_hz,
            "min_hz": snapshot.polling_metrics.min_hz,
            "max_hz": snapshot.polling_metrics.max_hz,
            "average_interval_ms": snapshot.polling_metrics.average_interval_ms,
            "jitter_ms": snapshot.polling_metrics.jitter_ms,
            "total_events": snapshot.polling_metrics.total_events,
        },
        "input": {
            "lx": snapshot.input.lx,
            "ly": snapshot.input.ly,
            "rx": snapshot.input.rx,
            "ry": snapshot.input.ry,
            "lt": snapshot.input.lt,
            "rt": snapshot.input.rt,
            "axes": snapshot.input.axes.to_vec(),
            "buttons": snapshot.input.buttons.to_vec(),
            "semantic_axes": snapshot.semantic_axes.iter().map(|axis| serde_json::json!({
                "name": axis.name,
                "value": axis.value,
            })).collect::<Vec<_>>(),
        },
        "diagnostics": {
            "left_stick": {
                "center_x": snapshot.left_stick.center_x,
                "center_y": snapshot.left_stick.center_y,
                "center_offset": snapshot.left_stick.center_offset,
                "noise": snapshot.left_stick.noise,
                "peak_drift": snapshot.left_stick.peak_drift,
                "recommended_deadzone": snapshot.left_stick.recommended_deadzone,
                "samples": snapshot.left_stick.samples,
            },
            "right_stick": {
                "center_x": snapshot.right_stick.center_x,
                "center_y": snapshot.right_stick.center_y,
                "center_offset": snapshot.right_stick.center_offset,
                "noise": snapshot.right_stick.noise,
                "peak_drift": snapshot.right_stick.peak_drift,
                "recommended_deadzone": snapshot.right_stick.recommended_deadzone,
                "samples": snapshot.right_stick.samples,
            },
            "left_trigger": {
                "min": snapshot.left_trigger.min,
                "max": snapshot.left_trigger.max,
                "travel": snapshot.left_trigger.travel,
                "samples": snapshot.left_trigger.samples,
            },
            "right_trigger": {
                "min": snapshot.right_trigger.min,
                "max": snapshot.right_trigger.max,
                "travel": snapshot.right_trigger.travel,
                "samples": snapshot.right_trigger.samples,
            },
            "left_circularity": {
                "average_error": snapshot.left_circularity.avg_error,
                "max_error": snapshot.left_circularity.max_error,
                "samples": snapshot.left_circularity.samples,
                "quadrants": snapshot.left_circularity.quadrant_error,
            },
            "right_circularity": {
                "average_error": snapshot.right_circularity.avg_error,
                "max_error": snapshot.right_circularity.max_error,
                "samples": snapshot.right_circularity.samples,
                "quadrants": snapshot.right_circularity.quadrant_error,
            },
            "calibration_working_profile": snapshot.calibration_summary,
            "polling_history_hz": snapshot.poll_history,
        },
        "transport": {
            "backend_status": snapshot.backend_status,
            "raw_hid_report": snapshot.raw_hid_report,
            "raw_input_status": snapshot.raw_input_status,
            "raw_input_report": snapshot.raw_input_report,
            "raw_capture_paused": snapshot.raw_capture_paused,
        },
        "vendor": {
            "protocol": snapshot.vendor_protocol,
            "battery": snapshot.battery_text,
            "firmware_detail": snapshot.firmware_detail,
            "gyro_raw": snapshot.gyro,
            "accel_raw": snapshot.accel,
            "gyro_dps": snapshot.gyro_dps,
            "accel_g": snapshot.accel_g,
            "motion_units": snapshot.motion_units,
            "power_source": snapshot.power_source,
            "nintendo_hd_rumble_available": snapshot.nintendo_rumble_available,
            "touch1": snapshot.touch1_text,
            "touch2": snapshot.touch2_text,
        }
    });

    let base = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|path| path.join("Documents"))
        .unwrap_or(std::env::current_dir()?);
    let directory = base.join("IBBE Controller Manager").join("Diagnostics");
    fs::create_dir_all(&directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let file = directory.join(format!("controller-diagnostics-{stamp}.json"));
    fs::write(&file, serde_json::to_vec_pretty(&report)?)?;
    Ok(file)
}

fn to_ui_controller(d: &device_manager::model::DeviceDescriptor) -> ControllerInfo {
    ControllerInfo {
        id: d.id,
        name: SharedString::from(d.display_name.clone()),
        uuid: SharedString::from(d.uuid.clone()),
        power_status: SharedString::from(d.power_status.clone()),
        layout: SharedString::from(d.layout.clone()),
        artwork: SharedString::from(d.artwork.clone()),
        ff_supported: d.ff_supported,
        manufacturer: SharedString::from(d.manufacturer.clone()),
        vid_pid: SharedString::from(d.vid_pid()),
        firmware: SharedString::from(d.firmware.clone()),
        release_number: SharedString::from(d.release_number.map(|v| format!("0x{v:04X}")).unwrap_or_else(|| "N/A".to_string())),
        interface_number: SharedString::from(d.interface_number.map(|v| v.to_string()).unwrap_or_else(|| "N/A".to_string())),
        serial: SharedString::from(if d.serial.is_empty() { "N/A".to_string() } else { d.serial.clone() }),
        device_class: SharedString::from(d.class.as_str()),
        family: SharedString::from(d.family.description()),
        transport: SharedString::from(d.transport.as_str()),
        hid_usage: SharedString::from(d.hid_usage_text()),
        hid_path: SharedString::from(if d.hid_path.is_empty() { "N/A".to_string() } else { d.hid_path.clone() }),
        backend: SharedString::from(d.backend.clone()),
        input_available: d.backend.starts_with("gilrs") || d.backend == "HID semantic",
    }
}

fn refresh_joy_profiles(ui: &MainWindow) {
    let profiles = config::registry::list_joykey_profiles()
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>();
    ui.set_joy_profiles(ModelRc::new(VecModel::from(profiles)));
}

fn set_profile_fields(ui: &MainWindow, profile: &JoyKeyProfile) {
    ui.set_joy_south(SharedString::from(mapping(profile, "South")));
    ui.set_joy_east(SharedString::from(mapping(profile, "East")));
    ui.set_joy_west(SharedString::from(mapping(profile, "West")));
    ui.set_joy_north(SharedString::from(mapping(profile, "North")));
    ui.set_joy_lb(SharedString::from(mapping(profile, "LeftShoulder")));
    ui.set_joy_rb(SharedString::from(mapping(profile, "RightShoulder")));
    ui.set_joy_lt(SharedString::from(mapping(profile, "LeftTrigger")));
    ui.set_joy_rt(SharedString::from(mapping(profile, "RightTrigger")));
    ui.set_joy_start(SharedString::from(mapping(profile, "Start")));
    ui.set_joy_select(SharedString::from(mapping(profile, "Select")));
    ui.set_joy_guide(SharedString::from(mapping(profile, "Guide")));
    ui.set_joy_lthumb(SharedString::from(mapping(profile, "LeftThumb")));
    ui.set_joy_rthumb(SharedString::from(mapping(profile, "RightThumb")));
    ui.set_joy_touchpad(SharedString::from(mapping(profile, "Touchpad")));
    ui.set_joy_dpad_up(SharedString::from(mapping(profile, "DPadUp")));
    ui.set_joy_dpad_down(SharedString::from(mapping(profile, "DPadDown")));
    ui.set_joy_dpad_left(SharedString::from(mapping(profile, "DPadLeft")));
    ui.set_joy_dpad_right(SharedString::from(mapping(profile, "DPadRight")));
    ui.set_joy_b1(SharedString::from(mapping(profile, "B1")));
    ui.set_joy_b2(SharedString::from(mapping(profile, "B2")));
    ui.set_joy_b3(SharedString::from(mapping(profile, "B3")));
    ui.set_joy_b4(SharedString::from(mapping(profile, "B4")));
    ui.set_joy_b5(SharedString::from(mapping(profile, "B5")));
    ui.set_joy_b6(SharedString::from(mapping(profile, "B6")));
    ui.set_joy_b7(SharedString::from(mapping(profile, "B7")));
    ui.set_joy_b8(SharedString::from(mapping(profile, "B8")));
}

fn collect_profile(ui: &MainWindow, process: String) -> JoyKeyProfile {
    let mut mappings = BTreeMap::new();
    insert_mapping(&mut mappings, "South", &ui.get_joy_south());
    insert_mapping(&mut mappings, "East", &ui.get_joy_east());
    insert_mapping(&mut mappings, "West", &ui.get_joy_west());
    insert_mapping(&mut mappings, "North", &ui.get_joy_north());
    insert_mapping(&mut mappings, "LeftShoulder", &ui.get_joy_lb());
    insert_mapping(&mut mappings, "RightShoulder", &ui.get_joy_rb());
    insert_mapping(&mut mappings, "LeftTrigger", &ui.get_joy_lt());
    insert_mapping(&mut mappings, "RightTrigger", &ui.get_joy_rt());
    insert_mapping(&mut mappings, "Start", &ui.get_joy_start());
    insert_mapping(&mut mappings, "Select", &ui.get_joy_select());
    insert_mapping(&mut mappings, "Guide", &ui.get_joy_guide());
    insert_mapping(&mut mappings, "LeftThumb", &ui.get_joy_lthumb());
    insert_mapping(&mut mappings, "RightThumb", &ui.get_joy_rthumb());
    insert_mapping(&mut mappings, "Touchpad", &ui.get_joy_touchpad());
    insert_mapping(&mut mappings, "DPadUp", &ui.get_joy_dpad_up());
    insert_mapping(&mut mappings, "DPadDown", &ui.get_joy_dpad_down());
    insert_mapping(&mut mappings, "DPadLeft", &ui.get_joy_dpad_left());
    insert_mapping(&mut mappings, "DPadRight", &ui.get_joy_dpad_right());
    insert_mapping(&mut mappings, "B1", &ui.get_joy_b1());
    insert_mapping(&mut mappings, "B2", &ui.get_joy_b2());
    insert_mapping(&mut mappings, "B3", &ui.get_joy_b3());
    insert_mapping(&mut mappings, "B4", &ui.get_joy_b4());
    insert_mapping(&mut mappings, "B5", &ui.get_joy_b5());
    insert_mapping(&mut mappings, "B6", &ui.get_joy_b6());
    insert_mapping(&mut mappings, "B7", &ui.get_joy_b7());
    insert_mapping(&mut mappings, "B8", &ui.get_joy_b8());
    JoyKeyProfile { process, enabled: true, mappings }
}

fn mapping<'a>(profile: &'a JoyKeyProfile, key: &str) -> &'a str {
    profile.mappings.get(key).map(String::as_str).unwrap_or("")
}

fn insert_mapping(map: &mut BTreeMap<String, String>, action: &str, value: &SharedString) {
    let value = value.trim();
    if !value.is_empty() {
        map.insert(action.to_string(), value.to_string());
    }
}
