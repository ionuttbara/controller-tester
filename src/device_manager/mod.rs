pub mod classifier;
pub mod hid;
pub mod model;
pub mod semantic_hid;
pub mod vendor;

use gilrs::Gilrs;
use model::{power_to_text, transport_from_power_and_name, DeviceDescriptor, HidMetadata, Transport};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

pub fn scan_controllers(gilrs: &Gilrs) -> Vec<DeviceDescriptor> {
    let hid_devices = hid::scan_hid_devices();
    let mut result = Vec::new();
    let mut matched_hid_indices = HashSet::new();
    let mut stable_keys = HashSet::new();

    for (id, gamepad) in gilrs.gamepads() {
        let name = gamepad.name().to_string();
        let os_name = gamepad.os_name();
        let best_hid_match = find_best_hid_match(os_name, gamepad.vendor_id(), gamepad.product_id(), &hid_devices, &matched_hid_indices);
        let best_hid = best_hid_match.map(|(index, hid)| {
            matched_hid_indices.insert(index);
            hid
        });
        let (family, class) = classifier::classify(&name, best_hid);
        let power = gamepad.power_info();
        let fallback_transport = transport_from_power_and_name(&power, &name);
        let transport = best_hid
            .map(|h| h.transport)
            .filter(|t| *t != Transport::Unknown)
            .unwrap_or(fallback_transport);
        let uuid = gamepad
            .uuid()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        let (manufacturer, vendor_id, product_id, release_number, serial, firmware, hid_path, usage_page, usage, interface_number) =
            if let Some(hid) = best_hid {
                (
                    nonempty(&hid.manufacturer, vendor_name(hid.vendor_id)).to_string(),
                    Some(hid.vendor_id),
                    Some(hid.product_id),
                    Some(hid.release_number),
                    hid.serial.clone(),
                    format_release(hid.release_number),
                    hid.path.clone(),
                    Some(hid.usage_page),
                    Some(hid.usage),
                    Some(hid.interface_number),
                )
            } else {
                (
                    gamepad.vendor_id().map(vendor_name).unwrap_or("Unknown").to_string(),
                    gamepad.vendor_id(),
                    gamepad.product_id(),
                    None,
                    String::new(),
                    "Not exposed".to_string(),
                    String::new(),
                    None,
                    None,
                    None,
                )
            };

        let stable_key = make_stable_key(vendor_id, product_id, &serial, &uuid);
        stable_keys.insert(stable_key.clone());
        result.push(DeviceDescriptor {
            id: usize::from(id) as i32,
            display_name: format!("{} [{}]", name, family.description()),
            name,
            uuid,
            manufacturer,
            vendor_id,
            product_id,
            release_number,
            serial,
            firmware,
            hid_path,
            usage_page,
            usage,
            interface_number,
            class,
            family,
            transport,
            power_status: power_to_text(&power),
            layout: family.layout().to_string(),
            artwork: family.artwork_key(class).to_string(),
            ff_supported: gamepad.is_ff_supported(),
            stable_key,
            backend: if best_hid.is_some() { "gilrs + HID".to_string() } else { "gilrs".to_string() },
        });
    }

    // Add relevant HID controller collections that gilrs did not expose. This is
    // particularly useful for wheels, HOTAS, pedals and simulation panels.
    for (index, hid) in hid_devices.iter().enumerate() {
        if matched_hid_indices.contains(&index) || !is_controller_hid(hid) {
            continue;
        }
        let product = if hid.product.trim().is_empty() {
            format!("HID {:04X}:{:04X}", hid.vendor_id, hid.product_id)
        } else {
            hid.product.clone()
        };
        let (family, class) = classifier::classify(&product, Some(hid));
        let stable_key = make_hid_stable_key(hid);
        if !stable_keys.insert(stable_key.clone()) {
            continue;
        }
        result.push(DeviceDescriptor {
            id: synthetic_hid_id(&stable_key),
            display_name: format!("{} [{}]", product, class.as_str()),
            name: product,
            uuid: String::new(),
            manufacturer: nonempty(&hid.manufacturer, vendor_name(hid.vendor_id)).to_string(),
            vendor_id: Some(hid.vendor_id),
            product_id: Some(hid.product_id),
            release_number: Some(hid.release_number),
            serial: hid.serial.clone(),
            firmware: format_release(hid.release_number),
            hid_path: hid.path.clone(),
            usage_page: Some(hid.usage_page),
            usage: Some(hid.usage),
            interface_number: Some(hid.interface_number),
            class,
            family,
            transport: hid.transport,
            power_status: "Power: Not exposed".to_string(),
            layout: family.layout().to_string(),
            artwork: family.artwork_key(class).to_string(),
            ff_supported: false,
            stable_key,
            backend: "HID semantic".to_string(),
        });
    }

    result.sort_by_key(|d| (d.id >= 100_000, d.id));
    result
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn find_best_hid_match<'a>(
    gamepad_name: &str,
    vendor_id: Option<u16>,
    product_id: Option<u16>,
    devices: &'a [HidMetadata],
    excluded: &HashSet<usize>,
) -> Option<(usize, &'a HidMetadata)> {
    let wanted = normalize_name(gamepad_name);
    let mut best: Option<(usize, &HidMetadata, i32)> = None;

    for (index, hid) in devices.iter().enumerate() {
        if excluded.contains(&index) {
            continue;
        }
        if hid.usage_page == 0x01 && matches!(hid.usage, 0x02 | 0x06) {
            continue;
        }
        let product = normalize_name(&hid.product);
        let mut score = 0;
        if vendor_id == Some(hid.vendor_id) && product_id == Some(hid.product_id) {
            score += 150;
        }
        if !product.is_empty() && product == wanted {
            score += 100;
        }
        if !product.is_empty() && (product.contains(&wanted) || wanted.contains(&product)) {
            score += 55;
        }
        for token in [
            "xbox", "dualshock", "dualsense", "logitech", "thrustmaster", "fanatec",
            "8bitdo", "nintendo", "gamesir", "razer", "hori",
        ] {
            if wanted.contains(token) && product.contains(token) {
                score += 30;
            }
        }
        if hid.usage_page == 0x01 && matches!(hid.usage, 0x04 | 0x05 | 0x08) {
            score += 10;
        }

        if score >= 40 && best.as_ref().map(|(_, _, old)| score > *old).unwrap_or(true) {
            best = Some((index, hid, score));
        }
    }
    best.map(|(index, hid, _)| (index, hid))
}

fn is_controller_hid(hid: &HidMetadata) -> bool {
    if hid.usage_page == 0x01 && matches!(hid.usage, 0x04 | 0x05 | 0x08) {
        return true;
    }
    // HID Simulation Controls top-level collections. Some wheels, pedals and
    // flight hardware enumerate here instead of under Generic Desktop.
    if hid.usage_page == 0x02 && (matches!(hid.usage, 0x01..=0x0C) || matches!(hid.usage, 0x20..=0x25)) {
        return true;
    }
    let name = hid.product.to_ascii_lowercase();
    [
        "controller", "gamepad", "joystick", "wheel", "pedal", "hotas", "throttle",
        "flight", "farm", "side panel", "button box", "fight stick", "arcade",
    ]
    .iter()
    .any(|token| name.contains(token))
}

fn nonempty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.trim().is_empty() { fallback } else { value }
}

fn vendor_name(vid: u16) -> &'static str {
    match vid {
        0x045E => "Microsoft",
        0x054C => "Sony Interactive Entertainment",
        0x057E => "Nintendo",
        0x046D => "Logitech",
        0x044F => "Thrustmaster",
        0x0EB7 => "Fanatec",
        0x2DC8 => "8BitDo",
        0x1532 => "Razer",
        0x0F0D => "HORI",
        0x28DE => "Valve",
        0x3537 => "GameSir",
        _ => "Unknown",
    }
}

fn format_release(release: u16) -> String {
    if release == 0 {
        "Not exposed".to_string()
    } else {
        format!("HID release {:x}.{:02x}", release >> 8, release & 0xff)
    }
}

fn make_stable_key(vid: Option<u16>, pid: Option<u16>, serial: &str, uuid: &str) -> String {
    let mut identity = match (vid, pid) {
        (Some(v), Some(p)) => format!("VID_{v:04X}&PID_{p:04X}"),
        _ => "GENERIC".to_string(),
    };
    if !serial.trim().is_empty() {
        identity.push_str("&SER_");
        identity.push_str(&sanitize_key(serial));
    } else if !uuid.is_empty() {
        identity.push_str("&UUID_");
        identity.push_str(&sanitize_key(uuid));
    }
    identity
}

fn make_hid_stable_key(hid: &HidMetadata) -> String {
    if !hid.serial.trim().is_empty() {
        return make_stable_key(Some(hid.vendor_id), Some(hid.product_id), &hid.serial, "");
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hid.path.hash(&mut hasher);
    format!("VID_{:04X}&PID_{:04X}&PATH_{:016X}", hid.vendor_id, hid.product_id, hasher.finish())
}

fn synthetic_hid_id(stable_key: &str) -> i32 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stable_key.hash(&mut hasher);
    100_000 + (hasher.finish() & 0x3FFF_FFFF) as i32
}

fn sanitize_key(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}
