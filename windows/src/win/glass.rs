//! Frosted glass: the system acrylic blur behind a window, so what is under
//! the readings strip and the dashboard stays partly visible, blurred.
//!
//! It uses the accent policy that the Windows shell itself uses for acrylic
//! (`SetWindowCompositionAttribute`, looked up at run time because it is not
//! in the SDK headers). It works on Windows 10 1803 and later and on Windows
//! 11. Where it is missing, `enable` returns false and the window is simply
//! drawn solid, as before.
//!
//! With the accent on, DWM blends the window's pixels using their alpha, so
//! `Canvas` paints into a 32-bit bitmap and then gives background pixels a
//! partial alpha (see `Canvas::begin_glass`); text and controls stay opaque.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{BOOL, HWND};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

use super::util::wide;

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> BOOL;

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

fn set_composition() -> Option<SetCompositionAttribute> {
    static FUNC: OnceLock<Option<usize>> = OnceLock::new();
    let addr = (*FUNC.get_or_init(|| unsafe {
        let user32 = GetModuleHandleW(wide("user32.dll").as_ptr());
        if user32.is_null() {
            return None;
        }
        GetProcAddress(user32, c"SetWindowCompositionAttribute".as_ptr() as *const u8).map(|f| f as usize)
    }))?;
    // SAFETY: the address came from GetProcAddress for this exact export.
    Some(unsafe { std::mem::transmute::<usize, SetCompositionAttribute>(addr) })
}

/// Dark title colours and rounded corners (Windows 11), plus the acrylic
/// blur. Returns whether the blur is on.
pub fn enable(hwnd: HWND) -> bool {
    unsafe {
        let dark: BOOL = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const _ as *const c_void, 4);
        let corner = DWMWCP_ROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const _ as *const c_void, 4);
    }
    let Some(set) = set_composition() else { return false };
    // A whisper of tint from the accent itself; the real tint is painted.
    let mut accent =
        AccentPolicy { state: ACCENT_ENABLE_ACRYLICBLURBEHIND, flags: 0, gradient: 0x0117_1913, animation: 0 };
    let mut data = CompositionData {
        attribute: WCA_ACCENT_POLICY,
        data: &mut accent as *mut _ as *mut c_void,
        size: std::mem::size_of::<AccentPolicy>(),
    };
    unsafe { set(hwnd, &mut data) != 0 }
}
