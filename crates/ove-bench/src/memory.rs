//! Peak resident memory of this process.

#[cfg(target_os = "linux")]
pub fn peak_bytes() -> u64 {
    // VmHWM: peak resident set size.
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|kb| kb.parse::<u64>().ok())
        })
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

#[cfg(all(unix, not(target_os = "linux")))]
pub fn peak_bytes() -> u64 {
    // SAFETY: getrusage fills a caller-provided struct.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut ru) != 0 {
            return 0;
        }
        // macOS reports bytes; the BSDs report kilobytes.
        if cfg!(target_os = "macos") { ru.ru_maxrss as u64 } else { ru.ru_maxrss as u64 * 1024 }
    }
}

#[cfg(windows)]
pub fn peak_bytes() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: the counters struct is caller-provided with its size set.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
            return 0;
        }
        c.PeakWorkingSetSize as u64
    }
}
