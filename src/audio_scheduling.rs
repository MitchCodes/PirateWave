//! Register only audio-working threads with Windows MMCSS. No system settings
//! change; the calling thread's registration is released on stop/exit.
#[cfg(windows)]
pub struct AudioThread(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl AudioThread {
    pub fn register() -> Self {
        use windows_sys::Win32::System::Threading::{
            AvSetMmThreadCharacteristicsW, AvSetMmThreadPriority, AVRT_PRIORITY_NORMAL,
        };
        let mut index = 0;
        let task: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
        let handle = unsafe { AvSetMmThreadCharacteristicsW(task.as_ptr(), &mut index) };
        if !handle.is_null() {
            unsafe {
                AvSetMmThreadPriority(handle, AVRT_PRIORITY_NORMAL);
            }
        }
        Self(handle)
    }
}
#[cfg(windows)]
impl Drop for AudioThread {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                windows_sys::Win32::System::Threading::AvRevertMmThreadCharacteristics(self.0);
            }
        }
    }
}
#[cfg(not(windows))]
pub struct AudioThread;
#[cfg(not(windows))]
impl AudioThread {
    pub fn register() -> Self {
        Self
    }
}
