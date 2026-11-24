//! Per-process memory and CPU, read only when needed: when a warning has
//! held for three polls, or every few seconds while the dashboard is open.

use std::collections::HashMap;
use std::mem::{size_of, zeroed};
use std::time::Instant;

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HWND, INVALID_HANDLE_VALUE, LPARAM, WPARAM};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::util::from_wide;
use stats_core::pressure::{group, Group};

#[derive(Clone, Copy)]
struct Sample {
    cpu_100ns: u64,
}

/// Remembers CPU time per process between scans to turn it into a share.
pub struct ProcessScanner {
    previous: HashMap<u32, Sample>,
    previous_at: Option<Instant>,
    cores: f64,
}

fn filetime(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

impl ProcessScanner {
    pub fn new() -> Self {
        let cores = std::thread::available_parallelism().map(|n| n.get() as f64).unwrap_or(1.0);
        Self { previous: HashMap::new(), previous_at: None, cores }
    }

    /// One pass over all processes, grouped by program. CPU shares are
    /// relative to the previous scan; on the first scan they are zero.
    pub fn scan(&mut self) -> Vec<Group> {
        let now = Instant::now();
        let elapsed_100ns =
            self.previous_at.map(|at| now.duration_since(at).as_nanos() as f64 / 100.0).filter(|e| *e > 0.0);
        let mut current = HashMap::with_capacity(self.previous.len().max(256));
        let mut rows = Vec::with_capacity(256);

        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
            return Vec::new();
        }
        let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        while more {
            let pid = entry.th32ProcessID;
            let name = from_wide(&entry.szExeFile);
            if pid != 0 {
                let (private_bytes, cpu_100ns) = read_process(pid);
                let cpu = match (elapsed_100ns, cpu_100ns, self.previous.get(&pid)) {
                    (Some(elapsed), Some(now_cpu), Some(prev)) => {
                        (now_cpu.saturating_sub(prev.cpu_100ns) as f64 / elapsed / self.cores * 100.0).clamp(0.0, 100.0)
                    }
                    _ => 0.0,
                };
                if let Some(cpu_100ns) = cpu_100ns {
                    current.insert(pid, Sample { cpu_100ns });
                }
                rows.push((pid, name, private_bytes.unwrap_or(0), cpu));
            }
            more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        self.previous = current;
        self.previous_at = Some(now);
        group(rows)
    }
}

fn read_process(pid: u32) -> (Option<u64>, Option<u64>) {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return (None, None);
    }
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { zeroed() };
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let memory =
        (unsafe { GetProcessMemoryInfo(handle, &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, counters.cb) }
            != 0)
            .then_some(counters.PrivateUsage as u64);
    let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = unsafe { zeroed() };
    let cpu = (unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0)
        .then(|| filetime(kernel) + filetime(user));
    unsafe { CloseHandle(handle) };
    (memory, cpu)
}

struct CloseSearch<'a> {
    pids: &'a [u32],
    closed: u32,
}

unsafe extern "system" fn close_windows_of(hwnd: HWND, lparam: LPARAM) -> i32 {
    let search = &mut *(lparam as *mut CloseSearch);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    // Only visible top-level windows with no owner: the app's main windows.
    if search.pids.contains(&pid) && IsWindowVisible(hwnd) != 0 && GetWindow(hwnd, GW_OWNER).is_null() {
        PostMessageW(hwnd, WM_CLOSE, 0 as WPARAM, 0);
        search.closed += 1;
    }
    1
}

/// Ask a program to close the normal way (as if you clicked its X), so it
/// can save or prompt. Returns how many windows were asked.
pub fn close_gracefully(pids: &[u32]) -> u32 {
    let mut search = CloseSearch { pids, closed: 0 };
    unsafe { EnumWindows(Some(close_windows_of), &mut search as *mut _ as LPARAM) };
    search.closed
}

/// Processes from `pids` still running.
pub fn still_running(pids: &[u32]) -> Vec<u32> {
    pids.iter()
        .copied()
        .filter(|&pid| {
            let handle: HANDLE = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                return false;
            }
            let alive = unsafe { WaitForSingleObject(handle, 0) } == windows_sys::Win32::Foundation::WAIT_TIMEOUT;
            unsafe { CloseHandle(handle) };
            alive
        })
        .collect()
}

/// Force-end processes. Only after the person confirmed it.
pub fn terminate(pids: &[u32]) -> (u32, u32) {
    let (mut ended, mut failed) = (0, 0);
    for &pid in pids {
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if !handle.is_null() && unsafe { TerminateProcess(handle, 1) } != 0 {
            ended += 1;
        } else {
            failed += 1;
        }
        if !handle.is_null() {
            unsafe { CloseHandle(handle) };
        }
    }
    (ended, failed)
}

/// Is a process with this executable name running? (e.g. Docker Desktop)
pub fn is_running(exe_lower: &str) -> bool {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return false;
    }
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    let mut found = false;
    while more && !found {
        found = from_wide(&entry.szExeFile).to_ascii_lowercase() == exe_lower;
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    unsafe { CloseHandle(snapshot) };
    found
}
