#[cfg(windows)]
mod imp {
    use anyhow::{anyhow, Result};
    use std::ptr::null;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, SetLastError, HANDLE};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    const ERROR_ALREADY_EXISTS: u32 = 183;

    pub struct AgentInstanceGuard {
        handle: HANDLE,
    }

    impl Drop for AgentInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe { CloseHandle(self.handle); }
            }
        }
    }

    pub fn try_acquire() -> Result<Option<AgentInstanceGuard>> {
        let name = "Local\\GalleryInc.IBBE-ControllerManager.JoyKeyAgent\0"
            .encode_utf16()
            .collect::<Vec<_>>();
        unsafe { SetLastError(0); }
        let handle = unsafe { CreateMutexW(null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(anyhow!("CreateMutexW failed with Win32 error {}", unsafe { GetLastError() }));
        }
        let last_error = unsafe { GetLastError() };
        if last_error == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle); }
            return Ok(None);
        }
        Ok(Some(AgentInstanceGuard { handle }))
    }
}

#[cfg(not(windows))]
mod imp {
    use anyhow::Result;
    pub struct AgentInstanceGuard;
    pub fn try_acquire() -> Result<Option<AgentInstanceGuard>> {
        Ok(Some(AgentInstanceGuard))
    }
}

pub use imp::try_acquire;
