use super::model::{ControllerFamily, DeviceClass, HidMetadata};

pub fn classify(name: &str, hid: Option<&HidMetadata>) -> (ControllerFamily, DeviceClass) {
    let lower = name.to_ascii_lowercase();
    let vid = hid.map(|h| h.vendor_id).unwrap_or(0);
    let pid = hid.map(|h| h.product_id).unwrap_or(0);
    let usage_page = hid.map(|h| h.usage_page).unwrap_or(0);
    let usage = hid.map(|h| h.usage).unwrap_or(0);

    let family = match (vid, pid) {
        (0x054C, 0x05C4 | 0x09CC) => ControllerFamily::DualShock4,
        (0x054C, 0x0CE6) => ControllerFamily::DualSense,
        (0x054C, 0x0DF2) => ControllerFamily::DualSenseEdge,
        (0x057E, 0x2009) => ControllerFamily::NintendoSwitchPro,
        (0x057E, 0x2006) => ControllerFamily::NintendoJoyConLeft,
        (0x057E, 0x2007) => ControllerFamily::NintendoJoyConRight,
        (0x045E, 0x028E) => ControllerFamily::Xbox360,
        (0x045E, 0x02D1 | 0x02DD | 0x02E0 | 0x02EA) => ControllerFamily::XboxOne,
        (0x045E, 0x0B12 | 0x0B13) => ControllerFamily::XboxSeries,
        (0x28DE, 0x1102 | 0x1142) => ControllerFamily::SteamController,
        (0x28DE, _) if lower.contains("steam deck") => ControllerFamily::SteamDeck,
        (0x046D, _) => ControllerFamily::Logitech,
        (0x044F, _) => ControllerFamily::Thrustmaster,
        (0x0EB7, _) => ControllerFamily::Fanatec,
        (0x2DC8, _) => ControllerFamily::EightBitDo,
        (0x1532, _) => ControllerFamily::Razer,
        (0x0F0D, _) => ControllerFamily::Hori,
        (0x3537, _) => ControllerFamily::GameSir,
        _ if lower.contains("dualsense edge") => ControllerFamily::DualSenseEdge,
        _ if lower.contains("dualsense") || lower.contains("ps5") => ControllerFamily::DualSense,
        _ if lower.contains("dualshock") || lower.contains("ps4") => ControllerFamily::DualShock4,
        _ if lower.contains("xbox 360") => ControllerFamily::Xbox360,
        _ if lower.contains("xbox series") => ControllerFamily::XboxSeries,
        _ if lower.contains("xbox") || lower.contains("xinput") => ControllerFamily::XboxOne,
        _ if lower.contains("switch pro") || lower.contains("nintendo") => ControllerFamily::NintendoSwitchPro,
        _ if lower.contains("8bitdo") => ControllerFamily::EightBitDo,
        _ if lower.contains("logitech") => ControllerFamily::Logitech,
        _ if lower.contains("thrustmaster") => ControllerFamily::Thrustmaster,
        _ if lower.contains("fanatec") => ControllerFamily::Fanatec,
        _ if lower.contains("razer") => ControllerFamily::Razer,
        _ if lower.contains("hori") => ControllerFamily::Hori,
        _ if lower.contains("gamesir") => ControllerFamily::GameSir,
        _ if vid != 0 || lower.contains("controller") || lower.contains("gamepad") => ControllerFamily::Generic,
        _ => ControllerFamily::Unknown,
    };

    let class = classify_device_class(&lower, usage_page, usage);
    (family, class)
}

fn classify_device_class(lower: &str, usage_page: u16, usage: u16) -> DeviceClass {
    if lower.contains("wheel") || lower.contains("g29") || lower.contains("g27") || lower.contains("racing") {
        return DeviceClass::RacingWheel;
    }
    if lower.contains("hotas") || (lower.contains("throttle") && lower.contains("joystick")) {
        return DeviceClass::Hotas;
    }
    if lower.contains("flight") || lower.contains("joystick") || lower.contains("flightstick") {
        return DeviceClass::FlightStick;
    }
    if lower.contains("pedal") || lower.contains("rudder") {
        return DeviceClass::Pedals;
    }
    if lower.contains("farm") || lower.contains("heavy equipment") || lower.contains("side panel") {
        return DeviceClass::FarmController;
    }
    if lower.contains("arcade") || lower.contains("fight stick") || lower.contains("fightstick") {
        return DeviceClass::ArcadeStick;
    }
    if lower.contains("button box") || lower.contains("button panel") || lower.contains("stream deck") {
        return DeviceClass::ButtonPanel;
    }

    // HID Generic Desktop: Joystick=0x04, Game Pad=0x05, Multi-axis=0x08.
    if usage_page == 0x01 {
        return match usage {
            0x04 => DeviceClass::FlightStick,
            0x05 => DeviceClass::Gamepad,
            0x08 => DeviceClass::GenericHid,
            _ => DeviceClass::Gamepad,
        };
    }

    // HID Simulation Controls top-level collections. The descriptor parser can
    // refine this further after the device is selected/opened.
    if usage_page == 0x02 {
        return match usage {
            0x01 => DeviceClass::FlightStick, // Flight Simulation Device
            0x02 => DeviceClass::RacingWheel, // Automobile Simulation Device
            0x03 => DeviceClass::GenericHid,  // Tank Simulation Device
            0x04 => DeviceClass::FlightStick, // Spaceship Simulation Device
            0x05 => DeviceClass::GenericHid,  // Submarine Simulation Device
            0x06 => DeviceClass::GenericHid,  // Sailing Simulation Device
            0x07 => DeviceClass::GenericHid,  // Motorcycle Simulation Device
            0x08 => DeviceClass::GenericHid,  // Sports Simulation Device
            0x09 => DeviceClass::FlightStick, // Airplane Simulation Device
            0x0A => DeviceClass::FlightStick, // Helicopter Simulation Device
            0x0B => DeviceClass::GenericHid,  // Magic Carpet Simulation Device
            0x0C => DeviceClass::GenericHid,  // Bicycle Simulation Device
            0x20 | 0x21 | 0x24 => DeviceClass::FlightStick,
            0x22 | 0x23 => DeviceClass::GenericHid,
            0x25 => DeviceClass::FarmController,
            _ => DeviceClass::GenericHid,
        };
    }

    if lower.contains("controller") || lower.contains("gamepad") || lower.contains("xbox") || lower.contains("dual") {
        DeviceClass::Gamepad
    } else {
        DeviceClass::Unknown
    }
}
