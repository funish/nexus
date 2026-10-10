//! Host resource discovery used to derive defaults instead of assuming a
//! particular deployment size.

/// Logical CPUs visible to this process. Rust's implementation accounts for
/// CPU affinity and Linux cgroup CPU quotas where available.
pub fn cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
}

/// Total memory in bytes. Inside Docker or systemd this prefers the effective
/// cgroup limit over the host total, so a small container does not inherit a
/// large host's memory-derived concurrency.
pub fn total_memory_bytes() -> Option<u64> {
    total_memory_for_target()
}

#[cfg(target_os = "linux")]
fn total_memory_for_target() -> Option<u64> {
    let host = proc_memory_bytes();
    let limit = cgroup_memory_limit();
    match (host, limit) {
        (Some(host), Some(limit)) => Some(host.min(limit)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(target_os = "linux")]
fn proc_memory_bytes() -> Option<u64> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = contents
        .lines()
        .find(|line| line.starts_with("MemTotal:"))?;
    let kilobytes = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    (kilobytes > 0).then_some(kilobytes.saturating_mul(1024))
}

#[cfg(target_os = "linux")]
fn cgroup_memory_limit() -> Option<u64> {
    read_cgroup_limit("/sys/fs/cgroup/memory.max")
        .or_else(|| read_cgroup_limit("/sys/fs/cgroup/memory/memory.limit_in_bytes"))
}

#[cfg(target_os = "linux")]
fn read_cgroup_limit(path: &str) -> Option<u64> {
    let value = std::fs::read_to_string(path).ok()?.trim().to_string();
    if value == "max" {
        return None;
    }
    let limit = value.parse::<u64>().ok()?;
    // cgroup v1 reports an unlimited controller as u64::MAX (or i64::MAX when
    // interpreted as signed). Those values are not a usable budget.
    let unlimited_by_signedness = limit > i64::MAX as u64;
    (limit > 0 && !unlimited_by_signedness).then_some(limit)
}

#[cfg(target_os = "macos")]
fn total_memory_for_target() -> Option<u64> {
    let mut value: u64 = 0;
    let mut length = std::mem::size_of::<u64>();
    let name = b"hw.memsize\0";
    let result = unsafe {
        libc::sysctlbyname(
            name.as_ptr().cast(),
            std::ptr::addr_of_mut!(value).cast(),
            std::ptr::addr_of_mut!(length),
            std::ptr::null_mut(),
            0,
        )
    };
    (result == 0 && value > 0).then_some(value)
}

#[cfg(target_os = "windows")]
fn total_memory_for_target() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..MEMORYSTATUSEX::default()
    };
    (unsafe { GlobalMemoryStatusEx(&mut status) } != 0 && status.ullTotalPhys > 0)
        .then_some(status.ullTotalPhys)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn total_memory_for_target() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::{cpu_count, total_memory_bytes};

    #[test]
    fn cpu_count_is_usable() {
        assert!(cpu_count() >= 1);
    }

    #[test]
    fn memory_detection_returns_a_positive_budget_when_supported() {
        if cfg!(any(
            target_os = "linux",
            target_os = "macos",
            target_os = "windows"
        )) {
            assert!(total_memory_bytes().is_some_and(|bytes| bytes > 0));
        }
    }
}
