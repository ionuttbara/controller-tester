use gilrs::PowerInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Gamepad,
    RacingWheel,
    FlightStick,
    Hotas,
    Pedals,
    FarmController,
    ArcadeStick,
    ButtonPanel,
    GenericHid,
    Unknown,
}

impl DeviceClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gamepad => "Gamepad",
            Self::RacingWheel => "Racing Wheel",
            Self::FlightStick => "Flight Stick",
            Self::Hotas => "HOTAS",
            Self::Pedals => "Pedals",
            Self::FarmController => "Farm / Simulation",
            Self::ArcadeStick => "Arcade / Fight Stick",
            Self::ButtonPanel => "Button Panel",
            Self::GenericHid => "Generic HID",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerFamily {
    Xbox360,
    XboxOne,
    XboxSeries,
    DualShock4,
    DualSense,
    DualSenseEdge,
    NintendoSwitchPro,
    NintendoJoyConLeft,
    NintendoJoyConRight,
    SteamController,
    SteamDeck,
    Logitech,
    Thrustmaster,
    Fanatec,
    EightBitDo,
    Razer,
    Hori,
    GameSir,
    Generic,
    Unknown,
}

impl ControllerFamily {
    pub fn description(self) -> &'static str {
        match self {
            Self::Xbox360 => "Xbox 360",
            Self::XboxOne => "Xbox One",
            Self::XboxSeries => "Xbox Series X|S",
            Self::DualShock4 => "PlayStation DualShock 4",
            Self::DualSense => "PlayStation 5 DualSense",
            Self::DualSenseEdge => "PlayStation 5 DualSense Edge",
            Self::NintendoSwitchPro => "Nintendo Switch Pro Controller",
            Self::NintendoJoyConLeft => "Nintendo Joy-Con (L)",
            Self::NintendoJoyConRight => "Nintendo Joy-Con (R)",
            Self::SteamController => "Steam Controller",
            Self::SteamDeck => "Steam Deck Controller",
            Self::Logitech => "Logitech Controller",
            Self::Thrustmaster => "Thrustmaster Controller",
            Self::Fanatec => "Fanatec Controller",
            Self::EightBitDo => "8BitDo Controller",
            Self::Razer => "Razer Controller",
            Self::Hori => "HORI Controller",
            Self::GameSir => "GameSir Controller",
            Self::Generic => "Generic PC Controller",
            Self::Unknown => "Unknown Controller",
        }
    }

    pub fn artwork_key(self, class: DeviceClass) -> &'static str {
        match class {
            DeviceClass::RacingWheel => "wheel",
            DeviceClass::FlightStick | DeviceClass::Hotas => "hotas",
            DeviceClass::Pedals => "pedals",
            DeviceClass::FarmController => "simulation",
            DeviceClass::ArcadeStick => "arcade",
            DeviceClass::ButtonPanel => "panel",
            _ => match self {
                Self::Xbox360 | Self::XboxOne | Self::XboxSeries | Self::Logitech | Self::Razer | Self::GameSir => "xbox",
                Self::DualShock4 | Self::DualSense | Self::DualSenseEdge => "playstation",
                Self::NintendoSwitchPro | Self::NintendoJoyConLeft | Self::NintendoJoyConRight => "nintendo",
                Self::SteamController | Self::SteamDeck => "steam",
                _ => "generic",
            },
        }
    }

    pub fn layout(self) -> &'static str {
        match self {
            Self::Xbox360 | Self::XboxOne | Self::XboxSeries | Self::Logitech | Self::Razer | Self::GameSir => "xbox",
            Self::DualShock4 | Self::DualSense | Self::DualSenseEdge => "playstation",
            Self::NintendoSwitchPro | Self::NintendoJoyConLeft | Self::NintendoJoyConRight => "nintendo",
            _ => "generic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Usb,
    Bluetooth,
    WirelessDongle,
    Unknown,
}

impl Default for Transport {
    fn default() -> Self {
        Self::Unknown
    }
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Usb => "USB",
            Self::Bluetooth => "Bluetooth / Wireless",
            Self::WirelessDongle => "Wireless Dongle",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HidMetadata {
    pub vendor_id: u16,
    pub product_id: u16,
    pub release_number: u16,
    pub manufacturer: String,
    pub product: String,
    pub serial: String,
    pub path: String,
    pub usage_page: u16,
    pub usage: u16,
    pub interface_number: i32,
    pub transport: Transport,
}

#[derive(Debug, Clone)]
pub struct DeviceDescriptor {
    pub id: i32,
    pub name: String,
    pub display_name: String,
    pub uuid: String,
    pub manufacturer: String,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub release_number: Option<u16>,
    pub serial: String,
    pub firmware: String,
    pub hid_path: String,
    pub usage_page: Option<u16>,
    pub usage: Option<u16>,
    pub interface_number: Option<i32>,
    pub class: DeviceClass,
    pub family: ControllerFamily,
    pub transport: Transport,
    pub power_status: String,
    pub layout: String,
    pub artwork: String,
    pub ff_supported: bool,
    pub stable_key: String,
    pub backend: String,
}

impl DeviceDescriptor {
    pub fn vid_pid(&self) -> String {
        match (self.vendor_id, self.product_id) {
            (Some(vid), Some(pid)) => format!("{:04X}:{:04X}", vid, pid),
            _ => "----:----".to_string(),
        }
    }

    pub fn hid_usage_text(&self) -> String {
        match (self.usage_page, self.usage) {
            (Some(page), Some(usage)) => format!("0x{page:04X} / 0x{usage:04X}"),
            _ => "N/A".to_string(),
        }
    }
}

pub fn power_to_text(power: &PowerInfo) -> String {
    match power {
        PowerInfo::Unknown => "Power: Unknown".to_string(),
        PowerInfo::Wired => "Wired (USB)".to_string(),
        PowerInfo::Discharging(level) => format!("Battery: {level}%"),
        PowerInfo::Charging(level) => format!("Charging: {level}%"),
        PowerInfo::Charged => "Battery Full".to_string(),
    }
}

pub fn transport_from_power_and_name(power: &PowerInfo, name: &str) -> Transport {
    let lower = name.to_ascii_lowercase();
    match power {
        PowerInfo::Wired => Transport::Usb,
        PowerInfo::Discharging(_) | PowerInfo::Charging(_) | PowerInfo::Charged => Transport::Bluetooth,
        PowerInfo::Unknown if lower.contains("wireless") || lower.contains("dongle") || lower.contains("f710") => Transport::WirelessDongle,
        PowerInfo::Unknown if lower.contains("usb") || lower.contains("wired") => Transport::Usb,
        PowerInfo::Unknown => Transport::Unknown,
    }
}
