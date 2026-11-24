//! The activity line: a small translucent capsule at the top of the screen
//! whose bars move with live system activity, like a voice meter. Click it
//! for the readings strip; right-click for the menu.
//!
//! It is a per-pixel-alpha layered window. The pixels are computed here
//! (anti-aliased rounded shapes into a premultiplied 32-bit DIB) and handed to
//! `UpdateLayeredWindow`, so edges stay smooth on any background and at any
//! DPI. Its timer also drives the strip's fade and auto-close, so the app has
//! one animation clock, which slows to 4 frames a second when nothing moves
//! and stops while the line is hidden.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, TTTOOLINFOW,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::app::{guarded, now_ms, show_menu, with_app, App};
use super::draw;
use super::util::wide;
use stats_core::config::Level;
use stats_core::numbers::fmt_percent;
use stats_core::place::{self, Rect};
use stats_core::wave;

const CLASS: &str = "StatsForWindowsLine";
const TIMER: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
pub const COLUMNS: usize = 28;
/// Window (hit area) and capsule sizes at 96 DPI.
const WIN_W: i32 = 172;
const WIN_H: i32 = 26;
const CAP_W: i32 = 132;
const CAP_H: i32 = 12;
const CAP_W_HOVER: i32 = 152;
const CAP_H_HOVER: i32 = 18;
// 20 frames a second: smooth for bars this small, at two thirds the cost of 30.
const FAST_MS: u32 = 50;
const SLOW_MS: u32 = 250;
const SAMPLE_MS: u64 = 100;

#[derive(Default)]
pub struct State {
    pub hwnd: HWND,
    tooltip: HWND,
    /// Where the window is on screen.
    pub rect: Rect,
    pub visible: bool,
    pub hover: bool,
    tracking: bool,
    /// 0 = resting, 1 = fully grown for hover.
    grow: f32,
    timer_ms: u32,
    last_frame: u64,
    last_sample: u64,
    pub started: u64,
    surface: Option<Surface>,
    /// The capsule without its bars, reused while size, hover and level stay
    /// the same, so most frames only draw the 28 small bars.
    base: Vec<u32>,
    base_key: Option<(i32, i32, u32, Level)>,
}

pub fn create(instance: HINSTANCE) -> Result<State, String> {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(line_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_HAND) },
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("Stats activity").as_ptr(),
            WS_POPUP,
            0,
            0,
            WIN_W,
            WIN_H,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err("could not create the activity line".into());
    }
    let tooltip = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            hwnd,
            null_mut(),
            instance,
            null(),
        )
    };
    if !tooltip.is_null() {
        let mut text = wide("Stats");
        let mut info = tool_info(hwnd, &mut text);
        unsafe {
            SendMessageW(tooltip, TTM_ADDTOOLW, 0, &mut info as *mut _ as LPARAM);
            SendMessageW(tooltip, TTM_SETMAXTIPWIDTH, 0, 400);
        }
    }
    Ok(State { hwnd, tooltip, started: now_ms(), ..Default::default() })
}

fn tool_info(hwnd: HWND, text: &mut [u16]) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd,
        uId: hwnd as usize,
        lpszText: text.as_mut_ptr(),
        ..unsafe { zeroed() }
    }
}

fn scale(app: &App, value: i32) -> i32 {
    app.bar_fonts.px(value)
}

/// Work area of the monitor the line belongs on (the primary one).
pub fn work_area(hwnd: HWND) -> Rect {
    unsafe {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let w = info.rcWork;
        Rect::new(w.left, w.top, w.right, w.bottom)
    }
}

fn primary_work_area() -> Rect {
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(monitor, &mut info);
        let w = info.rcWork;
        Rect::new(w.left, w.top, w.right, w.bottom)
    }
}

/// Put the line where the settings say, on the primary monitor.
pub fn place(app: &mut App) {
    let dpi = unsafe { GetDpiForWindow(app.line.hwnd) }.max(96);
    if dpi != app.bar_fonts.dpi {
        app.bar_fonts = draw::Fonts::new(dpi);
    }
    let work = primary_work_area();
    app.line.rect =
        place::line_rect(work, scale(app, WIN_W), scale(app, WIN_H), app.config.bar_position, scale(app, 16));
    render(app);
}

pub fn show(app: &mut App) {
    if app.line.visible {
        return;
    }
    app.line.visible = true;
    place(app);
    unsafe { ShowWindow(app.line.hwnd, SW_SHOWNOACTIVATE) };
    app.line.last_frame = now_ms();
    set_timer(app, FAST_MS);
}

pub fn hide(app: &mut App) {
    if !app.line.visible {
        return;
    }
    app.line.visible = false;
    unsafe {
        KillTimer(app.line.hwnd, TIMER);
        ShowWindow(app.line.hwnd, SW_HIDE);
    }
    app.line.timer_ms = 0;
}

fn set_timer(app: &mut App, ms: u32) {
    if app.line.timer_ms != ms {
        app.line.timer_ms = ms;
        unsafe { SetTimer(app.line.hwnd, TIMER, ms, None) };
    }
}

/// Forget the hover (another window took over the pointer).
pub fn release(app: &mut App) {
    app.line.hover = false;
    app.line.tracking = false;
}

/// Make sure frames run fast (something just started moving).
pub fn wake(app: &mut App) {
    if app.line.visible {
        set_timer(app, FAST_MS);
    }
}

/// Tooltip with the headline numbers; updated once a poll.
pub fn update_tooltip(app: &App) {
    if app.line.tooltip.is_null() {
        return;
    }
    let s = &app.snap;
    let mut text = wide(&format!(
        "CPU {} · RAM {} · Commit {}\r\nClick for details · right-click for the menu",
        fmt_percent(s.cpu),
        fmt_percent(s.memory),
        fmt_percent(s.commit)
    ));
    let mut info = tool_info(app.line.hwnd, &mut text);
    unsafe { SendMessageW(app.line.tooltip, TTM_UPDATETIPTEXTW, 0, &mut info as *mut _ as LPARAM) };
}

/// One animation step: sample, ease, draw, and pick the next frame rate.
fn frame(app: &mut App) {
    let now = now_ms();
    let dt = now.saturating_sub(app.line.last_frame).min(500) as f32;
    app.line.last_frame = now;

    if now.saturating_sub(app.line.last_sample) >= SAMPLE_MS {
        app.line.last_sample = now;
        let cpu = app.cpu_meter.sample();
        let disk = wave::normalize(app.snap.disk_io(), app.hist.disk.max(), 4.0e6);
        let net = wave::normalize(app.snap.net_down, app.hist.net_down.max(), 1.0e6);
        app.wave.push(wave::activity(cpu, disk, net));
    }
    app.wave.advance(dt);

    let target = if app.line.hover { 1.0 } else { 0.0 };
    let step = dt / 120.0;
    app.line.grow =
        if app.line.grow < target { (app.line.grow + step).min(target) } else { (app.line.grow - step).max(target) };

    let strip_moving = super::bar::animate(app, now, dt);
    render(app);

    let busy = !app.wave.is_settled() || app.line.grow != target || strip_moving;
    set_timer(app, if busy { FAST_MS } else { SLOW_MS });
}

/// The bitmap the line is drawn into, kept between frames (re-made only when
/// the size changes) so a frame allocates nothing.
pub struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    fn new(w: i32, h: i32) -> Option<Surface> {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            let mut info: BITMAPINFO = zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..zeroed()
            };
            let mut bits: *mut core::ffi::c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let old = SelectObject(dc, bitmap);
            Some(Surface { dc, bitmap, old, bits: bits as *mut u32, w, h })
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

/// Premultiplied BGRA pixels, written by hand so edges are anti-aliased.
struct Pixels<'a> {
    w: i32,
    h: i32,
    data: &'a mut [u32],
}

impl Pixels<'_> {
    /// Blend `color` at `alpha` (0..1) over pixel (x, y).
    fn blend(&mut self, x: i32, y: i32, color: COLORREF, alpha: f32) {
        if x < 0 || y < 0 || x >= self.w || y >= self.h || alpha <= 0.0 {
            return;
        }
        let a = alpha.min(1.0);
        let (r, g, b) = draw::parts(color);
        let i = (y * self.w + x) as usize;
        let dst = self.data[i];
        let inv = 1.0 - a;
        let ch = |shift: u32, src: u8| -> u32 {
            let d = ((dst >> shift) & 0xFF) as f32;
            ((src as f32 * a + d * inv).round() as u32).min(255) << shift
        };
        let da = ((dst >> 24) & 0xFF) as f32;
        let out_a = ((a * 255.0 + da * inv).round() as u32).min(255) << 24;
        self.data[i] = out_a | ch(16, r) | ch(8, g) | ch(0, b);
    }

    /// Anti-aliased rounded rectangle (radius in px, float edges allowed).
    fn round_rect(&mut self, (x0, y0, x1, y1): (f32, f32, f32, f32), radius: f32, color: COLORREF, alpha: f32) {
        let r = radius.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
        let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        let (hx, hy) = ((x1 - x0) / 2.0 - r, (y1 - y0) / 2.0 - r);
        for y in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for x in (x0.floor() as i32)..=(x1.ceil() as i32) {
                // Signed distance from the pixel centre to the shape.
                let px = (x as f32 + 0.5 - cx).abs() - hx;
                let py = (y as f32 + 0.5 - cy).abs() - hy;
                let outside = (px.max(0.0).powi(2) + py.max(0.0).powi(2)).sqrt();
                let inside = px.max(py).min(0.0);
                let d = outside + inside - r;
                let coverage = (0.5 - d).clamp(0.0, 1.0);
                if coverage > 0.0 {
                    self.blend(x, y, color, alpha * coverage);
                }
            }
        }
    }
}

fn render(app: &mut App) {
    if !app.line.visible {
        return;
    }
    let (w, h) = (app.line.rect.width(), app.line.rect.height());
    if w <= 0 || h <= 0 {
        return;
    }
    if app.line.surface.as_ref().is_none_or(|s| (s.w, s.h) != (w, h)) {
        app.line.surface = Surface::new(w, h);
    }
    let Some(surface) = app.line.surface.as_ref() else { return };
    // SAFETY: the DIB holds exactly w*h pixels and lives as long as `surface`.
    let data = unsafe { std::slice::from_raw_parts_mut(surface.bits, (w * h) as usize) };
    let dpi = app.bar_fonts.dpi as i32;
    let s = move |v: i32| (v * dpi / 96) as f32;

    // Capsule, growing a little on hover.
    let g = ease(app.line.grow);
    let cap_w = s(CAP_W) + (s(CAP_W_HOVER) - s(CAP_W)) * g;
    let cap_h = s(CAP_H) + (s(CAP_H_HOVER) - s(CAP_H)) * g;
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (x0, y0, x1, y1) = (cx - cap_w / 2.0, cy - cap_h / 2.0, cx + cap_w / 2.0, cy + cap_h / 2.0);
    let level = app.worst_level();
    let signal = draw::signal_color(level);
    let key = (w, h, (app.line.grow * 64.0) as u32, level);
    if app.line.base_key == Some(key) && app.line.base.len() == data.len() {
        data.copy_from_slice(&app.line.base);
    } else {
        data.fill(0);
        let mut px = Pixels { w, h, data: &mut *data };
        px.round_rect((x0 - 1.0, y0 - 1.0, x1 + 1.0, y1 + 1.0), cap_h / 2.0 + 1.0, draw::LINE, 0.55 + 0.3 * g);
        px.round_rect((x0, y0, x1, y1), cap_h / 2.0, draw::BG, 0.86);
        app.line.base.clear();
        app.line.base.extend_from_slice(data);
        app.line.base_key = Some(key);
    }
    let mut px = Pixels { w, h, data };

    // Bars, mirrored around the middle like a voice meter.
    let heights = app.wave.heights();
    let n = heights.len().max(1) as f32;
    let inner_w = cap_w - cap_h; // keep clear of the rounded ends
    let pitch = inner_w / n;
    let bar_w = (pitch * 0.55).max(1.2);
    let max_h = cap_h - s(4).max(3.0);
    // A slow breath keeps the idle line looking alive (about 1% CPU-free).
    let t = now_ms().saturating_sub(app.line.started) as f32 / 1000.0;
    let breath = 0.5 + 0.5 * (t * 1.6).sin();
    let min_h = s(2).max(1.5) + 0.6 * breath;
    for (i, &v) in heights.iter().enumerate() {
        let bh = (min_h + v * (max_h - min_h)).min(max_h);
        let bx = x0 + cap_h / 2.0 + pitch * (i as f32 + 0.5) - bar_w / 2.0;
        // Older bars fade a touch, so the newest activity reads first.
        let age = i as f32 / n;
        let alpha = 0.45 + 0.55 * age.powf(0.6);
        px.round_rect((bx, cy - bh / 2.0, bx + bar_w, cy + bh / 2.0), bar_w / 2.0, signal, alpha);
    }

    unsafe {
        GdiFlush();
        let pos = POINT { x: app.line.rect.left, y: app.line.rect.top };
        let size = SIZE { cx: w, cy: h };
        let src = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(app.line.hwnd, null_mut(), &pos, &size, surface.dc, &src, 0, &blend, ULW_ALPHA);
    }
}

fn ease(t: f32) -> f32 {
    // Ease-out cubic.
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

unsafe extern "system" fn line_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_TIMER if wparam == TIMER => {
            with_app(frame);
            Some(0)
        }
        WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
        WM_MOUSEMOVE => {
            with_app(|app| {
                if !app.line.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut tme);
                    app.line.tracking = true;
                }
                if !app.line.hover {
                    app.line.hover = true;
                    super::bar::touch(app);
                    wake(app);
                }
            });
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| {
                app.line.tracking = false;
                app.line.hover = false;
                super::bar::touch(app);
                wake(app);
            });
            Some(0)
        }
        WM_LBUTTONUP => {
            with_app(super::bar::toggle);
            Some(0)
        }
        WM_RBUTTONUP => {
            show_menu();
            Some(0)
        }
        WM_DPICHANGED => {
            with_app(place);
            Some(0)
        }
        _ => None,
    })
}
