use super::model::{ControllerFamily, DeviceClass};
use crate::input::InputSnapshot;
use super::vendor::{
    self, AdaptiveTriggerMode, AdaptiveTriggerParams, AdaptiveTriggerSide,
    NintendoRumbleParams, NintendoRumbleSide, VendorTelemetry,
};

#[derive(Debug, Clone, Default)]
pub struct SemanticAxisValue {
    pub name: String,
    pub value: f32,
}

#[derive(Debug, Clone, Default)]
pub struct SemanticHidSnapshot {
    pub input: InputSnapshot,
    pub axes: Vec<SemanticAxisValue>,
    pub button_usages: Vec<(u16, bool)>,
    pub raw_report: Vec<u8>,
    pub vendor: VendorTelemetry,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use anyhow::{Context, Result};
    use hidreport::{Field, Report, ReportDescriptor, VariableField};
    use std::ffi::CString;

    pub struct SemanticHidReader {
        device: hidapi::HidDevice,
        descriptor: ReportDescriptor,
        family: ControllerFamily,
        class: DeviceClass,
        semantic_names: Vec<String>,
        last_input: InputSnapshot,
        axis_values: std::collections::BTreeMap<String, f32>,
        firmware_detail: String,
        bus_type: hidapi::BusType,
        output_sequence: u8,
        control_status: String,
    }

    impl SemanticHidReader {
        pub fn open(path: &str, family: ControllerFamily, class: DeviceClass) -> Result<Self> {
            let api = hidapi::HidApi::new().context("HID API initialization failed")?;
            let path = CString::new(path).context("HID path contained an embedded NUL")?;
            let device = api.open_path(path.as_c_str()).context("Unable to open HID path")?;
            let bus_type = device
                .get_device_info()
                .map(|info| info.bus_type())
                .unwrap_or(hidapi::BusType::Unknown);

            let control_status = if matches!(
                family,
                ControllerFamily::NintendoSwitchPro
                    | ControllerFamily::NintendoJoyConLeft
                    | ControllerFamily::NintendoJoyConRight
            ) {
                vendor::initialize_nintendo_controller(&device, bus_type)
            } else if matches!(family, ControllerFamily::DualSense | ControllerFamily::DualSenseEdge) {
                format!("DualSense adaptive-trigger output ready • {:?}", bus_type)
            } else {
                "No vendor output protocol required".to_string()
            };

            let _ = device.set_blocking_mode(false);

            let mut bytes = vec![0u8; 4096];
            let len = device
                .get_report_descriptor(&mut bytes)
                .context("Unable to read HID report descriptor")?;
            bytes.truncate(len);
            let descriptor = ReportDescriptor::try_from(bytes.as_slice())
                .map_err(|e| anyhow::anyhow!("Unable to parse HID report descriptor: {e}"))?;

            let semantic_names = descriptor
                .input_reports()
                .iter()
                .flat_map(|report| report.fields())
                .filter_map(|field| match field {
                    Field::Variable(var) => usage_name(var).map(str::to_string),
                    _ => None,
                })
                .collect::<Vec<_>>();

            let class = refine_class(class, &semantic_names);
            let axis_values = semantic_names
                .iter()
                .cloned()
                .map(|name| (name, 0.0))
                .collect();

            let firmware_detail = vendor::read_firmware_feature(&device, family);

            Ok(Self {
                device,
                descriptor,
                family,
                class,
                semantic_names,
                last_input: InputSnapshot::default(),
                axis_values,
                firmware_detail,
                bus_type,
                output_sequence: 0,
                control_status,
            })
        }

        pub fn semantic_names(&self) -> &[String] {
            &self.semantic_names
        }

        pub fn refined_class(&self) -> DeviceClass {
            refine_class(self.class, &self.semantic_names)
        }

        pub fn adaptive_trigger_available(&self) -> bool {
            matches!(self.family, ControllerFamily::DualSense | ControllerFamily::DualSenseEdge)
        }

        pub fn nintendo_rumble_available(&self) -> bool {
            matches!(
                self.family,
                ControllerFamily::NintendoSwitchPro
                    | ControllerFamily::NintendoJoyConLeft
                    | ControllerFamily::NintendoJoyConRight
            )
        }

        pub fn set_nintendo_hd_rumble(
            &mut self,
            side: NintendoRumbleSide,
            params: NintendoRumbleParams,
        ) -> Result<()> {
            if !self.nintendo_rumble_available() {
                anyhow::bail!("Nintendo HD rumble is available only for Switch Pro / Joy-Con protocols");
            }
            vendor::nintendo_set_hd_rumble(&self.device, &mut self.output_sequence, side, params)?;
            self.control_status = format!(
                "Nintendo HD rumble • {:?} • LF {:.0} Hz • HF {:.0} Hz • {:.0}%",
                side,
                params.low_frequency_hz,
                params.high_frequency_hz,
                params.amplitude.clamp(0.0, 1.0) * 100.0
            );
            Ok(())
        }

        pub fn stop_nintendo_hd_rumble(&mut self) -> Result<()> {
            if !self.nintendo_rumble_available() {
                return Ok(());
            }
            vendor::nintendo_stop_hd_rumble(&self.device, &mut self.output_sequence)?;
            self.control_status = "Nintendo HD rumble stopped".to_string();
            Ok(())
        }

        pub fn control_status(&self) -> &str {
            &self.control_status
        }

        pub fn set_adaptive_triggers(
            &mut self,
            side: AdaptiveTriggerSide,
            mode: AdaptiveTriggerMode,
            params: AdaptiveTriggerParams,
        ) -> Result<()> {
            if !self.adaptive_trigger_available() {
                anyhow::bail!("Adaptive triggers are available only for DualSense / DualSense Edge");
            }
            vendor::dualsense_set_adaptive_triggers(
                &self.device,
                self.bus_type,
                &mut self.output_sequence,
                side,
                mode,
                params,
            )?;
            self.control_status = format!("DualSense adaptive triggers • {:?} • {:?}", side, mode);
            Ok(())
        }

        pub fn clear_adaptive_triggers(&mut self) -> Result<()> {
            if !self.adaptive_trigger_available() {
                return Ok(());
            }
            vendor::dualsense_set_adaptive_triggers(
                &self.device,
                self.bus_type,
                &mut self.output_sequence,
                AdaptiveTriggerSide::Both,
                AdaptiveTriggerMode::Off,
                AdaptiveTriggerParams::default(),
            )?;
            self.control_status = "DualSense adaptive triggers off".to_string();
            Ok(())
        }

        pub fn read_latest(&mut self) -> Option<SemanticHidSnapshot> {
            let mut latest = None;
            for _ in 0..32 {
                let mut buffer = [0u8; 1024];
                match self.device.read_timeout(&mut buffer, 0) {
                    Ok(0) => break,
                    Ok(len) => latest = Some(buffer[..len].to_vec()),
                    Err(_) => break,
                }
            }
            let bytes = latest?;
            self.decode(&bytes)
        }

        fn decode(&mut self, bytes: &[u8]) -> Option<SemanticHidSnapshot> {
            let report = self.descriptor.find_input_report(bytes)?;
            let mut vendor_state = vendor::parse_report(self.family, bytes);
            if vendor_state.firmware_detail.is_empty() {
                vendor_state.firmware_detail = self.firmware_detail.clone();
            }
            if matches!(
                self.family,
                ControllerFamily::NintendoSwitchPro
                    | ControllerFamily::NintendoJoyConLeft
                    | ControllerFamily::NintendoJoyConRight
            ) && bytes.first().copied() == Some(0x30)
            {
                self.control_status = "Nintendo full report 0x30 active • IMU report stream active • HD-rumble channel initialized".to_string();
            }
            let mut snapshot = SemanticHidSnapshot {
                input: self.last_input.clone(),
                raw_report: bytes.to_vec(),
                vendor: vendor_state,
                ..Default::default()
            };
            let mut axis_slot = 0usize;
            let mut hat_value: Option<i32> = None;

            for field in report.fields() {
                let Field::Variable(var) = field else { continue; };
                let value = match var.extract(bytes) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let page = u16::from(var.usage.usage_page);
                let usage = u16::from(var.usage.usage_id);
                let signed_value = if value.is_signed() {
                    i32::from(&value)
                } else {
                    u32::from(&value) as i32
                };

                match page {
                    0x09 => {
                        let pressed = signed_value != 0;
                        snapshot.button_usages.push((usage, pressed));
                        snapshot.input.set_physical_button(usage, pressed);
                        if matches!(self.class, DeviceClass::Gamepad | DeviceClass::ArcadeStick) {
                            if let Some(action) = button_action_for_usage(self.family, usage) {
                                snapshot.input.set_action(action, pressed);
                            }
                        }
                    }
                    0x01 => {
                        if usage == 0x39 {
                            hat_value = Some(signed_value);
                            continue;
                        }
                        if let Some(action) = generic_desktop_dpad_action(usage) {
                            snapshot.input.set_action(action, signed_value != 0);
                            continue;
                        }
                        if let Some(name) = generic_desktop_axis_name(usage) {
                            let value = normalize_variable(var, signed_value, is_centered_axis(page, usage, self.class));
                            push_axis(&mut snapshot, &mut axis_slot, name, value);
                            apply_axis_to_standard_input(&mut snapshot.input, name, value, self.class);
                        }
                    }
                    0x02 => {
                        if let Some(name) = simulation_axis_name(usage) {
                            let centered = matches!(name, "Steering" | "Aileron" | "Elevator" | "Rudder" | "Anti-Torque" | "Collective");
                            let value = normalize_variable(var, signed_value, centered);
                            push_axis(&mut snapshot, &mut axis_slot, name, value);
                            apply_axis_to_standard_input(&mut snapshot.input, name, value, self.class);
                        } else if matches!(usage, 0xB3 | 0xB4 | 0xB7 | 0xBC | 0xBD | 0xBE | 0xC0 | 0xC1 | 0xC2) {
                            // Simulation controls may expose momentary controls outside the
                            // Button page. Mirror these after the ordinary physical buttons.
                            let pseudo = 16u16.saturating_add(usage.saturating_sub(0xB3));
                            snapshot.input.set_physical_button(pseudo, signed_value != 0);
                        }
                    }
                    0x85 => {
                        // HID Battery System usages. Many controllers use vendor-specific
                        // reports instead, but standards-compliant devices can expose
                        // remaining capacity and charging state directly in the descriptor.
                        match usage {
                            0x44 => snapshot.vendor.charging = Some(signed_value != 0), // Charging
                            0x45 => if signed_value != 0 { snapshot.vendor.charging = Some(false); }, // Discharging
                            0x66 => {
                                let min = i32::from(&var.logical_minimum) as f32;
                                let max = i32::from(&var.logical_maximum) as f32;
                                if max > min {
                                    let pct = (((signed_value as f32 - min) / (max - min)) * 100.0).clamp(0.0, 100.0) as u8;
                                    snapshot.vendor.battery_percent = Some(pct);
                                    if snapshot.vendor.protocol.is_empty() {
                                        snapshot.vendor.protocol = "HID Battery System".to_string();
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }

            if let Some(hat) = hat_value {
                apply_hat(&mut snapshot.input, hat);
            }
            snapshot.input.set_action("LeftTrigger", snapshot.input.lt >= 0.12);
            snapshot.input.set_action("RightTrigger", snapshot.input.rt >= 0.12);

            for axis in &snapshot.axes {
                self.axis_values.insert(axis.name.clone(), axis.value);
            }
            snapshot.axes = self
                .axis_values
                .iter()
                .map(|(name, value)| SemanticAxisValue { name: name.clone(), value: *value })
                .collect();
            snapshot.input.axes.fill(0.0);
            for (index, axis) in snapshot.axes.iter().take(snapshot.input.axes.len()).enumerate() {
                snapshot.input.axes[index] = axis.value;
            }
            self.last_input = snapshot.input.clone();
            Some(snapshot)
        }
    }

    fn push_axis(snapshot: &mut SemanticHidSnapshot, slot: &mut usize, name: &str, value: f32) {
        if *slot < snapshot.input.axes.len() {
            snapshot.input.axes[*slot] = value;
            *slot += 1;
        }
        snapshot.axes.push(SemanticAxisValue { name: name.to_string(), value });
    }

    fn normalize_variable(var: &VariableField, value: i32, centered: bool) -> f32 {
        let min = i32::from(&var.logical_minimum) as f32;
        let max = i32::from(&var.logical_maximum) as f32;
        if !min.is_finite() || !max.is_finite() || (max - min).abs() < f32::EPSILON {
            return 0.0;
        }
        let unit = ((value as f32 - min) / (max - min)).clamp(0.0, 1.0);
        if centered { unit * 2.0 - 1.0 } else { unit }
    }

    fn usage_name(var: &VariableField) -> Option<&'static str> {
        let page = u16::from(var.usage.usage_page);
        let usage = u16::from(var.usage.usage_id);
        match page {
            0x01 => generic_desktop_axis_name(usage),
            0x02 => simulation_axis_name(usage),
            _ => None,
        }
    }

    pub use SemanticHidReader as Reader;
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    use anyhow::{anyhow, Result};

    pub struct Reader;
    impl Reader {
        pub fn open(_path: &str, _family: ControllerFamily, _class: DeviceClass) -> Result<Self> {
            Err(anyhow!("Semantic HID is Windows-only"))
        }
        pub fn semantic_names(&self) -> &[String] { &[] }
        pub fn refined_class(&self) -> DeviceClass { DeviceClass::Unknown }
        pub fn adaptive_trigger_available(&self) -> bool { false }
        pub fn nintendo_rumble_available(&self) -> bool { false }
        pub fn set_nintendo_hd_rumble(
            &mut self,
            _side: NintendoRumbleSide,
            _params: NintendoRumbleParams,
        ) -> Result<()> { Err(anyhow!("Nintendo HD rumble is Windows-only")) }
        pub fn stop_nintendo_hd_rumble(&mut self) -> Result<()> { Ok(()) }
        pub fn control_status(&self) -> &str { "Vendor outputs are Windows-only" }
        pub fn set_adaptive_triggers(
            &mut self,
            _side: AdaptiveTriggerSide,
            _mode: AdaptiveTriggerMode,
            _params: AdaptiveTriggerParams,
        ) -> Result<()> { Err(anyhow!("Adaptive triggers are Windows-only")) }
        pub fn clear_adaptive_triggers(&mut self) -> Result<()> { Ok(()) }
        pub fn read_latest(&mut self) -> Option<SemanticHidSnapshot> { None }
    }
}

pub use imp::Reader as SemanticHidReader;

pub fn refine_class(original: DeviceClass, names: &[String]) -> DeviceClass {
    let has = |needle: &str| names.iter().any(|name| name == needle);
    if has("Steering") {
        return DeviceClass::RacingWheel;
    }
    if has("Accelerator") || has("Brake") || has("Clutch") || has("Toe Brake") {
        if !has("Steering") {
            return DeviceClass::Pedals;
        }
    }
    if has("Throttle") && (has("Rudder") || has("Aileron") || has("Elevator")) {
        return DeviceClass::Hotas;
    }
    if has("Aileron") || has("Elevator") || has("Rudder") {
        return DeviceClass::FlightStick;
    }
    original
}

pub fn button_action_for_usage(family: ControllerFamily, usage_id: u16) -> Option<&'static str> {
    use ControllerFamily::*;
    match family {
        DualShock4 | DualSense | DualSenseEdge => match usage_id {
            1 => Some("West"),       // Square
            2 => Some("South"),      // Cross
            3 => Some("East"),       // Circle
            4 => Some("North"),      // Triangle
            5 => Some("LeftShoulder"),
            6 => Some("RightShoulder"),
            7 => Some("LeftTrigger"),
            8 => Some("RightTrigger"),
            9 => Some("Select"),
            10 => Some("Start"),
            11 => Some("LeftThumb"),
            12 => Some("RightThumb"),
            13 => Some("Guide"),
            14 => Some("Touchpad"),
            _ => None,
        },
        NintendoSwitchPro | NintendoJoyConLeft | NintendoJoyConRight => match usage_id {
            1 => Some("West"),       // Y
            2 => Some("South"),      // B
            3 => Some("East"),       // A
            4 => Some("North"),      // X
            5 => Some("LeftShoulder"),
            6 => Some("RightShoulder"),
            7 => Some("LeftTrigger"),
            8 => Some("RightTrigger"),
            9 => Some("Select"),     // Minus
            10 => Some("Start"),     // Plus
            11 => Some("LeftThumb"),
            12 => Some("RightThumb"),
            13 => Some("Guide"),     // Home
            _ => None,
        },
        _ => match usage_id {
            1 => Some("South"),
            2 => Some("East"),
            3 => Some("West"),
            4 => Some("North"),
            5 => Some("LeftShoulder"),
            6 => Some("RightShoulder"),
            7 => Some("Select"),
            8 => Some("Start"),
            9 => Some("LeftThumb"),
            10 => Some("RightThumb"),
            _ => None,
        },
    }
}

fn generic_desktop_axis_name(usage: u16) -> Option<&'static str> {
    Some(match usage {
        0x30 => "X",
        0x31 => "Y",
        0x32 => "Z",
        0x33 => "Rx",
        0x34 => "Ry",
        0x35 => "Rz",
        0x36 => "Slider",
        0x37 => "Dial",
        0x38 => "Wheel",
        _ => return None,
    })
}

fn generic_desktop_dpad_action(usage: u16) -> Option<&'static str> {
    match usage {
        0x90 => Some("DPadUp"),
        0x91 => Some("DPadDown"),
        0x92 => Some("DPadRight"),
        0x93 => Some("DPadLeft"),
        _ => None,
    }
}

fn simulation_axis_name(usage: u16) -> Option<&'static str> {
    Some(match usage {
        0xB0 => "Aileron",
        0xB1 => "Aileron Trim",
        0xB2 => "Anti-Torque",
        0xB5 => "Collective",
        0xB6 => "Dive Brake",
        0xB8 => "Elevator",
        0xB9 => "Elevator Trim",
        0xBA => "Rudder",
        0xBB => "Throttle",
        0xBF => "Toe Brake",
        0xC3 => "Wing Flaps",
        0xC4 => "Accelerator",
        0xC5 => "Brake",
        0xC6 => "Clutch",
        0xC7 => "Shifter",
        0xC8 => "Steering",
        0xC9 => "Turret Direction",
        0xCA => "Barrel Elevation",
        0xCB => "Dive Plane",
        0xCC => "Ballast",
        0xCD => "Bicycle Crank",
        0xCE => "Handle Bars",
        0xCF => "Front Brake",
        0xD0 => "Rear Brake",
        _ => return None,
    })
}

fn is_centered_axis(page: u16, usage: u16, class: DeviceClass) -> bool {
    if page != 0x01 {
        return false;
    }
    match class {
        DeviceClass::Gamepad => matches!(usage, 0x30 | 0x31 | 0x33 | 0x34),
        DeviceClass::FlightStick | DeviceClass::Hotas => matches!(usage, 0x30 | 0x31 | 0x32 | 0x33 | 0x34 | 0x35),
        DeviceClass::RacingWheel => matches!(usage, 0x30 | 0x31 | 0x32 | 0x33 | 0x34 | 0x35),
        _ => matches!(usage, 0x30 | 0x31 | 0x33 | 0x34),
    }
}

fn apply_axis_to_standard_input(input: &mut InputSnapshot, name: &str, value: f32, class: DeviceClass) {
    match (class, name) {
        (DeviceClass::Gamepad, "X") => input.lx = value,
        (DeviceClass::Gamepad, "Y") => input.ly = value,
        (DeviceClass::Gamepad, "Rx") => input.rx = value,
        (DeviceClass::Gamepad, "Ry") => input.ry = value,
        (DeviceClass::Gamepad, "Z") => input.lt = value.clamp(0.0, 1.0),
        (DeviceClass::Gamepad, "Rz") => input.rt = value.clamp(0.0, 1.0),
        (_, "Accelerator") => input.lt = value.clamp(0.0, 1.0),
        (_, "Brake") => input.rt = value.clamp(0.0, 1.0),
        (DeviceClass::RacingWheel, "Steering") => input.lx = value,
        (DeviceClass::FlightStick | DeviceClass::Hotas, "X" | "Aileron") => input.lx = value,
        (DeviceClass::FlightStick | DeviceClass::Hotas, "Y" | "Elevator") => input.ly = value,
        (DeviceClass::FlightStick | DeviceClass::Hotas, "Rudder") => input.rx = value,
        (DeviceClass::FlightStick | DeviceClass::Hotas, "Throttle") => input.lt = value.clamp(0.0, 1.0),
        _ => {}
    }
}

fn apply_hat(input: &mut InputSnapshot, value: i32) {
    // HID hats are commonly 0..7 clockwise starting at Up, with any other value
    // representing neutral/null. Diagonal values activate two directions.
    let (up, right, down, left) = match value {
        0 => (true, false, false, false),
        1 => (true, true, false, false),
        2 => (false, true, false, false),
        3 => (false, true, true, false),
        4 => (false, false, true, false),
        5 => (false, false, true, true),
        6 => (false, false, false, true),
        7 => (true, false, false, true),
        _ => (false, false, false, false),
    };
    input.set_action("DPadUp", up);
    input.set_action("DPadRight", right);
    input.set_action("DPadDown", down);
    input.set_action("DPadLeft", left);
}
