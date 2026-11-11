pub fn parse_virtual_key(value: &str) -> Option<u16> {
    let key = value.trim().to_ascii_uppercase();
    if key.len() == 1 {
        let c = key.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some(c as u16);
        }
    }
    if let Some(num) = key.strip_prefix('F').and_then(|s| s.parse::<u16>().ok()) {
        if (1..=24).contains(&num) {
            return Some(0x70 + num - 1);
        }
    }
    Some(match key.as_str() {
        "SPACE" => 0x20,
        "ENTER" | "RETURN" => 0x0D,
        "ESC" | "ESCAPE" => 0x1B,
        "TAB" => 0x09,
        "BACKSPACE" => 0x08,
        "SHIFT" => 0x10,
        "CTRL" | "CONTROL" => 0x11,
        "ALT" => 0x12,
        "LEFT" => 0x25,
        "UP" => 0x26,
        "RIGHT" => 0x27,
        "DOWN" => 0x28,
        "HOME" => 0x24,
        "END" => 0x23,
        "PAGEUP" | "PGUP" => 0x21,
        "PAGEDOWN" | "PGDN" => 0x22,
        "INSERT" | "INS" => 0x2D,
        "DELETE" | "DEL" => 0x2E,
        "CAPSLOCK" => 0x14,
        "NUMLOCK" => 0x90,
        "LWIN" | "WIN" => 0x5B,
        _ => return None,
    })
}

#[cfg(windows)]
pub fn send_key(vk: u16, pressed: bool) {
    use std::mem::size_of;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    };

    let flags = if pressed { 0 } else { KEYEVENTF_KEYUP };
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        let _ = SendInput(1, &input, size_of::<INPUT>() as i32);
    }
}

#[cfg(not(windows))]
pub fn send_key(_vk: u16, _pressed: bool) {}
