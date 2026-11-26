//! Operating system hooks: background thread priority, physical memory size and power state.

/// Lowers the calling thread to background priority: Windows background mode (lower CPU, I/O and memory priority);
/// macOS quality-of-service class "utility". Returns whether the system accepted it.
pub fn enter_background() -> bool {
    imp::enter_background()
}

/// Whether the calling thread currently runs at background priority (read back from the system).
pub fn is_background() -> bool {
    imp::is_background()
}

/// Physical memory in bytes, when the system reports it.
pub fn physical_memory() -> Option<u64> {
    imp::physical_memory()
}

/// What the power and thermal sources currently say. `None` fields are unknown on this system.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PowerState {
    pub on_battery: Option<bool>,
    pub low_power: Option<bool>,
    pub thermal_pressure: Option<bool>,
}

/// Reads the current power source, low-power mode and thermal state.
pub fn power_state() -> PowerState {
    imp::power_state()
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadInformation, MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_NORMAL, SetThreadPriority,
        THREAD_MODE_BACKGROUND_BEGIN, ThreadMemoryPriority,
    };

    use super::PowerState;

    pub fn enter_background() -> bool {
        // SAFETY: GetCurrentThread returns a pseudo-handle for the calling thread; no other preconditions.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) != 0 }
    }

    pub fn is_background() -> bool {
        // Background mode lowers the thread's memory priority below normal (with its CPU and I/O priority); the
        // memory priority is the part the system reports back.
        let mut info = MEMORY_PRIORITY_INFORMATION { MemoryPriority: MEMORY_PRIORITY_NORMAL };
        // SAFETY: GetCurrentThread is a pseudo-handle; `info` is a valid buffer of the size passed.
        let ok = unsafe {
            GetThreadInformation(
                GetCurrentThread(),
                ThreadMemoryPriority,
                (&mut info as *mut MEMORY_PRIORITY_INFORMATION).cast(),
                std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
            )
        } != 0;
        ok && info.MemoryPriority < MEMORY_PRIORITY_NORMAL
    }

    pub fn physical_memory() -> Option<u64> {
        // SAFETY: an all-zero MEMORYSTATUSEX is a valid value; dwLength is set as the API requires.
        let mut s: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        s.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        // SAFETY: `s` is a valid, initialised struct the call writes into.
        let ok = unsafe { GlobalMemoryStatusEx(&mut s) } != 0;
        ok.then_some(s.ullTotalPhys)
    }

    pub fn power_state() -> PowerState {
        // SAFETY: an all-zero SYSTEM_POWER_STATUS is a valid value for the call to overwrite.
        let mut s: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        // SAFETY: `s` is a valid struct the call writes into.
        if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
            return PowerState::default();
        }
        let on_battery = match s.ACLineStatus {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        };
        // SystemStatusFlag 1: battery saver is on.
        PowerState { on_battery, low_power: Some(s.SystemStatusFlag == 1), thermal_pressure: None }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2_foundation::{NSProcessInfo, NSProcessInfoThermalState};

    use super::PowerState;

    pub fn enter_background() -> bool {
        // SAFETY: sets the calling thread's QoS class; no pointers involved.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) == 0 }
    }

    pub fn is_background() -> bool {
        let mut class = libc::qos_class_t::QOS_CLASS_UNSPECIFIED;
        let mut relative: libc::c_int = 0;
        // SAFETY: pthread_self is the calling thread; both out-pointers are valid locals.
        let ok = unsafe { libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut relative) } == 0;
        ok && matches!(class, libc::qos_class_t::QOS_CLASS_UTILITY | libc::qos_class_t::QOS_CLASS_BACKGROUND)
    }

    pub fn physical_memory() -> Option<u64> {
        let mut size: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        // SAFETY: the name is NUL-terminated; `size` and `len` are valid out-pointers of the advertised length.
        let r = unsafe {
            libc::sysctlbyname(
                c"hw.memsize".as_ptr(),
                (&mut size as *mut u64).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (r == 0 && size > 0).then_some(size)
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        /// Seconds of battery left; -2.0 (unlimited) when on external power, -1.0 while still estimating.
        fn IOPSGetTimeRemainingEstimate() -> f64;
    }

    pub fn power_state() -> PowerState {
        // SAFETY: takes no arguments and only reads power-source state.
        let remaining = unsafe { IOPSGetTimeRemainingEstimate() };
        let on_battery = Some(remaining != -2.0);
        let info = NSProcessInfo::processInfo();
        let thermal = info.thermalState();
        PowerState {
            on_battery,
            low_power: Some(info.isLowPowerModeEnabled()),
            thermal_pressure: Some(
                thermal == NSProcessInfoThermalState::Serious || thermal == NSProcessInfoThermalState::Critical,
            ),
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::PowerState;

    pub fn enter_background() -> bool {
        false
    }

    pub fn is_background() -> bool {
        false
    }

    pub fn physical_memory() -> Option<u64> {
        None
    }

    pub fn power_state() -> PowerState {
        PowerState::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies the priority change by reading it back from the system on a fresh thread.
    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn background_priority_reads_back() {
        let (before, entered, after) = std::thread::spawn(|| {
            let before = is_background();
            let entered = enter_background();
            (before, entered, is_background())
        })
        .join()
        .unwrap();
        assert!(!before, "a new thread starts at normal priority");
        assert!(entered, "the system accepted background priority");
        assert!(after, "the thread reads back as background");
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn physical_memory_is_known() {
        let m = physical_memory().unwrap();
        assert!(m >= 1024 * 1024 * 1024, "{m}");
    }

    #[test]
    fn power_state_reads() {
        // Values depend on the machine; reading must not fail or panic.
        let _ = power_state();
    }
}
