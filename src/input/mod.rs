pub mod raw_input;

use gilrs::{Axis, Button, Gamepad};

pub const MAX_AXES: usize = 16;
pub const MAX_BUTTONS: usize = 32;

// Canonical button slots used by the UI preview and by semantic HID decoding.
pub const BTN_SOUTH: usize = 0;
pub const BTN_EAST: usize = 1;
pub const BTN_WEST: usize = 2;
pub const BTN_NORTH: usize = 3;
pub const BTN_LB: usize = 4;
pub const BTN_RB: usize = 5;
pub const BTN_SELECT: usize = 6;
pub const BTN_START: usize = 7;
pub const BTN_LTHUMB: usize = 8;
pub const BTN_RTHUMB: usize = 9;
pub const BTN_DPAD_UP: usize = 10;
pub const BTN_DPAD_RIGHT: usize = 11;
pub const BTN_DPAD_DOWN: usize = 12;
pub const BTN_DPAD_LEFT: usize = 13;
pub const BTN_GUIDE: usize = 14;
pub const BTN_LT: usize = 15;
pub const BTN_RT: usize = 16;
pub const BTN_TOUCHPAD: usize = 17;

// Physical HID Button-page usages B1..B14 are mirrored here. This keeps the
// standardized preview independent from the raw physical button numbering.
pub const BTN_PHYSICAL_BASE: usize = 18;

#[derive(Debug, Clone)]
pub struct InputSnapshot {
    pub lx: f32,
    pub ly: f32,
    pub rx: f32,
    pub ry: f32,
    pub lt: f32,
    pub rt: f32,
    pub axes: [f32; MAX_AXES],
    pub buttons: [bool; MAX_BUTTONS],
}

impl Default for InputSnapshot {
    fn default() -> Self {
        Self {
            lx: 0.0,
            ly: 0.0,
            rx: 0.0,
            ry: 0.0,
            lt: 0.0,
            rt: 0.0,
            axes: [0.0; MAX_AXES],
            buttons: [false; MAX_BUTTONS],
        }
    }
}

impl InputSnapshot {
    pub fn from_gamepad(gamepad: Gamepad<'_>) -> Self {
        let lx = gamepad.value(Axis::LeftStickX);
        let ly = gamepad.value(Axis::LeftStickY);
        let rx = gamepad.value(Axis::RightStickX);
        let ry = gamepad.value(Axis::RightStickY);

        // In gilrs triggers are normally mapped as analog buttons, not axes.
        // Reading ButtonData first fixes LT/RT being stuck at zero on many XInput,
        // DualShock and DualSense mappings. Axis Z is retained as a fallback for
        // older/DirectInput mappings which expose triggers as axes.
        let lt_button = analog_button_value(&gamepad, Button::LeftTrigger2);
        let rt_button = analog_button_value(&gamepad, Button::RightTrigger2);
        let lt_axis = normalize_trigger_axis(gamepad.value(Axis::LeftZ));
        let rt_axis = normalize_trigger_axis(gamepad.value(Axis::RightZ));
        let lt = if lt_button > 0.001 { lt_button } else { lt_axis };
        let rt = if rt_button > 0.001 { rt_button } else { rt_axis };

        let mut axes = [0.0; MAX_AXES];
        axes[0] = lx;
        axes[1] = ly;
        axes[2] = rx;
        axes[3] = ry;
        axes[4] = lt;
        axes[5] = rt;
        axes[6] = gamepad.value(Axis::DPadX);
        axes[7] = gamepad.value(Axis::DPadY);
        axes[8] = gamepad.value(Axis::LeftZ);
        axes[9] = gamepad.value(Axis::RightZ);

        let mut buttons = [false; MAX_BUTTONS];
        buttons[BTN_SOUTH] = gamepad.is_pressed(Button::South);
        buttons[BTN_EAST] = gamepad.is_pressed(Button::East);
        buttons[BTN_WEST] = gamepad.is_pressed(Button::West);
        buttons[BTN_NORTH] = gamepad.is_pressed(Button::North);
        buttons[BTN_LB] = gamepad.is_pressed(Button::LeftTrigger);
        buttons[BTN_RB] = gamepad.is_pressed(Button::RightTrigger);
        buttons[BTN_SELECT] = gamepad.is_pressed(Button::Select);
        buttons[BTN_START] = gamepad.is_pressed(Button::Start);
        buttons[BTN_LTHUMB] = gamepad.is_pressed(Button::LeftThumb);
        buttons[BTN_RTHUMB] = gamepad.is_pressed(Button::RightThumb);
        let dpad_x = axes[6];
        let dpad_y = axes[7];
        buttons[BTN_DPAD_UP] = gamepad.is_pressed(Button::DPadUp) || dpad_y > 0.5;
        buttons[BTN_DPAD_RIGHT] = gamepad.is_pressed(Button::DPadRight) || dpad_x > 0.5;
        buttons[BTN_DPAD_DOWN] = gamepad.is_pressed(Button::DPadDown) || dpad_y < -0.5;
        buttons[BTN_DPAD_LEFT] = gamepad.is_pressed(Button::DPadLeft) || dpad_x < -0.5;
        buttons[BTN_GUIDE] = gamepad.is_pressed(Button::Mode);
        buttons[BTN_LT] = lt >= 0.12 || gamepad.is_pressed(Button::LeftTrigger2);
        buttons[BTN_RT] = rt >= 0.12 || gamepad.is_pressed(Button::RightTrigger2);

        Self { lx, ly, rx, ry, lt, rt, axes, buttons }
    }

    pub fn set_action(&mut self, action: &str, pressed: bool) {
        let index = match action {
            "South" => Some(BTN_SOUTH),
            "East" => Some(BTN_EAST),
            "West" => Some(BTN_WEST),
            "North" => Some(BTN_NORTH),
            "LeftShoulder" => Some(BTN_LB),
            "RightShoulder" => Some(BTN_RB),
            "Select" => Some(BTN_SELECT),
            "Start" => Some(BTN_START),
            "LeftThumb" => Some(BTN_LTHUMB),
            "RightThumb" => Some(BTN_RTHUMB),
            "DPadUp" => Some(BTN_DPAD_UP),
            "DPadRight" => Some(BTN_DPAD_RIGHT),
            "DPadDown" => Some(BTN_DPAD_DOWN),
            "DPadLeft" => Some(BTN_DPAD_LEFT),
            "Guide" => Some(BTN_GUIDE),
            "LeftTrigger" => Some(BTN_LT),
            "RightTrigger" => Some(BTN_RT),
            "Touchpad" => Some(BTN_TOUCHPAD),
            _ => None,
        };
        if let Some(index) = index {
            self.buttons[index] = pressed;
        }
    }

    pub fn set_physical_button(&mut self, usage_id: u16, pressed: bool) {
        if usage_id == 0 {
            return;
        }
        let slot = BTN_PHYSICAL_BASE + usage_id.saturating_sub(1) as usize;
        if slot < self.buttons.len() {
            self.buttons[slot] = pressed;
        }
    }
}

fn analog_button_value(gamepad: &Gamepad<'_>, button: Button) -> f32 {
    gamepad
        .button_data(button)
        .map(|data| data.value().clamp(0.0, 1.0))
        .unwrap_or(0.0)
}

fn normalize_trigger_axis(value: f32) -> f32 {
    // Some DirectInput drivers expose an unpressed trigger as -1.0 and pressed as
    // +1.0, while others already expose 0.0..1.0. Avoid turning a centered 0.0
    // axis into 50% by only remapping values that clearly use the signed range.
    if value < -0.05 {
        ((value + 1.0) * 0.5).clamp(0.0, 1.0)
    } else {
        value.clamp(0.0, 1.0)
    }
}
