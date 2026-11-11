pub mod hid;
pub mod keyboard;
pub mod process;
pub mod single_instance;
pub mod xinput;

use anyhow::Result;
use gilrs::{Axis, Button, Event, EventType, GilrsBuilder};
use std::collections::HashMap;
use std::thread;
use std::time::{Duration, Instant};

use crate::config::models::JoyKeyProfile;
use crate::config::registry;

const ANALOG_PRESS_THRESHOLD: f32 = 0.18;
const ANALOG_RELEASE_THRESHOLD: f32 = 0.10;

#[derive(Debug)]
pub struct JoyKeyEngine {
    enabled: bool,
    foreground: Option<String>,
    last_process_check: Instant,
    dpad_state: HashMap<String, (i8, i8)>,
    analog_state: HashMap<String, bool>,
    // source/action -> virtual key. One physical source can therefore hold the
    // same action independently from another controller.
    held_sources: HashMap<String, u16>,
    // Multiple actions/controllers may map to the same key. Key-up is emitted
    // only when the last owner releases it.
    key_owners: HashMap<u16, usize>,
}

impl JoyKeyEngine {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            foreground: None,
            last_process_check: Instant::now() - Duration::from_secs(1),
            dpad_state: HashMap::new(),
            analog_state: HashMap::new(),
            held_sources: HashMap::new(),
            key_owners: HashMap::new(),
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled && !enabled {
            self.release_all();
        }
        self.enabled = enabled;
    }

    pub fn tick(&mut self) {
        if !self.enabled || self.last_process_check.elapsed() < Duration::from_millis(50) {
            return;
        }
        let previous = self.foreground.clone();
        self.last_process_check = Instant::now();
        self.foreground = process::foreground_process_name();
        if previous.is_some() && previous != self.foreground {
            // Never carry a held synthetic key from one foreground process into
            // another profile/window.
            self.release_all();
        }
    }

    pub fn handle_event(&mut self, event: &Event) {
        if !self.enabled {
            return;
        }
        let source = format!("gilrs:{}", usize::from(event.id));
        match &event.event {
            EventType::ButtonPressed(button, _) => {
                self.send_mapped_action_from(&source, button_action(*button), true)
            }
            EventType::ButtonReleased(button, _) => {
                self.send_mapped_action_from(&source, button_action(*button), false)
            }
            EventType::ButtonChanged(button, value, _) => {
                let action = button_action(*button);
                if matches!(*button, Button::LeftTrigger2 | Button::RightTrigger2) {
                    self.handle_analog_button(&source, action, *value);
                }
            }
            EventType::AxisChanged(Axis::DPadX, value, _) => {
                self.handle_dpad_axis(&source, true, *value)
            }
            EventType::AxisChanged(Axis::DPadY, value, _) => {
                self.handle_dpad_axis(&source, false, *value)
            }
            EventType::Disconnected => self.release_source(&source),
            _ => {}
        }
    }

    fn handle_analog_button(&mut self, source: &str, action: &str, value: f32) {
        if action == "Unknown" {
            return;
        }
        let state_key = source_action_key(source, action);
        let previous = *self.analog_state.get(&state_key).unwrap_or(&false);
        let next = if previous {
            value > ANALOG_RELEASE_THRESHOLD
        } else {
            value >= ANALOG_PRESS_THRESHOLD
        };
        if next != previous {
            self.analog_state.insert(state_key, next);
            self.send_mapped_action_from(source, action, next);
        }
    }

    fn handle_dpad_axis(&mut self, source: &str, horizontal: bool, value: f32) {
        let next = if value > 0.5 { 1 } else if value < -0.5 { -1 } else { 0 };
        let (old_x, old_y) = self.dpad_state.get(source).copied().unwrap_or((0, 0));
        let previous = if horizontal { old_x } else { old_y };
        if next == previous {
            return;
        }

        if previous != 0 {
            let action = if horizontal {
                if previous > 0 { "DPadRight" } else { "DPadLeft" }
            } else if previous > 0 {
                "DPadUp"
            } else {
                "DPadDown"
            };
            self.send_mapped_action_from(source, action, false);
        }
        if next != 0 {
            let action = if horizontal {
                if next > 0 { "DPadRight" } else { "DPadLeft" }
            } else if next > 0 {
                "DPadUp"
            } else {
                "DPadDown"
            };
            self.send_mapped_action_from(source, action, true);
        }
        let state = if horizontal { (next, old_y) } else { (old_x, next) };
        self.dpad_state.insert(source.to_string(), state);
    }

    pub fn send_mapped_action_from(&mut self, source: &str, action: &str, pressed: bool) {
        if !self.enabled || action == "Unknown" {
            return;
        }
        let hold_key = source_action_key(source, action);

        if !pressed {
            self.release_hold_key(&hold_key);
            return;
        }
        if self.held_sources.contains_key(&hold_key) {
            return;
        }

        self.refresh_foreground_now();
        let Some(process) = self.foreground.clone() else { return; };
        let Some(profile) = self.profile_for(&process) else { return; };
        if !profile.enabled {
            return;
        }
        let Some(key_name) = profile.mappings.get(action) else { return; };
        let Some(vk) = keyboard::parse_virtual_key(key_name) else { return; };

        let owners = self.key_owners.entry(vk).or_insert(0);
        if *owners == 0 {
            keyboard::send_key(vk, true);
        }
        *owners += 1;
        self.held_sources.insert(hold_key, vk);
    }

    pub fn release_source(&mut self, source: &str) {
        let prefix = format!("{source}\u{1f}");
        let keys = self
            .held_sources
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.release_hold_key(&key);
        }
        self.dpad_state.remove(source);
        self.analog_state.retain(|key, _| !key.starts_with(&prefix));
    }

    pub fn release_all(&mut self) {
        let keys = self.held_sources.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.release_hold_key(&key);
        }
        self.dpad_state.clear();
        self.analog_state.clear();
    }

    fn release_hold_key(&mut self, hold_key: &str) {
        let Some(vk) = self.held_sources.remove(hold_key) else { return; };
        let mut should_release = false;
        if let Some(owners) = self.key_owners.get_mut(&vk) {
            *owners = owners.saturating_sub(1);
            should_release = *owners == 0;
        }
        if should_release {
            self.key_owners.remove(&vk);
            keyboard::send_key(vk, false);
        }
    }

    fn refresh_foreground_now(&mut self) {
        let previous = self.foreground.clone();
        self.last_process_check = Instant::now();
        self.foreground = process::foreground_process_name();
        if previous.is_some() && previous != self.foreground {
            self.release_all();
        }
    }

    fn profile_for(&self, process: &str) -> Option<JoyKeyProfile> {
        // Profiles are deliberately re-read on new press transitions, so edits
        // made by the UI become active without restarting the agent.
        registry::load_joykey_profile(process)
    }
}

fn source_action_key(source: &str, action: &str) -> String {
    format!("{source}\u{1f}{action}")
}

pub fn button_action(button: Button) -> &'static str {
    match button {
        Button::South => "South",
        Button::East => "East",
        Button::West => "West",
        Button::North => "North",
        Button::LeftTrigger => "LeftShoulder",
        Button::RightTrigger => "RightShoulder",
        Button::LeftTrigger2 => "LeftTrigger",
        Button::RightTrigger2 => "RightTrigger",
        Button::Select => "Select",
        Button::Start => "Start",
        Button::Mode => "Guide",
        Button::LeftThumb => "LeftThumb",
        Button::RightThumb => "RightThumb",
        Button::DPadUp => "DPadUp",
        Button::DPadDown => "DPadDown",
        Button::DPadLeft => "DPadLeft",
        Button::DPadRight => "DPadRight",
        _ => "Unknown",
    }
}

pub fn run_headless_agent() -> Result<()> {
    let Some(_instance_guard) = single_instance::try_acquire()? else {
        return Ok(());
    };
    if !registry::load_settings().joykey_enabled {
        return Ok(());
    }

    let mut gilrs = GilrsBuilder::new()
        .with_default_filters(false)
        .build()
        .map_err(|e| anyhow::anyhow!("JoyKey gilrs init failed: {e}"))?;
    let mut engine = JoyKeyEngine::new(true);
    let mut xinput = xinput::XInputJoyKeyPoller::default();
    let mut hid = hid::HidJoyKeyPoller::default();
    let mut settings_check = Instant::now();

    loop {
        engine.tick();
        xinput.poll(&mut engine);
        hid.poll(&mut engine);

        while let Some(event) = gilrs.next_event() {
            let gamepad = gilrs.gamepad(event.id);
            let is_xbox = gamepad.vendor_id() == Some(0x045E)
                || gamepad.name().to_ascii_lowercase().contains("xbox");
            // Direct HID handles non-Xbox devices in background. gilrs remains a
            // fallback for devices which could not be opened by the HID poller;
            // duplicate source ownership is harmless but we avoid the common path.
            if !is_xbox && !hid.matches_gilrs_device(gamepad.vendor_id(), gamepad.product_id()) {
                engine.handle_event(&event);
            }
        }

        if settings_check.elapsed() >= Duration::from_secs(1) {
            if !registry::load_settings().joykey_enabled {
                engine.set_enabled(false);
                break;
            }
            settings_check = Instant::now();
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
