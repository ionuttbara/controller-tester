use super::model::{HidMetadata, Transport};

#[cfg(windows)]
pub fn scan_hid_devices() -> Vec<HidMetadata> {
    let Ok(api) = hidapi::HidApi::new() else {
        return Vec::new();
    };

    api.device_list()
        .map(|d| HidMetadata {
            vendor_id: d.vendor_id(),
            product_id: d.product_id(),
            release_number: d.release_number(),
            manufacturer: d.manufacturer_string().unwrap_or_default().to_string(),
            product: d.product_string().unwrap_or_default().to_string(),
            serial: d.serial_number().unwrap_or_default().to_string(),
            path: d.path().to_string_lossy().into_owned(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            interface_number: d.interface_number(),
            transport: match d.bus_type() {
                hidapi::BusType::Usb => Transport::Usb,
                hidapi::BusType::Bluetooth => Transport::Bluetooth,
                _ => Transport::Unknown,
            },
        })
        .collect()
}

#[cfg(not(windows))]
pub fn scan_hid_devices() -> Vec<HidMetadata> {
    Vec::new()
}
