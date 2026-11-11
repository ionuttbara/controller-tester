#[cfg(windows)]
mod imp {
    use anyhow::{Context, Result};
    use std::ffi::CString;

    pub struct RawHidReader {
        device: hidapi::HidDevice,
    }

    impl RawHidReader {
        pub fn open(path: &str) -> Result<Self> {
            let api = hidapi::HidApi::new().context("HID API initialization failed")?;
            let path = CString::new(path).context("HID path contained an embedded NUL")?;
            let device = api.open_path(path.as_c_str()).context("Unable to open HID path")?;
            Ok(Self { device })
        }

        pub fn read_latest(&self) -> Option<Vec<u8>> {
            let mut latest = None;
            // Drain a small batch so the UI sees the newest packet without allowing a
            // high-rate device to monopolize the controller worker.
            for _ in 0..16 {
                let mut buffer = [0u8; 512];
                match self.device.read_timeout(&mut buffer, 0) {
                    Ok(0) => break,
                    Ok(len) => latest = Some(buffer[..len].to_vec()),
                    Err(_) => break,
                }
            }
            latest
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::{anyhow, Result};
    pub struct RawHidReader;
    impl RawHidReader {
        pub fn open(_path: &str) -> Result<Self> { Err(anyhow!("Raw HID reader is Windows-only")) }
        pub fn read_latest(&self) -> Option<Vec<u8>> { None }
    }
}

pub use imp::RawHidReader;
