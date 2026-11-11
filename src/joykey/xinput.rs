#[cfg(windows)]
mod imp {
    use super::super::JoyKeyEngine;
    use windows_sys::Win32::UI::Input::XboxController::{
        XInputGetState, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_BACK,
        XINPUT_GAMEPAD_DPAD_DOWN, XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT,
        XINPUT_GAMEPAD_DPAD_UP, XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_LEFT_THUMB,
        XINPUT_GAMEPAD_RIGHT_SHOULDER, XINPUT_GAMEPAD_RIGHT_THUMB, XINPUT_GAMEPAD_START,
        XINPUT_GAMEPAD_X, XINPUT_GAMEPAD_Y, XINPUT_STATE,
    };

    const ERROR_SUCCESS: u32 = 0;
    const TRIGGER_PRESS_THRESHOLD: u8 = 35;
    const TRIGGER_RELEASE_THRESHOLD: u8 = 20;

    pub struct XInputJoyKeyPoller {
        previous_buttons: [u16; 4],
        previous_lt: [bool; 4],
        previous_rt: [bool; 4],
        connected: [bool; 4],
    }

    impl Default for XInputJoyKeyPoller {
        fn default() -> Self {
            Self {
                previous_buttons: [0; 4],
                previous_lt: [false; 4],
                previous_rt: [false; 4],
                connected: [false; 4],
            }
        }
    }

    impl XInputJoyKeyPoller {
        pub fn poll(&mut self, engine: &mut JoyKeyEngine) {
            for index in 0..4u32 {
                let source = format!("xinput:{index}");
                let mut state = XINPUT_STATE::default();
                let result = unsafe { XInputGetState(index, &mut state) };
                let slot = index as usize;
                if result != ERROR_SUCCESS {
                    if self.connected[slot] {
                        engine.release_source(&source);
                    }
                    self.previous_buttons[slot] = 0;
                    self.previous_lt[slot] = false;
                    self.previous_rt[slot] = false;
                    self.connected[slot] = false;
                    continue;
                }
                self.connected[slot] = true;

                let now = state.Gamepad.wButtons;
                let old = self.previous_buttons[slot];
                for (mask, action) in BUTTONS {
                    let was = old & mask != 0;
                    let is = now & mask != 0;
                    if was != is {
                        engine.send_mapped_action_from(&source, action, is);
                    }
                }
                self.previous_buttons[slot] = now;

                let lt = if self.previous_lt[slot] {
                    state.Gamepad.bLeftTrigger > TRIGGER_RELEASE_THRESHOLD
                } else {
                    state.Gamepad.bLeftTrigger >= TRIGGER_PRESS_THRESHOLD
                };
                if lt != self.previous_lt[slot] {
                    engine.send_mapped_action_from(&source, "LeftTrigger", lt);
                    self.previous_lt[slot] = lt;
                }
                let rt = if self.previous_rt[slot] {
                    state.Gamepad.bRightTrigger > TRIGGER_RELEASE_THRESHOLD
                } else {
                    state.Gamepad.bRightTrigger >= TRIGGER_PRESS_THRESHOLD
                };
                if rt != self.previous_rt[slot] {
                    engine.send_mapped_action_from(&source, "RightTrigger", rt);
                    self.previous_rt[slot] = rt;
                }
            }
        }
    }

    const BUTTONS: &[(u16, &str)] = &[
        (XINPUT_GAMEPAD_A, "South"),
        (XINPUT_GAMEPAD_B, "East"),
        (XINPUT_GAMEPAD_X, "West"),
        (XINPUT_GAMEPAD_Y, "North"),
        (XINPUT_GAMEPAD_LEFT_SHOULDER, "LeftShoulder"),
        (XINPUT_GAMEPAD_RIGHT_SHOULDER, "RightShoulder"),
        (XINPUT_GAMEPAD_BACK, "Select"),
        (XINPUT_GAMEPAD_START, "Start"),
        (XINPUT_GAMEPAD_LEFT_THUMB, "LeftThumb"),
        (XINPUT_GAMEPAD_RIGHT_THUMB, "RightThumb"),
        (XINPUT_GAMEPAD_DPAD_UP, "DPadUp"),
        (XINPUT_GAMEPAD_DPAD_DOWN, "DPadDown"),
        (XINPUT_GAMEPAD_DPAD_LEFT, "DPadLeft"),
        (XINPUT_GAMEPAD_DPAD_RIGHT, "DPadRight"),
    ];
}

#[cfg(not(windows))]
mod imp {
    use super::super::JoyKeyEngine;
    #[derive(Default)]
    pub struct XInputJoyKeyPoller;
    impl XInputJoyKeyPoller {
        pub fn poll(&mut self, _engine: &mut JoyKeyEngine) {}
    }
}

pub use imp::XInputJoyKeyPoller;
