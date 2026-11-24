//! Stats for Windows: a light floating system monitor that warns before
//! memory or storage runs out, names the program responsible, and offers a
//! one-click fix.

#![windows_subsystem = "windows"]

mod win;

use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use win::app::{self, MAIN_CLASS, WM_APP_SHOW_DASH};
use win::util::{log, wide};

/// Writes serious native faults to the log before Windows ends the process,
/// so a crash report says where it happened. (First-chance handler: it only
/// records, it never handles.)
unsafe extern "system" fn log_fault(
    info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS,
) -> i32 {
    const CONTINUE_SEARCH: i32 = 0;
    let Some(record) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) else { return CONTINUE_SEARCH };
    let code = record.ExceptionCode as u32;
    // Access violation, stack overflow, illegal instruction, fast-fail.
    if matches!(code, 0xC000_0005 | 0xC000_00FD | 0xC000_001D | 0xC000_0409) {
        let base = GetModuleHandleW(std::ptr::null()) as usize;
        let at = record.ExceptionAddress as usize;
        let detail = if code == 0xC000_0005 && record.NumberParameters >= 2 {
            let kind = match record.ExceptionInformation[0] {
                0 => "read",
                1 => "write",
                _ => "execute",
            };
            format!(" ({kind} of {:#x})", record.ExceptionInformation[1])
        } else {
            String::new()
        };
        log(&format!("fault {code:#x} at {at:#x} (stats.exe+{:#x}){detail}", at.wrapping_sub(base)));
        if cfg!(debug_assertions) && code != 0xC000_00FD {
            log(&format!("{}", std::backtrace::Backtrace::force_capture()));
        }
    }
    CONTINUE_SEARCH
}

fn main() {
    std::panic::set_hook(Box::new(|info| log(&format!("panic: {info}"))));
    unsafe { windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(log_fault)) };

    // One copy only. Starting it again opens the running copy's dashboard.
    let mutex_name = wide("Local\\StatsForWindows.SingleInstance");
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
    if mutex.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let class = wide(MAIN_CLASS);
        let existing = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) };
        if !existing.is_null() {
            unsafe { PostMessageW(existing, WM_APP_SHOW_DASH, 0, 0) };
        }
        return;
    }

    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let instance = unsafe { GetModuleHandleW(std::ptr::null()) };
    log(&format!("Stats {} starting", env!("CARGO_PKG_VERSION")));
    if let Err(e) = app::create(instance) {
        log(&format!("startup failed: {e}"));
        let text = wide(&format!("Stats could not start: {e}"));
        let title = wide("Stats");
        unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONERROR) };
        return;
    }

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while unsafe { GetMessageW(&mut msg, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    log("Stats exited");
}
