//! The floating bar: a slim always-on-top strip, by default just above the
//! taskbar on the right. Click it for the dashboard, drag it anywhere
//! (the position is saved), right-click for the menu.

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::app::{guarded, save_setting, show_menu, with_app, App};
use super::draw::{self, Canvas, Fonts, MeasureDc};
use super::util::wide;
use stats_core::config::{Level, Metric};
use stats_core::fit::{fit, Detail, Segment};
use stats_core::numbers::{fmt_percent, fmt_rate, fmt_temp};

const CLASS: &str = "StatsForWindowsBar";

/// One metric as drawn: muted name, coloured value, muted speed.
#[derive(Clone, Debug, Default)]
pub struct Piece {
    pub name: String,
    pub value: String,
    pub speed: String,
    pub level: Option<Level>,
    pub keep: u8,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub pieces: Vec<Piece>,
    pub detail_full: bool,
}

pub fn create(instance: HINSTANCE) -> Result<HWND, String> {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(bar_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("Stats bar").as_ptr(),
            WS_POPUP,
            0,
            0,
            10,
            10,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err("could not create the bar window".into());
    }
    unsafe {
        SetLayeredWindowAttributes(hwnd, 0, 235, LWA_ALPHA);
        let corner = DWMWCP_ROUND;
        // Rounded corners on Windows 11; ignored on Windows 10.
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const _ as *const _, 4);
    }
    Ok(hwnd)
}

fn pieces(app: &App) -> Vec<Piece> {
    let s = &app.snap;
    let c = &app.config;
    let mut out = Vec::new();
    for metric in &c.bar_metrics {
        let piece = match metric {
            Metric::Cpu => Piece {
                name: "CPU".into(),
                value: fmt_percent(s.cpu),
                level: Some(c.cpu.level(s.cpu)),
                keep: 7,
                ..Default::default()
            },
            Metric::Memory => Piece {
                name: "RAM".into(),
                value: fmt_percent(s.memory),
                level: Some(c.memory.level(s.memory)),
                keep: 8,
                ..Default::default()
            },
            Metric::Commit => Piece {
                name: "Commit".into(),
                value: fmt_percent(s.commit),
                level: Some(c.commit.level(s.commit)),
                keep: 9,
                ..Default::default()
            },
            Metric::Storage => {
                let drive = s.drives.first();
                Piece {
                    name: drive.map(|d| d.name.clone()).unwrap_or_else(|| "Disk".into()),
                    value: fmt_percent(drive.and_then(|d| d.percent)),
                    speed: if c.bar_show_speeds { fmt_rate(s.disk_io()) } else { String::new() },
                    level: Some(c.storage.level(drive.and_then(|d| d.percent))),
                    keep: 6,
                }
            }
            Metric::Gpu => Piece {
                name: "GPU".into(),
                value: fmt_percent(s.gpu),
                level: Some(c.gpu.level(s.gpu)),
                keep: 3,
                ..Default::default()
            },
            Metric::Temperature => {
                if s.temperature.is_none() {
                    continue;
                }
                Piece {
                    name: "Temp".into(),
                    value: fmt_temp(s.temperature, c.temperature_unit),
                    level: Some(c.temp.level(s.temperature)),
                    keep: 2,
                    ..Default::default()
                }
            }
            Metric::Network => Piece {
                name: String::new(),
                value: format!("↓ {}", fmt_rate(s.net_down)),
                speed: if c.bar_show_speeds { format!("↑ {}", fmt_rate(s.net_up)) } else { String::new() },
                level: None,
                keep: 4,
            },
            Metric::Refresh => match app.sensors.displays.first() {
                Some(d) if d.hz > 1 => {
                    Piece { name: "Hz".into(), value: d.hz.to_string(), keep: 1, ..Default::default() }
                }
                _ => continue,
            },
        };
        out.push(piece);
    }
    out
}

fn work_area_for(hwnd: HWND) -> RECT {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        info.rcWork
    }
}

/// Measure the pieces, fit them, size and place the window.
pub fn relayout(app: &mut App) {
    // The bar may have been dragged onto a monitor with a different scale.
    let dpi = unsafe { GetDpiForWindow(app.bar) }.max(96);
    if dpi != app.bar_fonts.dpi {
        app.bar_fonts = Fonts::new(dpi);
    }
    let fonts: &Fonts = &app.bar_fonts;
    let pad = fonts.px(10);
    let gap = fonts.px(14);
    let inner = fonts.px(5);
    let height = fonts.px(26);
    let all = pieces(app);
    let dc = MeasureDc::new(fonts.normal);
    let mut segments = Vec::with_capacity(all.len());
    for p in &all {
        dc.select(fonts.normal);
        let name = if p.name.is_empty() { 0 } else { dc.width(&p.name) + inner };
        let speed = if p.speed.is_empty() { 0 } else { dc.width(&p.speed) + inner };
        dc.select(fonts.bold);
        let value = dc.width(&p.value);
        segments.push(Segment { full: name + value + speed, compact: name + value, keep_priority: p.keep });
    }
    drop(dc);
    let work = work_area_for(app.bar);
    let max_width = ((work.right - work.left) as f32 * 0.65) as i32 - 2 * pad;
    let (detail, shown) = fit(&segments, gap, max_width);
    let full = detail == Detail::Full;
    let content: i32 = shown.iter().map(|&i| if full { segments[i].full } else { segments[i].compact }).sum::<i32>()
        + gap * (shown.len() as i32 - 1).max(0);
    let width = content + 2 * pad;
    let layout = Layout { pieces: shown.iter().map(|&i| all[i].clone()).collect(), detail_full: full };

    // Place: saved position (clamped onto a visible screen), else above the
    // taskbar on the right.
    let mut rect: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(app.bar, &mut rect) };
    let margin = fonts.px(8);
    let (x, y) = match (app.config.bar_x, app.config.bar_y) {
        (Some(x), Some(y)) => {
            let probe = POINT { x, y };
            let monitor = unsafe { MonitorFromPoint(probe, MONITOR_DEFAULTTONEAREST) };
            let mut info: MONITORINFO = unsafe { zeroed() };
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            unsafe { GetMonitorInfoW(monitor, &mut info) };
            let m = info.rcMonitor;
            (x.clamp(m.left, (m.right - width).max(m.left)), y.clamp(m.top, (m.bottom - height).max(m.top)))
        }
        _ => (work.right - width - margin, work.bottom - height - margin),
    };
    let moved = rect.left != x || rect.top != y || rect.right - rect.left != width || rect.bottom - rect.top != height;
    app.bar_layout = layout;
    if moved {
        unsafe { SetWindowPos(app.bar, HWND_TOPMOST, x, y, width, height, SWP_NOACTIVATE | SWP_NOOWNERZORDER) };
    }
}

fn paint(app: &App, hwnd: HWND) {
    let fonts = &app.bar_fonts;
    let layout = &app.bar_layout;
    let mut canvas = Canvas::begin(hwnd);
    let full = RECT { left: 0, top: 0, right: canvas.width, bottom: canvas.height };
    canvas.fill(full, draw::BG);
    // Thin top accent in the colour of the worst level.
    let accent = match app.worst_level() {
        Level::Normal => draw::ACCENT,
        level => draw::level_color(level),
    };
    canvas.fill(RECT { left: 0, top: 0, right: canvas.width, bottom: fonts.px(2) }, accent);

    let pad = fonts.px(10);
    let gap = fonts.px(14);
    let inner = fonts.px(5);
    let (top, bottom) = (fonts.px(1), canvas.height);
    let mut x = pad;
    for (i, piece) in layout.pieces.iter().enumerate() {
        if i > 0 {
            let mid = x - gap / 2;
            canvas.fill(
                RECT { left: mid, top: fonts.px(7), right: mid + 1, bottom: canvas.height - fonts.px(6) },
                draw::LINE,
            );
        }
        canvas.font(fonts.normal);
        if !piece.name.is_empty() {
            x = canvas.text(&piece.name, x, top, bottom, draw::MUTED) + inner;
        }
        canvas.font(fonts.bold);
        let color = piece.level.map(draw::level_color).unwrap_or(draw::TEXT);
        x = canvas.text(&piece.value, x, top, bottom, color);
        if layout.detail_full && !piece.speed.is_empty() {
            canvas.font(fonts.normal);
            x = canvas.text(&piece.speed, x + inner, top, bottom, draw::MUTED);
        }
        x += gap;
    }
}

/// Left button: drag to move, or click to open the dashboard. Windows runs
/// its own move loop here, so this must be called with the app released.
fn press(hwnd: HWND) {
    let mut before: RECT = unsafe { zeroed() };
    unsafe {
        GetWindowRect(hwnd, &mut before);
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, 0);
    }
    let mut after: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(hwnd, &mut after) };
    let moved = (after.left - before.left).abs() > 3 || (after.top - before.top).abs() > 3;
    with_app(|app| {
        if moved {
            app.config.bar_x = Some(after.left);
            app.config.bar_y = Some(after.top);
            save_setting(app, "bar-x", &after.left.to_string());
            save_setting(app, "bar-y", &after.top.to_string());
        } else {
            super::dashboard::toggle(app);
        }
    });
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_PAINT => with_app(|app| {
            paint(app, hwnd);
            0
        }),
        WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
        WM_LBUTTONDOWN => {
            press(hwnd);
            Some(0)
        }
        WM_RBUTTONUP => {
            show_menu();
            Some(0)
        }
        WM_DPICHANGED => {
            with_app(|app| {
                app.bar_fonts = Fonts::new(GetDpiForWindow(hwnd));
                relayout(app);
            });
            Some(0)
        }
        WM_ERASEBKGND => Some(1),
        _ => None,
    })
}
