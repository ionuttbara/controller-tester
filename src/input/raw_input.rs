//! Win32 Raw Input (`WM_INPUT`) transport.
//!
//! This deliberately lives beside the direct HID reader instead of replacing it.
//! Direct HID is ideal when a shared endpoint can be opened because it also gives
//! us the report descriptor. Raw Input is the Windows-native fallback transport
//! for drivers/devices that deliver HID reports to a window but reject a second
//! shared HID open.

#[derive(Debug, Clone, Default)]
pub struct RawInputPacket {
    pub device_path: String,
    pub report: Vec<u8>,
    pub report_size: u32,
    pub report_count: u32,
}

#[cfg(windows)]
mod imp {
    use super::RawInputPacket;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use std::mem::{size_of, zeroed};
    use std::ptr::{null, null_mut};
    use std::sync::{Mutex, OnceLock};
    use std::thread::{self, JoinHandle};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, GetRawInputDeviceInfoW, RegisterRawInputDevices, HRAWINPUT, RAWHID,
        RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_DEVNOTIFY, RIDEV_INPUTSINK, RIDEV_PAGEONLY,
        RID_INPUT, RIDI_DEVICENAME, RIM_TYPEHID,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, CS_HREDRAW, CS_VREDRAW,
        HWND_MESSAGE, MSG, WM_CLOSE, WM_DESTROY, WM_INPUT, WNDCLASSW,
    };

    const CLASS_NAME: &[u16] = &[
        'I' as u16, 'B' as u16, 'B' as u16, 'E' as u16, '_' as u16, 'R' as u16,
        'a' as u16, 'w' as u16, 'I' as u16, 'n' as u16, 'p' as u16, 'u' as u16,
        't' as u16, '_' as u16, 'v' as u16, '0' as u16, '5' as u16, 0,
    ];

    static PACKET_SENDER: OnceLock<Mutex<Option<Sender<RawInputPacket>>>> = OnceLock::new();

    fn packet_sender() -> &'static Mutex<Option<Sender<RawInputPacket>>> {
        PACKET_SENDER.get_or_init(|| Mutex::new(None))
    }

    pub struct RawInputMonitor {
        rx: Receiver<RawInputPacket>,
        hwnd: usize,
        worker: Option<JoinHandle<()>>,
    }

    impl RawInputMonitor {
        pub fn start() -> anyhow::Result<Self> {
            let (packet_tx, packet_rx) = unbounded();
            let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<usize, String>>(1);

            let worker = thread::Builder::new()
                .name("ibbe-wm-input".to_string())
                .spawn(move || unsafe {
                    if let Ok(mut slot) = packet_sender().lock() {
                        *slot = Some(packet_tx);
                    }
                    let result = create_window_and_register();
                    match result {
                        Ok(hwnd) => {
                            let _ = ready_tx.send(Ok(hwnd as usize));
                            let mut msg: MSG = zeroed();
                            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                                TranslateMessage(&msg);
                                DispatchMessageW(&msg);
                            }
                            if let Ok(mut slot) = packet_sender().lock() { *slot = None; }
                        }
                        Err(err) => {
                            let _ = ready_tx.send(Err(err));
                            if let Ok(mut slot) = packet_sender().lock() { *slot = None; }
                        }
                    }
                })?;

            match ready_rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(hwnd)) => Ok(Self { rx: packet_rx, hwnd, worker: Some(worker) }),
                Ok(Err(err)) => {
                    let _ = worker.join();
                    Err(anyhow::anyhow!(err))
                }
                Err(_) => Err(anyhow::anyhow!("Raw Input window initialization timed out")),
            }
        }

        /// Drain the queue and return the newest packet. Keeping only the latest
        /// packet bounds work even for 1 kHz devices.
        pub fn latest(&self) -> Option<RawInputPacket> {
            let mut latest = None;
            while let Ok(packet) = self.rx.try_recv() {
                latest = Some(packet);
            }
            latest
        }
    }

    impl Drop for RawInputMonitor {
        fn drop(&mut self) {
            if self.hwnd != 0 {
                unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0); }
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    unsafe fn create_window_and_register() -> Result<HWND, String> {
        let hinstance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(window_proc),
            hInstance: hinstance,
            lpszClassName: CLASS_NAME.as_ptr(),
            ..zeroed()
        };
        RegisterClassW(&class);

        let hwnd = CreateWindowExW(
            0,
            CLASS_NAME.as_ptr(),
            CLASS_NAME.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            null_mut(),
            hinstance,
            null(),
        );
        if hwnd.is_null() {
            return Err("CreateWindowExW failed for Raw Input message window".to_string());
        }

        let flags = RIDEV_INPUTSINK | RIDEV_DEVNOTIFY;
        let devices = [
            RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x04, dwFlags: flags, hwndTarget: hwnd }, // Joystick
            RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x05, dwFlags: flags, hwndTarget: hwnd }, // Gamepad
            RAWINPUTDEVICE { usUsagePage: 0x01, usUsage: 0x08, dwFlags: flags, hwndTarget: hwnd }, // Multi-axis
            // Entire Simulation Controls page: flight controls, pedals, steering, etc.
            RAWINPUTDEVICE { usUsagePage: 0x02, usUsage: 0x00, dwFlags: flags | RIDEV_PAGEONLY, hwndTarget: hwnd },
        ];
        if RegisterRawInputDevices(devices.as_ptr(), devices.len() as u32, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            DestroyWindow(hwnd);
            return Err("RegisterRawInputDevices failed".to_string());
        }
        Ok(hwnd)
    }

    unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        match msg {
            WM_INPUT => {
                if let Some(packet) = read_packet(lparam as HRAWINPUT) {
                    if let Ok(slot) = packet_sender().lock() {
                        if let Some(tx) = slot.as_ref() {
                            let _ = tx.try_send(packet);
                        }
                    }
                }
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    unsafe fn read_packet(handle: HRAWINPUT) -> Option<RawInputPacket> {
        let mut size = 0u32;
        let header_size = size_of::<RAWINPUTHEADER>() as u32;
        if GetRawInputData(handle, RID_INPUT, null_mut(), &mut size, header_size) == u32::MAX || size < header_size {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        let got = GetRawInputData(handle, RID_INPUT, buffer.as_mut_ptr().cast(), &mut size, header_size);
        if got == u32::MAX || got < header_size {
            return None;
        }

        let header = &*(buffer.as_ptr() as *const RAWINPUTHEADER);
        if header.dwType != RIM_TYPEHID {
            return None;
        }
        let hid_offset = size_of::<RAWINPUTHEADER>();
        if buffer.len() < hid_offset + size_of::<RAWHID>() {
            return None;
        }
        let hid = &*(buffer.as_ptr().add(hid_offset) as *const RAWHID);
        let byte_count = hid.dwSizeHid.saturating_mul(hid.dwCount) as usize;
        if byte_count == 0 {
            return None;
        }
        let data_offset = hid_offset + 8; // dwSizeHid + dwCount; bRawData begins immediately after.
        if data_offset + byte_count > buffer.len() {
            return None;
        }

        Some(RawInputPacket {
            device_path: raw_device_name(header.hDevice),
            report: buffer[data_offset..data_offset + byte_count].to_vec(),
            report_size: hid.dwSizeHid,
            report_count: hid.dwCount,
        })
    }

    unsafe fn raw_device_name(device: *mut core::ffi::c_void) -> String {
        let mut chars = 0u32;
        let _ = GetRawInputDeviceInfoW(device, RIDI_DEVICENAME, null_mut(), &mut chars);
        if chars == 0 {
            return String::new();
        }
        let mut buf = vec![0u16; chars as usize + 1];
        let got = GetRawInputDeviceInfoW(device, RIDI_DEVICENAME, buf.as_mut_ptr().cast(), &mut chars);
        if got == u32::MAX {
            return String::new();
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    }
}

#[cfg(windows)]
pub use imp::RawInputMonitor;

#[cfg(not(windows))]
pub struct RawInputMonitor;

#[cfg(not(windows))]
impl RawInputMonitor {
    pub fn start() -> anyhow::Result<Self> { Ok(Self) }
    pub fn latest(&self) -> Option<RawInputPacket> { None }
}
