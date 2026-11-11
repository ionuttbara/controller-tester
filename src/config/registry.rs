use anyhow::Result;
use crate::config::models::{CalibrationProfile, JoyKeyProfile, WheelCalibration};

pub const APP_REG_PATH: &str = r"Software\Gallery Inc\IBBE-ControllerManager";
pub const JOYKEY_REG_PATH: &str = r"Software\Gallery Inc\IBBE-ControllerManager\JoyToKeyCfgList";

#[derive(Debug, Clone)]
pub struct AppSettings {
    pub joykey_enabled: bool,
    pub renderer: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            joykey_enabled: false,
            renderer: "auto".to_string(),
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::io;
    use winreg::enums::*;
    use winreg::RegKey;

    pub fn load_settings() -> AppSettings {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey(APP_REG_PATH) {
            Ok(key) => AppSettings {
                joykey_enabled: key.get_value::<u32, _>("JoyKeyEnabled").unwrap_or(0) != 0,
                renderer: key.get_value::<String, _>("Renderer").unwrap_or_else(|_| "auto".to_string()),
            },
            Err(_) => AppSettings::default(),
        }
    }


    pub fn save_renderer_preference(renderer: &str) -> Result<()> {
        let normalized = match renderer.trim().to_ascii_lowercase().as_str() {
            "gpu" => "gpu",
            "cpu" => "cpu",
            _ => "auto",
        }.to_string();
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(APP_REG_PATH)?;
        key.set_value("Renderer", &normalized)?;
        Ok(())
    }

    pub fn save_joykey_enabled(enabled: bool) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(APP_REG_PATH)?;
        key.set_value("JoyKeyEnabled", &(enabled as u32))?;
        set_startup_agent(enabled)?;
        Ok(())
    }

    pub fn set_startup_agent(enabled: bool) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (run, _) = hkcu.create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")?;
        let value_name = "IBBE ControllerManager JoyKey";
        if enabled {
            let exe = std::env::current_exe()?;
            let command = format!("\"{}\" --joykey-agent", exe.display());
            run.set_value(value_name, &command)?;
        } else {
            match run.delete_value(value_name) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }

    pub fn save_calibration(device_key: &str, profile: &CalibrationProfile) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"{}\Controllers\{}", APP_REG_PATH, device_key);
        let (key, _) = hkcu.create_subkey(path)?;
        key.set_value("CalibrationJson", &serde_json::to_string(profile)?)?;
        Ok(())
    }

    pub fn load_calibration(device_key: &str) -> Option<CalibrationProfile> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"{}\Controllers\{}", APP_REG_PATH, device_key);
        let key = hkcu.open_subkey(path).ok()?;
        let json: String = key.get_value("CalibrationJson").ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn save_wheel_calibration(device_key: &str, profile: &WheelCalibration) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"{}\Controllers\{}", APP_REG_PATH, device_key);
        let (key, _) = hkcu.create_subkey(path)?;
        key.set_value("WheelCalibrationJson", &serde_json::to_string(profile)?)?;
        Ok(())
    }

    pub fn load_wheel_calibration(device_key: &str) -> Option<WheelCalibration> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!(r"{}\Controllers\{}", APP_REG_PATH, device_key);
        let key = hkcu.open_subkey(path).ok()?;
        let json: String = key.get_value("WheelCalibrationJson").ok()?;
        serde_json::from_str(&json).ok()
    }

    pub fn save_joykey_profile(profile: &JoyKeyProfile) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let process = sanitize_process_name(&profile.process);
        let path = format!(r"{}\{}", JOYKEY_REG_PATH, process);
        let (key, _) = hkcu.create_subkey(path)?;
        key.set_value("Enabled", &(profile.enabled as u32))?;
        key.set_value("MappingJson", &serde_json::to_string(&profile.mappings)?)?;
        Ok(())
    }

    pub fn load_joykey_profile(process: &str) -> Option<JoyKeyProfile> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let process_key = sanitize_process_name(process);
        let path = format!(r"{}\{}", JOYKEY_REG_PATH, process_key);
        let key = hkcu.open_subkey(path).ok()?;
        let enabled = key.get_value::<u32, _>("Enabled").unwrap_or(1) != 0;
        let json: String = key.get_value("MappingJson").unwrap_or_else(|_| "{}".to_string());
        let mappings = serde_json::from_str(&json).unwrap_or_default();
        Some(JoyKeyProfile { process: process.to_string(), enabled, mappings })
    }


    pub fn list_joykey_profiles() -> Vec<String> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let Ok(key) = hkcu.open_subkey(JOYKEY_REG_PATH) else {
            return Vec::new();
        };
        let mut names = key.enum_keys().filter_map(Result::ok).collect::<Vec<_>>();
        names.sort_by_key(|name| name.to_ascii_lowercase());
        names
    }

    pub fn delete_joykey_profile(process: &str) -> Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let process_key = sanitize_process_name(process);
        let parent = match hkcu.open_subkey_with_flags(JOYKEY_REG_PATH, KEY_READ | KEY_WRITE) {
            Ok(key) => key,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        match parent.delete_subkey_all(process_key) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    fn sanitize_process_name(name: &str) -> String {
        name.trim().replace('\\', "_").replace('/', "_")
    }

    pub fn launch_joykey_agent() -> Result<()> {
        let exe = std::env::current_exe()?;
        std::process::Command::new(exe).arg("--joykey-agent").spawn()?;
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;
    pub fn load_settings() -> AppSettings { AppSettings::default() }
    pub fn save_renderer_preference(_renderer: &str) -> Result<()> { Ok(()) }
    pub fn save_joykey_enabled(_enabled: bool) -> Result<()> { Ok(()) }
    pub fn set_startup_agent(_enabled: bool) -> Result<()> { Ok(()) }
    pub fn save_calibration(_device_key: &str, _profile: &CalibrationProfile) -> Result<()> { Ok(()) }
    pub fn load_calibration(_device_key: &str) -> Option<CalibrationProfile> { None }
    pub fn save_wheel_calibration(_device_key: &str, _profile: &WheelCalibration) -> Result<()> { Ok(()) }
    pub fn load_wheel_calibration(_device_key: &str) -> Option<WheelCalibration> { None }
    pub fn save_joykey_profile(_profile: &JoyKeyProfile) -> Result<()> { Ok(()) }
    pub fn load_joykey_profile(_process: &str) -> Option<JoyKeyProfile> { None }
    pub fn list_joykey_profiles() -> Vec<String> { Vec::new() }
    pub fn delete_joykey_profile(_process: &str) -> Result<()> { Ok(()) }
    pub fn launch_joykey_agent() -> Result<()> { Ok(()) }
}

pub use imp::*;
