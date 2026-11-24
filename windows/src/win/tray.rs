//! Notification-area icon, its tooltip, and balloon notifications.

use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{HICON, WM_APP};

use super::util::fill_wide;

pub const WM_APP_TRAY: u32 = WM_APP + 1;
const ID: u32 = 1;

fn base(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = ID;
    data
}

/// Add the icon. Called at start and again when Explorer restarts
/// (TaskbarCreated), which otherwise leaves the icon gone.
pub fn add(hwnd: HWND, icon: HICON, tip: &str) -> bool {
    let mut data = base(hwnd);
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
    data.uCallbackMessage = WM_APP_TRAY;
    data.hIcon = icon;
    fill_wide(&mut data.szTip, tip);
    let added = unsafe { Shell_NotifyIconW(NIM_ADD, &data) } != 0;
    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) };
    added
}

pub fn update(hwnd: HWND, icon: Option<HICON>, tip: &str) {
    let mut data = base(hwnd);
    data.uFlags = NIF_TIP | NIF_SHOWTIP;
    if let Some(icon) = icon {
        data.uFlags |= NIF_ICON;
        data.hIcon = icon;
    }
    fill_wide(&mut data.szTip, tip);
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

pub fn balloon(hwnd: HWND, title: &str, body: &str, critical: bool) {
    let mut data = base(hwnd);
    data.uFlags = NIF_INFO;
    fill_wide(&mut data.szInfoTitle, title);
    fill_wide(&mut data.szInfo, body);
    // Critical warnings show even during Windows' first-hour quiet time.
    data.dwInfoFlags = if critical { NIIF_ERROR } else { NIIF_WARNING | NIIF_RESPECT_QUIET_TIME };
    unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) };
}

pub fn remove(hwnd: HWND) {
    let data = base(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &data) };
}
