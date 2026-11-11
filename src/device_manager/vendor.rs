use super::model::ControllerFamily;

#[derive(Debug, Clone)]
pub struct TouchPoint {
    pub active: bool,
    pub id: u8,
    pub x: u16,
    pub y: u16,
    pub max_x: u16,
    pub max_y: u16,
}

impl Default for TouchPoint {
    fn default() -> Self {
        Self { active: false, id: 0, x: 0, y: 0, max_x: 1, max_y: 1 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct VendorTelemetry {
    pub protocol: String,
    pub battery_percent: Option<u8>,
    pub charging: Option<bool>,
    pub battery_state: String,
    /// Raw motion sensor samples are always retained for diagnostics.
    pub gyro: Option<[i16; 3]>,
    pub accel: Option<[i16; 3]>,
    /// Physical-unit presentation when the controller protocol has a documented
    /// nominal scale. The UI labels whether the values are nominal or calibrated.
    pub gyro_dps: Option<[f32; 3]>,
    pub accel_g: Option<[f32; 3]>,
    pub motion_units: String,
    pub power_source: String,
    pub touch: [TouchPoint; 2],
    pub firmware_detail: String,
}

impl VendorTelemetry {
    pub fn has_motion(&self) -> bool { self.gyro.is_some() || self.accel.is_some() }
    pub fn has_touch(&self) -> bool { self.touch.iter().any(|p| p.active) }

    pub fn power_text(&self) -> Option<String> {
        let percent = self.battery_percent?;
        let state = if self.battery_state.is_empty() {
            match self.charging {
                Some(true) => "charging",
                Some(false) => "discharging",
                None => "battery",
            }
        } else {
            self.battery_state.as_str()
        };
        Some(format!("{percent}% • {state}"))
    }
}

pub fn parse_report(family: ControllerFamily, report: &[u8]) -> VendorTelemetry {
    match family {
        ControllerFamily::DualSense | ControllerFamily::DualSenseEdge => parse_dualsense(report),
        ControllerFamily::DualShock4 => parse_dualshock4(report),
        ControllerFamily::NintendoSwitchPro | ControllerFamily::NintendoJoyConLeft | ControllerFamily::NintendoJoyConRight => parse_nintendo(report),
        _ => VendorTelemetry::default(),
    }
}

fn parse_dualsense(report: &[u8]) -> VendorTelemetry {
    // USB: report 0x01, 64 bytes, common payload starts at +1.
    // Bluetooth: report 0x31, 78 bytes, common payload starts at +2.
    let base = match report.first().copied() {
        Some(0x01) if report.len() >= 64 => 1usize,
        Some(0x31) if report.len() >= 65 => 2usize,
        _ => return VendorTelemetry::default(),
    };
    if report.len() < base + 53 { return VendorTelemetry::default(); }

    let gyro = read_i16x3(report, base + 15);
    let accel = read_i16x3(report, base + 21);
    let status = report[base + 52];
    let level = status & 0x0f;
    let charge_code = (status >> 4) & 0x0f;
    let (battery_percent, charging, battery_state) = match charge_code {
        0x0 => (Some((level.saturating_mul(10).saturating_add(5)).min(100)), Some(false), "discharging"),
        0x1 => (Some((level.saturating_mul(10).saturating_add(5)).min(100)), Some(true), "charging"),
        0x2 => (Some(100), Some(false), "full"),
        0xA | 0xB => (None, Some(false), "not charging"),
        _ => (None, None, "unknown"),
    };

    let mut touch = [TouchPoint::default(), TouchPoint::default()];
    for (i, out) in touch.iter_mut().enumerate() {
        let off = base + 32 + i * 4;
        if off + 3 >= report.len() { continue; }
        *out = decode_touch(&report[off..off + 4], 1920, 1080);
    }

    VendorTelemetry {
        protocol: if report[0] == 0x31 { "DualSense Bluetooth enhanced" } else { "DualSense USB enhanced" }.to_string(),
        battery_percent,
        charging,
        battery_state: battery_state.to_string(),
        gyro,
        accel,
        touch,
        gyro_dps: gyro.map(|v| [v[0] as f32 / 1024.0, v[1] as f32 / 1024.0, v[2] as f32 / 1024.0]),
        accel_g: accel.map(|v| [v[0] as f32 / 8192.0, v[1] as f32 / 8192.0, v[2] as f32 / 8192.0]),
        motion_units: "Sony nominal IMU scale • dps / g • raw retained".to_string(),
        power_source: "Sony enhanced input report".to_string(),
        firmware_detail: String::new(),
    }
}

fn parse_dualshock4(report: &[u8]) -> VendorTelemetry {
    // USB common payload starts at +1. Full Bluetooth report 0x11 has two
    // transport bytes before the common payload.
    let base = match report.first().copied() {
        Some(0x01) if report.len() >= 64 => 1usize,
        Some(0x11) if report.len() >= 40 => 3usize,
        _ => return VendorTelemetry::default(),
    };
    if report.len() < base + 32 { return VendorTelemetry::default(); }
    let gyro = read_i16x3(report, base + 12);
    let accel = read_i16x3(report, base + 18);
    let status0 = report[base + 29];
    let level = status0 & 0x0f;
    let wired = (status0 & 0x10) != 0;
    let percent = if wired && level >= 11 { 100 } else { (level.min(10) * 10).min(100) };

    let mut touch = [TouchPoint::default(), TouchPoint::default()];
    let touch_count_off = base + 32;
    if report.len() > touch_count_off && report[touch_count_off] > 0 {
        // Each touch report starts with one timestamp byte followed by two points.
        let point_base = touch_count_off + 2;
        for (i, out) in touch.iter_mut().enumerate() {
            let off = point_base + i * 4;
            if off + 3 < report.len() {
                *out = decode_touch(&report[off..off + 4], 1920, 942);
            }
        }
    }

    VendorTelemetry {
        protocol: if report[0] == 0x11 { "DualShock 4 Bluetooth enhanced" } else { "DualShock 4 USB enhanced" }.to_string(),
        battery_percent: Some(percent),
        charging: Some(wired),
        battery_state: if wired { if percent >= 100 { "full" } else { "charging" } } else { "discharging" }.to_string(),
        gyro,
        accel,
        touch,
        gyro_dps: gyro.map(|v| [v[0] as f32 / 1024.0, v[1] as f32 / 1024.0, v[2] as f32 / 1024.0]),
        accel_g: accel.map(|v| [v[0] as f32 / 8192.0, v[1] as f32 / 8192.0, v[2] as f32 / 8192.0]),
        motion_units: "DualShock 4 nominal IMU scale • dps / g • raw retained".to_string(),
        power_source: "DualShock 4 enhanced input report".to_string(),
        firmware_detail: String::new(),
    }
}

fn parse_nintendo(report: &[u8]) -> VendorTelemetry {
    if report.first().copied() != Some(0x30) || report.len() < 49 {
        return VendorTelemetry::default();
    }
    let bat = report[2];
    // Nintendo encodes battery capacity in bits 7..5 as 0..4
    // (critical/low/medium/high/full); bit 4 indicates charging.
    let raw_level = (bat >> 5) & 0x07;
    let percent = ((raw_level.min(4) as u16 * 100) / 4) as u8;
    let charging = (bat & 0x10) != 0;

    // Full input mode contains three 12-byte IMU samples. Display the newest
    // sample; all three remain visible in the raw report inspector.
    let imu = 13 + 24;
    let accel = read_i16x3(report, imu);
    let gyro = read_i16x3(report, imu + 6);

    VendorTelemetry {
        protocol: "Nintendo full report 0x30".to_string(),
        battery_percent: Some(percent),
        charging: Some(charging),
        battery_state: if charging { "charging" } else { "discharging" }.to_string(),
        gyro,
        accel,
        touch: [TouchPoint::default(), TouchPoint::default()],
        gyro_dps: gyro.map(|v| [v[0] as f32 * 0.06103, v[1] as f32 * 0.06103, v[2] as f32 * 0.06103]),
        accel_g: accel.map(|v| [v[0] as f32 * 0.000244, v[1] as f32 * 0.000244, v[2] as f32 * 0.000244]),
        motion_units: "Nintendo full-report nominal scale • dps / g • raw retained".to_string(),
        power_source: "Nintendo full report 0x30".to_string(),
        firmware_detail: String::new(),
    }
}

fn decode_touch(bytes: &[u8], max_x: u16, max_y: u16) -> TouchPoint {
    let contact = bytes[0];
    let x = bytes[1] as u16 | (((bytes[2] & 0x0f) as u16) << 8);
    let y = ((bytes[2] as u16) >> 4) | ((bytes[3] as u16) << 4);
    TouchPoint {
        active: (contact & 0x80) == 0,
        id: contact & 0x7f,
        x,
        y,
        max_x,
        max_y,
    }
}

fn read_i16x3(bytes: &[u8], offset: usize) -> Option<[i16; 3]> {
    if bytes.len() < offset + 6 { return None; }
    Some([
        i16::from_le_bytes([bytes[offset], bytes[offset + 1]]),
        i16::from_le_bytes([bytes[offset + 2], bytes[offset + 3]]),
        i16::from_le_bytes([bytes[offset + 4], bytes[offset + 5]]),
    ])
}



#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NintendoRumbleSide {
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy)]
pub struct NintendoRumbleParams {
    pub low_frequency_hz: f32,
    pub high_frequency_hz: f32,
    pub amplitude: f32,
}

impl Default for NintendoRumbleParams {
    fn default() -> Self {
        Self { low_frequency_hz: 160.0, high_frequency_hz: 320.0, amplitude: 0.35 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveTriggerSide {
    Left,
    Right,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdaptiveTriggerMode {
    Off,
    Feedback,
    Weapon,
    Vibration,
}

#[derive(Debug, Clone, Copy)]
pub struct AdaptiveTriggerParams {
    /// Normalized start position. 0.0 is the top of the trigger travel.
    pub start: f32,
    /// Normalized end position used by Weapon mode.
    pub end: f32,
    /// Normalized 0..1 strength/intensity.
    pub strength: f32,
    /// Raw-ish vibration frequency normalized by the UI into 1..255.
    pub frequency: u8,
}

impl Default for AdaptiveTriggerParams {
    fn default() -> Self {
        Self { start: 0.25, end: 0.70, strength: 0.70, frequency: 110 }
    }
}

#[cfg(windows)]
pub fn dualsense_set_adaptive_triggers(
    device: &hidapi::HidDevice,
    bus: hidapi::BusType,
    sequence: &mut u8,
    side: AdaptiveTriggerSide,
    mode: AdaptiveTriggerMode,
    params: AdaptiveTriggerParams,
) -> anyhow::Result<()> {
    use anyhow::{bail, Context};

    let params = AdaptiveTriggerParams {
        start: params.start.clamp(0.0, 1.0),
        end: params.end.clamp(0.0, 1.0),
        strength: params.strength.clamp(0.0, 1.0),
        frequency: params.frequency.max(1),
    };

    let mut common = [0u8; 47];
    let enable_right = matches!(side, AdaptiveTriggerSide::Right | AdaptiveTriggerSide::Both);
    let enable_left = matches!(side, AdaptiveTriggerSide::Left | AdaptiveTriggerSide::Both);

    // Common-output offsets are equivalent to the Windows GameInput DualSense
    // helper's USB absolute offsets minus the one-byte report ID. The trigger
    // effect blocks are 8 bytes and are 11 bytes apart.
    const RIGHT_TRIGGER_OFFSET: usize = 10;
    const LEFT_TRIGGER_OFFSET: usize = 21;
    const RIGHT_TRIGGER_FLAG: u8 = 0x04;
    const LEFT_TRIGGER_FLAG: u8 = 0x08;

    if enable_right {
        common[0] |= RIGHT_TRIGGER_FLAG;
        encode_trigger_effect(&mut common[RIGHT_TRIGGER_OFFSET..RIGHT_TRIGGER_OFFSET + 8], mode, params);
    }
    if enable_left {
        common[0] |= LEFT_TRIGGER_FLAG;
        encode_trigger_effect(&mut common[LEFT_TRIGGER_OFFSET..LEFT_TRIGGER_OFFSET + 8], mode, params);
    }

    match bus {
        hidapi::BusType::Bluetooth => {
            // DualSense Bluetooth output: report id, sequence/tag, 0x10 tag,
            // 47-byte common payload, 24 reserved bytes, CRC32.
            let mut report = [0u8; 78];
            report[0] = 0x31;
            report[1] = (*sequence & 0x0f) << 4;
            report[2] = 0x10;
            report[3..50].copy_from_slice(&common);
            *sequence = (*sequence).wrapping_add(1) & 0x0f;

            let mut crc_data = Vec::with_capacity(75);
            crc_data.push(0xA2); // PlayStation output CRC seed byte.
            crc_data.extend_from_slice(&report[..74]);
            let crc = crc32fast::hash(&crc_data);
            report[74..78].copy_from_slice(&crc.to_le_bytes());
            send_hid_output(device, &report).context("DualSense Bluetooth adaptive-trigger output failed")?;
        }
        hidapi::BusType::Usb | hidapi::BusType::Unknown | hidapi::BusType::I2c | hidapi::BusType::Spi => {
            // Microsoft's GameInput companion helper uses a 64-byte USB raw
            // output report beginning with report ID 0x02. Linux exposes a
            // 63-byte transport payload on some stacks; try 64 first and then
            // 63 as a HIDAPI compatibility fallback.
            let mut report = [0u8; 64];
            report[0] = 0x02;
            report[1..48].copy_from_slice(&common);
            match send_hid_output(device, &report) {
                Ok(()) => {}
                Err(first) => {
                    if let Err(second) = send_hid_output(device, &report[..63]) {
                        bail!("DualSense USB adaptive-trigger output failed: {first}; 63-byte fallback: {second}");
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn encode_trigger_effect(block: &mut [u8], mode: AdaptiveTriggerMode, params: AdaptiveTriggerParams) {
    block.fill(0);
    match mode {
        AdaptiveTriggerMode::Off => {}
        AdaptiveTriggerMode::Feedback => {
            block[0] = 0x01;
            block[1] = normalized_byte(params.start, 255);
            block[2] = normalized_byte(params.strength, 255);
        }
        AdaptiveTriggerMode::Weapon => {
            block[0] = 0x02;
            let start = normalized_byte(params.start, 255);
            let end = normalized_byte(params.end.max(params.start), 255);
            block[1] = start;
            block[2] = end;
            block[3] = normalized_byte(params.strength, 255);
        }
        AdaptiveTriggerMode::Vibration => {
            block[0] = 0x06;
            block[1] = params.frequency;
            // The documented byte is resistance where 0 means high resistance
            // and 255 means low resistance, hence the inversion from UI strength.
            block[2] = 255u8.saturating_sub(normalized_byte(params.strength, 255));
            block[3] = normalized_byte(params.start, 147);
        }
    }
}

#[cfg(windows)]
fn normalized_byte(value: f32, maximum: u8) -> u8 {
    (value.clamp(0.0, 1.0) * maximum as f32).round() as u8
}

#[cfg(windows)]
fn send_hid_output(device: &hidapi::HidDevice, data: &[u8]) -> anyhow::Result<()> {
    use anyhow::Context;
    match device.write(data) {
        Ok(_) => Ok(()),
        Err(write_error) => device
            .send_output_report(data)
            .with_context(|| format!("HID write failed ({write_error}); Set_Report fallback also failed")),
    }
}

#[cfg(windows)]
pub fn initialize_nintendo_controller(device: &hidapi::HidDevice, bus: hidapi::BusType) -> String {
    let mut notes = Vec::<String>::new();

    // Wired Switch controllers need the documented USB handshake before the
    // normal 0x01/0x21 subcommand path is guaranteed to be active. A controller
    // already initialized by another driver may reject or simply ignore these;
    // that is not fatal, so the sequence is intentionally best-effort.
    if matches!(bus, hidapi::BusType::Usb) {
        match nintendo_usb_command(device, 0x02, true) {
            Ok(()) => {
                notes.push("USB handshake".to_string());
                let _ = nintendo_usb_command(device, 0x03, true); // 3 Mbit baud rate.
                let _ = nintendo_usb_command(device, 0x02, true); // handshake again.
                let _ = nintendo_usb_command(device, 0x04, false); // disable USB timeout; no reply expected.
            }
            Err(err) => notes.push(format!("USB handshake skipped ({err})")),
        }
    }

    let mut seq = 0u8;
    let report_mode = nintendo_subcommand(device, &mut seq, 0x03, &[0x30]);
    let imu = nintendo_subcommand(device, &mut seq, 0x40, &[0x01]);
    let rumble = nintendo_subcommand(device, &mut seq, 0x48, &[0x01]);

    notes.push(match report_mode {
        Ok(()) => "full report 0x30".to_string(),
        Err(err) => format!("report-mode init failed ({err})"),
    });
    notes.push(match imu {
        Ok(()) => "IMU enabled".to_string(),
        Err(err) => format!("IMU enable failed ({err})"),
    });
    notes.push(match rumble {
        Ok(()) => "HD-rumble channel enabled".to_string(),
        Err(err) => format!("rumble enable failed ({err})"),
    });

    format!("Nintendo init • {}", notes.join(" • "))
}

#[cfg(windows)]
fn nintendo_usb_command(device: &hidapi::HidDevice, command: u8, wait_reply: bool) -> anyhow::Result<()> {
    use anyhow::{bail, Context};

    // The kernel protocol uses a compact two-byte USB command. Some Windows
    // HID stacks insist on the declared output-report length, so keep a padded
    // 64-byte compatibility fallback rather than assuming one transport shape.
    let compact = [0x80u8, command];
    if send_hid_output(device, &compact).is_err() {
        let mut padded = [0u8; 64];
        padded[..2].copy_from_slice(&compact);
        send_hid_output(device, &padded)
            .with_context(|| format!("USB command 0x{command:02X} write failed"))?;
    }
    if !wait_reply {
        return Ok(());
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(220);
    let mut buffer = [0u8; 128];
    while std::time::Instant::now() < deadline {
        match device.read_timeout(&mut buffer, 15) {
            Ok(len) if len >= 2 && buffer[0] == 0x81 && buffer[1] == command => return Ok(()),
            Ok(_) => {}
            Err(err) => bail!("USB command 0x{command:02X} reply read failed: {err}"),
        }
    }
    bail!("USB command 0x{command:02X} reply timed out")
}

#[cfg(windows)]
fn nintendo_subcommand(
    device: &hidapi::HidDevice,
    sequence: &mut u8,
    subcommand: u8,
    payload: &[u8],
) -> anyhow::Result<()> {
    use anyhow::{bail, Context};
    if payload.len() > 53 {
        bail!("Nintendo subcommand payload too large");
    }

    // OUTPUT 0x01 = packet counter + 8 rumble bytes + subcommand + payload.
    // Linux sends only the used bytes; some Windows HID stacks require a fixed
    // report length. Try compact first, then common 49/64-byte padded shapes.
    let used_len = 11 + payload.len();
    let mut report = vec![0u8; used_len];
    report[0] = 0x01;
    report[1] = *sequence & 0x0f;
    *sequence = (*sequence).wrapping_add(1) & 0x0f;
    // Neutral HD-rumble encoding for left + right actuator.
    report[2..10].copy_from_slice(&[0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40]);
    report[10] = subcommand;
    report[11..11 + payload.len()].copy_from_slice(payload);

    let write_result = send_hid_output(device, &report).or_else(|_| {
        let mut padded = vec![0u8; 49];
        padded[..report.len()].copy_from_slice(&report);
        send_hid_output(device, &padded)
    }).or_else(|_| {
        let mut padded = vec![0u8; 64];
        padded[..report.len()].copy_from_slice(&report);
        send_hid_output(device, &padded)
    });
    write_result.with_context(|| format!("Nintendo subcommand 0x{subcommand:02X} write failed"))?;

    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(260);
    let mut buffer = [0u8; 128];
    while std::time::Instant::now() < deadline {
        match device.read_timeout(&mut buffer, 15) {
            Ok(len) if len >= 15 && buffer[0] == 0x21 && buffer[14] == subcommand => {
                if buffer[13] & 0x80 != 0 {
                    return Ok(());
                }
                bail!("Nintendo subcommand 0x{subcommand:02X} was NACKed (0x{:02X})", buffer[13]);
            }
            Ok(_) => {}
            Err(err) => bail!("Nintendo subcommand 0x{subcommand:02X} reply read failed: {err}"),
        }
    }
    bail!("Nintendo subcommand 0x{subcommand:02X} reply timed out")
}

#[cfg(windows)]
pub fn nintendo_set_hd_rumble(
    device: &hidapi::HidDevice,
    sequence: &mut u8,
    side: NintendoRumbleSide,
    params: NintendoRumbleParams,
) -> anyhow::Result<()> {
    use anyhow::Context;
    let amp = params.amplitude.clamp(0.0, 1.0);
    let left = encode_nintendo_rumble(
        params.low_frequency_hz,
        params.high_frequency_hz,
        if matches!(side, NintendoRumbleSide::Left | NintendoRumbleSide::Both) { amp } else { 0.0 },
    );
    let right = encode_nintendo_rumble(
        params.low_frequency_hz,
        params.high_frequency_hz,
        if matches!(side, NintendoRumbleSide::Right | NintendoRumbleSide::Both) { amp } else { 0.0 },
    );

    // OUTPUT 0x10 is rumble-only: report id, packet counter, 4 bytes left, 4 bytes right.
    let mut report = [0u8; 10];
    report[0] = 0x10;
    report[1] = *sequence & 0x0f;
    *sequence = (*sequence).wrapping_add(1) & 0x0f;
    report[2..6].copy_from_slice(&left);
    report[6..10].copy_from_slice(&right);

    send_hid_output(device, &report).or_else(|_| {
        let mut padded = [0u8; 49];
        padded[..report.len()].copy_from_slice(&report);
        send_hid_output(device, &padded)
    }).or_else(|_| {
        let mut padded = [0u8; 64];
        padded[..report.len()].copy_from_slice(&report);
        send_hid_output(device, &padded)
    }).context("Nintendo HD-rumble output failed")
}

#[cfg(windows)]
pub fn nintendo_stop_hd_rumble(
    device: &hidapi::HidDevice,
    sequence: &mut u8,
) -> anyhow::Result<()> {
    nintendo_set_hd_rumble(
        device,
        sequence,
        NintendoRumbleSide::Both,
        NintendoRumbleParams { amplitude: 0.0, ..Default::default() },
    )
}

fn encode_nintendo_rumble(low_frequency_hz: f32, high_frequency_hz: f32, amplitude: f32) -> [u8; 4] {
    if amplitude <= 0.0001 {
        return [0x00, 0x01, 0x40, 0x40];
    }

    // Encoding follows the public Switch HID reverse-engineering formula.
    // Amplitude is deliberately clamped to 1.0, below the unsafe actuator range.
    let high = high_frequency_hz.clamp(81.75, 1252.0);
    let low = low_frequency_hz.clamp(40.87, 626.0);
    let amp = amplitude.clamp(0.0, 1.0);

    let high_code = ((high / 10.0).log2() * 32.0).round() as i32;
    let low_code = ((low / 10.0).log2() * 32.0).round() as i32;
    let hf = ((high_code - 0x60).clamp(0, 0x7f) as u16) * 4;
    let lf = (low_code - 0x40).clamp(1, 0x7f) as u8;

    let amp_code = if amp > 0.23 {
        ((amp * 8.7).log2() * 32.0).round()
    } else if amp > 0.12 {
        ((amp * 17.0).log2() * 16.0).round()
    } else if amp > 0.0 {
        // The low-amplitude region is quantized more aggressively. This keeps
        // the output in the documented safe table while still allowing subtle effects.
        (amp / 0.12 * 31.0).round()
    } else {
        0.0
    }.clamp(0.0, 0x64 as f32) as u8;

    let hf_amp = amp_code.saturating_mul(2).min(0xC8);
    let lf_amp = (amp_code / 2).saturating_add(0x40).min(0x72);

    [
        (hf & 0xff) as u8,
        hf_amp.saturating_add(((hf >> 8) & 0xff) as u8),
        lf,
        lf_amp,
    ]
}

#[cfg(windows)]
pub fn read_firmware_feature(device: &hidapi::HidDevice, family: ControllerFamily) -> String {
    match family {
        ControllerFamily::DualSense | ControllerFamily::DualSenseEdge => {
            let mut buf = [0u8; 64];
            buf[0] = 0x20;
            let Ok(len) = device.get_feature_report(&mut buf) else { return String::new(); };
            if len < 60 || buf[0] != 0x20 { return String::new(); }
            let build_date = printable_ascii(&buf[1..12]);
            let build_time = printable_ascii(&buf[12..20]);
            let fw_type = u16::from_le_bytes([buf[20], buf[21]]);
            let sw_series = u16::from_le_bytes([buf[22], buf[23]]);
            let hardware = u32::from_le_bytes([buf[24], buf[25], buf[26], buf[27]]);
            let firmware = u32::from_le_bytes([buf[28], buf[29], buf[30], buf[31]]);
            let update = u16::from_le_bytes([buf[44], buf[45]]);
            format!(
                "FW 0x{firmware:08X} • update 0x{update:04X} • type 0x{fw_type:04X} • series 0x{sw_series:04X} • HW 0x{hardware:08X} • {build_date} {build_time}"
            )
        }
        // DS4 feature report 0xA3 is retained as a capability probe for now;
        // its board-specific formatting differs across generations. Keep the
        // raw identity concise instead of inventing a semantic version.
        ControllerFamily::DualShock4 => {
            let mut buf = [0u8; 49];
            buf[0] = 0xA3;
            match device.get_feature_report(&mut buf) {
                Ok(len) if len >= 16 && buf[0] == 0xA3 => {
                    format!("DS4 firmware feature A3 • {} bytes", len)
                }
                _ => String::new(),
            }
        }
        ControllerFamily::NintendoSwitchPro | ControllerFamily::NintendoJoyConLeft | ControllerFamily::NintendoJoyConRight => {
            "Nintendo runtime protocol active • firmware identity available through HID release/device-info fallback".to_string()
        }
        ControllerFamily::Xbox360 | ControllerFamily::XboxOne | ControllerFamily::XboxSeries =>
            "Xbox firmware is not exposed by XInput/HID generically • HID release retained as identity".to_string(),
        ControllerFamily::Logitech | ControllerFamily::Thrustmaster | ControllerFamily::Fanatec |
        ControllerFamily::EightBitDo | ControllerFamily::Razer | ControllerFamily::Hori |
        ControllerFamily::GameSir | ControllerFamily::SteamController | ControllerFamily::SteamDeck |
        ControllerFamily::Generic | ControllerFamily::Unknown =>
            "Vendor firmware feature not standardized • HID release retained as identity".to_string(),
    }
}

#[cfg(not(windows))]
pub fn read_firmware_feature(_device: &(), _family: ControllerFamily) -> String { String::new() }

fn printable_ascii(bytes: &[u8]) -> String {
    bytes.iter().copied().filter(|b| b.is_ascii_graphic() || *b == b' ').map(char::from).collect::<String>().trim().to_string()
}
