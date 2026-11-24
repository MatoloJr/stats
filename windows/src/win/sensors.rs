//! Reading the machine. Everything here works without administrator rights.
//!
//! - CPU: `GetSystemTimes` deltas.
//! - RAM and commit: `GlobalMemoryStatusEx` and `GetPerformanceInfo`.
//! - Disks, GPU, network, temperature, WSL VM: Performance Data Helper (PDH)
//!   counters added by their English names, so they work on any Windows
//!   display language. A counter that does not exist on this machine
//!   (no thermal zone, an old GPU driver) just reads `n/a`.
//! - Refresh rate: `EnumDisplaySettingsW`, re-read on display changes.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Performance::*;
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows_sys::Win32::System::Threading::GetSystemTimes;

use super::util::{free_space, from_wide, wide};
use stats_core::counters::gpu_busy;
use stats_core::numbers::{counter_delta, percent};

const PDH_OK: u32 = 0;
const PDH_MORE_DATA: u32 = 0x8000_07D2;
/// Do not cap percentages at 100 (multi-core and summed counters exceed it).
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;

/// One PDH query holding every counter Stats reads each poll. One
/// `PdhCollectQueryData` call per poll updates them all.
struct Pdh {
    query: isize,
    disk_read: Option<isize>,
    disk_write: Option<isize>,
    net_down: Option<isize>,
    net_up: Option<isize>,
    gpu_engines: Option<isize>,
    gpu_dedicated: Option<isize>,
    thermal: Option<isize>,
    vm: Vec<isize>,
}

impl Pdh {
    fn open() -> Option<Self> {
        let mut query = 0isize;
        if unsafe { PdhOpenQueryW(null(), 0, &mut query) } != PDH_OK {
            return None;
        }
        let add = |path: &str| -> Option<isize> {
            let mut counter = 0isize;
            let path = wide(path);
            (unsafe { PdhAddEnglishCounterW(query, path.as_ptr(), 0, &mut counter) } == PDH_OK).then_some(counter)
        };
        let pdh = Pdh {
            query,
            disk_read: add(r"\PhysicalDisk(_Total)\Disk Read Bytes/sec"),
            disk_write: add(r"\PhysicalDisk(_Total)\Disk Write Bytes/sec"),
            net_down: add(r"\Network Interface(*)\Bytes Received/sec"),
            net_up: add(r"\Network Interface(*)\Bytes Sent/sec"),
            gpu_engines: add(r"\GPU Engine(*)\Utilization Percentage"),
            gpu_dedicated: add(r"\GPU Adapter Memory(*)\Dedicated Usage"),
            thermal: add(r"\Thermal Zone Information(*)\Temperature"),
            // Docker Desktop and WSL2 run in this VM; Task Manager names it
            // vmmem or VmmemWSL depending on the Windows build.
            vm: [r"\Process(vmmem)\Working Set - Private", r"\Process(VmmemWSL)\Working Set - Private"]
                .iter()
                .filter_map(|p| add(p))
                .collect(),
        };
        // Rate counters need two collections before they have a value.
        unsafe { PdhCollectQueryData(query) };
        Some(pdh)
    }

    fn collect(&self) {
        unsafe { PdhCollectQueryData(self.query) };
    }

    fn value(counter: Option<isize>) -> Option<f64> {
        let counter = counter?;
        let mut value: PDH_FMT_COUNTERVALUE = unsafe { zeroed() };
        let status =
            unsafe { PdhGetFormattedCounterValue(counter, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, null_mut(), &mut value) };
        (status == PDH_OK && value.CStatus == PDH_OK).then_some(unsafe { value.Anonymous.doubleValue })
    }

    /// Every instance of a wildcard counter as (instance name, value).
    fn array(counter: Option<isize>) -> Vec<(String, f64)> {
        let Some(counter) = counter else { return Vec::new() };
        let mut bytes = 0u32;
        let mut count = 0u32;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut bytes, &mut count, null_mut())
        };
        if status != PDH_MORE_DATA || bytes == 0 {
            return Vec::new();
        }
        // u64 storage keeps the item array correctly aligned.
        let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
        let items = buffer.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
        let status = unsafe {
            PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, &mut bytes, &mut count, items)
        };
        if status != PDH_OK {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            let item = unsafe { &*items.add(i) };
            if item.FmtValue.CStatus != PDH_OK {
                continue;
            }
            let name = unsafe { wide_ptr_to_string(item.szName) };
            out.push((name, unsafe { item.FmtValue.Anonymous.doubleValue }));
        }
        out
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

unsafe fn wide_ptr_to_string(ptr: *const u16) -> String {
    if ptr.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *ptr.add(len) != 0 && len < 1024 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len))
}

#[derive(Clone, Debug, Default)]
pub struct Drive {
    pub name: String,
    pub used: u64,
    pub total: u64,
    pub percent: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Display {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub hz: u32,
}

/// Everything read in one poll.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub cpu: Option<f64>,
    pub ram_used: u64,
    pub ram_total: u64,
    pub memory: Option<f64>,
    pub commit_used: u64,
    pub commit_limit: u64,
    pub commit: Option<f64>,
    pub physical_total: u64,
    pub drives: Vec<Drive>,
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
    pub net_down: Option<f64>,
    pub net_up: Option<f64>,
    pub gpu: Option<f64>,
    pub gpu_dedicated: Option<u64>,
    pub temperature: Option<f64>,
    /// RAM used by the WSL / Docker virtual machine, when it is running.
    pub vm_bytes: Option<u64>,
}

impl Snapshot {
    /// Page file size implied by the commit limit.
    pub fn pagefile_bytes(&self) -> u64 {
        self.commit_limit.saturating_sub(self.physical_total)
    }

    pub fn disk_io(&self) -> Option<f64> {
        match (self.disk_read, self.disk_write) {
            (Some(r), Some(w)) => Some(r + w),
            (r, w) => r.or(w),
        }
    }
}

pub struct Sensors {
    pdh: Option<Pdh>,
    cpu_prev: Option<(u64, u64)>,
    pub displays: Vec<Display>,
}

fn filetime(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

impl Sensors {
    pub fn new() -> Self {
        let pdh = Pdh::open();
        if pdh.is_none() {
            super::util::log("PDH query could not be opened; disk, GPU, network and temperature will read n/a");
        }
        let mut sensors = Sensors { pdh, cpu_prev: None, displays: Vec::new() };
        sensors.refresh_displays();
        sensors
    }

    fn cpu(&mut self) -> Option<f64> {
        let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) = unsafe { zeroed() };
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        // Kernel time includes idle time.
        let idle = filetime(idle);
        let total = filetime(kernel) + filetime(user);
        let previous = self.cpu_prev.replace((idle, total));
        let (prev_idle, prev_total) = previous?;
        let total_delta = counter_delta(prev_total, total)?;
        let idle_delta = counter_delta(prev_idle, idle)?;
        percent(total_delta.saturating_sub(idle_delta) as f64, total_delta as f64)
    }

    pub fn sample(&mut self, drives: &[String]) -> Snapshot {
        let mut s = Snapshot { cpu: self.cpu(), ..Default::default() };

        let mut status: MEMORYSTATUSEX = unsafe { zeroed() };
        status.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
            s.ram_total = status.ullTotalPhys;
            s.ram_used = status.ullTotalPhys.saturating_sub(status.ullAvailPhys);
            s.memory = percent(s.ram_used as f64, s.ram_total as f64);
        }
        let mut perf: PERFORMANCE_INFORMATION = unsafe { zeroed() };
        if unsafe { GetPerformanceInfo(&mut perf, size_of::<PERFORMANCE_INFORMATION>() as u32) } != 0 {
            let page = perf.PageSize as u64;
            s.commit_used = perf.CommitTotal as u64 * page;
            s.commit_limit = perf.CommitLimit as u64 * page;
            s.physical_total = perf.PhysicalTotal as u64 * page;
            s.commit = percent(s.commit_used as f64, s.commit_limit as f64);
        }

        for name in drives {
            let root = format!("{name}\\");
            if let Some((free, total)) = free_space(&root) {
                let used = total.saturating_sub(free);
                s.drives.push(Drive { name: name.clone(), used, total, percent: percent(used as f64, total as f64) });
            } else {
                s.drives.push(Drive { name: name.clone(), ..Default::default() });
            }
        }

        if let Some(pdh) = &self.pdh {
            pdh.collect();
            s.disk_read = Pdh::value(pdh.disk_read);
            s.disk_write = Pdh::value(pdh.disk_write);
            let sum = |items: Vec<(String, f64)>| -> Option<f64> {
                (!items.is_empty()).then(|| items.iter().map(|(_, v)| v.max(0.0)).sum())
            };
            s.net_down = sum(Pdh::array(pdh.net_down));
            s.net_up = sum(Pdh::array(pdh.net_up));
            s.gpu = gpu_busy(&Pdh::array(pdh.gpu_engines));
            s.gpu_dedicated = sum(Pdh::array(pdh.gpu_dedicated)).map(|v| v as u64);
            // Thermal zones report Kelvin. Many laptops expose none, or only
            // a fixed ACPI value; that is shown as is, or n/a.
            s.temperature = Pdh::array(pdh.thermal)
                .iter()
                .map(|(_, k)| k - 273.15)
                .filter(|c| (1.0..150.0).contains(c))
                .reduce(f64::max);
            let vm: f64 = pdh.vm.iter().filter_map(|c| Pdh::value(Some(*c))).sum();
            s.vm_bytes = (vm > 0.0).then_some(vm as u64);
        }
        s
    }

    /// Monitors and their current refresh rate. Called at start and on
    /// WM_DISPLAYCHANGE, not every poll.
    pub fn refresh_displays(&mut self) {
        let mut displays = Vec::new();
        for index in 0..16u32 {
            let mut device: DISPLAY_DEVICEW = unsafe { zeroed() };
            device.cb = size_of::<DISPLAY_DEVICEW>() as u32;
            if unsafe { EnumDisplayDevicesW(null(), index, &mut device, 0) } == 0 {
                break;
            }
            if device.StateFlags & DISPLAY_DEVICE_ACTIVE == 0 {
                continue;
            }
            let mut mode: DEVMODEW = unsafe { zeroed() };
            mode.dmSize = size_of::<DEVMODEW>() as u16;
            if unsafe { EnumDisplaySettingsW(device.DeviceName.as_ptr(), ENUM_CURRENT_SETTINGS, &mut mode) } == 0 {
                continue;
            }
            // Friendly monitor name is the first child device.
            let mut monitor: DISPLAY_DEVICEW = unsafe { zeroed() };
            monitor.cb = size_of::<DISPLAY_DEVICEW>() as u32;
            let name = if unsafe { EnumDisplayDevicesW(device.DeviceName.as_ptr(), 0, &mut monitor, 0) } != 0 {
                from_wide(&monitor.DeviceString)
            } else {
                from_wide(&device.DeviceName)
            };
            displays.push(Display {
                name,
                width: mode.dmPelsWidth,
                height: mode.dmPelsHeight,
                hz: mode.dmDisplayFrequency,
            });
        }
        self.displays = displays;
    }
}
