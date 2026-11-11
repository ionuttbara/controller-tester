#[cfg(windows)]
mod imp {
    use super::super::JoyKeyEngine;
    use crate::device_manager::classifier;
    use crate::device_manager::model::{HidMetadata, Transport};
    use crate::device_manager::semantic_hid::SemanticHidReader;
    use crate::input::{BTN_PHYSICAL_BASE, MAX_BUTTONS};
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    struct TrackedHid {
        path: String,
        vendor_id: u16,
        product_id: u16,
        reader: SemanticHidReader,
        previous: [bool; MAX_BUTTONS],
    }

    pub struct HidJoyKeyPoller {
        devices: Vec<TrackedHid>,
        last_scan: Instant,
    }

    impl Default for HidJoyKeyPoller {
        fn default() -> Self {
            Self {
                devices: Vec::new(),
                last_scan: Instant::now() - Duration::from_secs(10),
            }
        }
    }

    impl HidJoyKeyPoller {
        pub fn poll(&mut self, engine: &mut JoyKeyEngine) {
            if self.last_scan.elapsed() >= Duration::from_secs(3) {
                self.rescan(engine);
                self.last_scan = Instant::now();
            }

            for device in &mut self.devices {
                let Some(snapshot) = device.reader.read_latest() else { continue; };
                let source = format!("hid:{}", device.path);
                for index in 0..MAX_BUTTONS {
                    let now = snapshot.input.buttons[index];
                    let old = device.previous[index];
                    if now == old {
                        continue;
                    }
                    if let Some(action) = action_for_button_slot(index) {
                        engine.send_mapped_action_from(&source, &action, now);
                    }
                    device.previous[index] = now;
                }
            }
        }

        pub fn matches_gilrs_device(&self, vendor_id: Option<u16>, product_id: Option<u16>) -> bool {
            match (vendor_id, product_id) {
                (Some(vid), Some(pid)) => self.devices.iter().any(|d| d.vendor_id == vid && d.product_id == pid),
                _ => false,
            }
        }

        fn rescan(&mut self, engine: &mut JoyKeyEngine) {
            let Ok(api) = hidapi::HidApi::new() else { return; };
            let mut candidates = Vec::new();
            let mut seen = HashSet::new();

            for info in api.device_list() {
                if info.vendor_id() == 0x045E {
                    // Xbox/XInput has a dedicated poller and should not be injected twice.
                    continue;
                }
                let controller_collection =
                    (info.usage_page() == 0x01 && matches!(info.usage(), 0x04 | 0x05 | 0x08))
                    || (info.usage_page() == 0x02
                        && (matches!(info.usage(), 0x01..=0x0C) || matches!(info.usage(), 0x20..=0x25)));
                if !controller_collection {
                    continue;
                }
                let path = info.path().to_string_lossy().into_owned();
                if !seen.insert(path.clone()) {
                    continue;
                }
                let metadata = HidMetadata {
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    release_number: info.release_number(),
                    manufacturer: info.manufacturer_string().unwrap_or_default().to_string(),
                    product: info.product_string().unwrap_or_default().to_string(),
                    serial: info.serial_number().unwrap_or_default().to_string(),
                    path: path.clone(),
                    usage_page: info.usage_page(),
                    usage: info.usage(),
                    interface_number: info.interface_number(),
                    transport: match info.bus_type() {
                        hidapi::BusType::Usb => Transport::Usb,
                        hidapi::BusType::Bluetooth => Transport::Bluetooth,
                        _ => Transport::Unknown,
                    },
                };
                let product = if metadata.product.is_empty() { "HID Controller" } else { &metadata.product };
                let (family, class) = classifier::classify(product, Some(&metadata));
                candidates.push((path, metadata.vendor_id, metadata.product_id, family, class));
            }

            let candidate_paths = candidates.iter().map(|c| c.0.as_str()).collect::<HashSet<_>>();
            let removed = self
                .devices
                .iter()
                .filter(|d| !candidate_paths.contains(d.path.as_str()))
                .map(|d| format!("hid:{}", d.path))
                .collect::<Vec<_>>();
            for source in removed {
                engine.release_source(&source);
            }
            self.devices.retain(|d| candidate_paths.contains(d.path.as_str()));

            let existing = self.devices.iter().map(|d| d.path.clone()).collect::<HashSet<_>>();
            for (path, vid, pid, family, class) in candidates {
                if existing.contains(&path) {
                    continue;
                }
                if let Ok(reader) = SemanticHidReader::open(&path, family, class) {
                    self.devices.push(TrackedHid {
                        path,
                        vendor_id: vid,
                        product_id: pid,
                        reader,
                        previous: [false; MAX_BUTTONS],
                    });
                }
            }
        }
    }

    fn action_for_button_slot(index: usize) -> Option<String> {
        let canonical = match index {
            0 => Some("South"),
            1 => Some("East"),
            2 => Some("West"),
            3 => Some("North"),
            4 => Some("LeftShoulder"),
            5 => Some("RightShoulder"),
            6 => Some("Select"),
            7 => Some("Start"),
            8 => Some("LeftThumb"),
            9 => Some("RightThumb"),
            10 => Some("DPadUp"),
            11 => Some("DPadRight"),
            12 => Some("DPadDown"),
            13 => Some("DPadLeft"),
            14 => Some("Guide"),
            15 => Some("LeftTrigger"),
            16 => Some("RightTrigger"),
            17 => Some("Touchpad"),
            _ => None,
        };
        if let Some(action) = canonical {
            return Some(action.to_string());
        }
        if index >= BTN_PHYSICAL_BASE {
            return Some(format!("B{}", index - BTN_PHYSICAL_BASE + 1));
        }
        None
    }
}

#[cfg(not(windows))]
mod imp {
    use super::super::JoyKeyEngine;
    #[derive(Default)]
    pub struct HidJoyKeyPoller;
    impl HidJoyKeyPoller {
        pub fn poll(&mut self, _engine: &mut JoyKeyEngine) {}
        pub fn matches_gilrs_device(&self, _vendor_id: Option<u16>, _product_id: Option<u16>) -> bool { false }
    }
}

pub use imp::HidJoyKeyPoller;
