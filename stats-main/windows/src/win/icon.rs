//! The tray icon is drawn at runtime: three bars whose colour follows the
//! worst level (green, amber, red), so the tray alone tells you when
//! memory is getting tight.

use windows_sys::Win32::Graphics::Gdi::{CreateBitmap, DeleteObject};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconIndirect, DestroyIcon, GetSystemMetrics, HICON, ICONINFO, SM_CXSMICON,
};

use stats_core::config::Level;

fn bgra(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32
}

/// Build an icon at the small-icon size for the current DPI. The caller owns
/// it and must `destroy` it.
pub fn make(level: Level, fill: f64) -> HICON {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.clamp(16, 64);
    let (r, g, b) = match level {
        Level::Normal => (96, 208, 140),
        Level::Warning => (255, 184, 76),
        Level::Critical => (255, 92, 92),
    };
    let n = size as usize;
    let mut pixels = vec![0u32; n * n];
    // Three bars; the middle one shows how full committed memory is.
    let bar_w = (n / 5).max(2);
    let gap = (n - 3 * bar_w) / 4;
    let heights = [0.45, fill.clamp(0.15, 1.0), 0.7];
    for (i, h) in heights.iter().enumerate() {
        let x0 = gap + i * (bar_w + gap);
        let bar_h = ((n as f64 - 2.0) * h) as usize;
        for y in (n - 1 - bar_h)..(n - 1) {
            for x in x0..(x0 + bar_w).min(n) {
                pixels[y * n + x] = if i == 1 { bgra(r, g, b, 255) } else { bgra(200, 205, 215, 255) };
            }
        }
    }
    unsafe {
        let color = CreateBitmap(size, size, 1, 32, pixels.as_ptr() as *const _);
        let mask_bits = vec![0u8; n.div_ceil(16) * 2 * n];
        let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as *const _);
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

pub fn destroy(icon: HICON) {
    if !icon.is_null() {
        unsafe { DestroyIcon(icon) };
    }
}
