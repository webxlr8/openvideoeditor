//! Free disk space for the volume holding a path.

use std::io;
use std::path::Path;

/// Bytes available to this user on the volume holding `path` (the nearest existing ancestor is asked).
pub fn free_bytes(path: &Path) -> io::Result<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = probe.parent().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no existing parent folder"))?;
    }
    platform_free(probe)
}

#[cfg(windows)]
fn platform_free(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path that outlives the call; the out-pointer is a valid u64 and the
    // two optional out-pointers are null, which the API allows.
    let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(available) }
}

#[cfg(unix)]
fn platform_free(path: &Path) -> io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))?;
    // SAFETY: zeroed is a valid bit pattern for the plain C struct `statvfs`.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a NUL-terminated path that outlives the call; `s` is a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(clippy::unnecessary_cast)] // The field types differ between macOS and Linux.
    Ok(s.f_bavail as u64 * s.f_frsize as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reports_some_free_space_for_a_missing_child_folder() {
        let dir = tempfile::tempdir().unwrap();
        let free = super::free_bytes(&dir.path().join("not/yet/created")).unwrap();
        assert!(free > 0);
    }
}
