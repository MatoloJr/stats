//! The activity line: an "island" on a screen edge (top centre, or the left
//! or right side, drawn vertically) that moves with live system activity.
//!
//! At rest it is a hairline flush against the edge, a soft glow whose
//! brightness and thickness follow activity, and the window lets every click
//! through, so a browser's tabs underneath keep working. Rest the pointer on
//! the hairline for a moment (or press a shortcut) and it wakes: it grows into
//! a small capsule with voice-meter bars that you can click for the readings
//! strip, or right-click for the menu. It sleeps again when the pointer
//! leaves.
//!
//! It is a per-pixel-alpha layered window: the pixels are computed here
//! (anti-aliased rounded shapes into a premultiplied 32-bit DIB) and handed to
//! `UpdateLayeredWindow`. Its timer is the app's one animation clock (it also
//! drives the strip's fade and auto-close); it slows to 4 frames a second when
//! nothing moves and stops while the line is hidden.

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Controls::{
    TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_SETMAXTIPWIDTH, TTM_UPDATETIPTEXTW, TTS_ALWAYSTIP,
    TTS_NOPREFIX, TTTOOLINFOW,
};
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::app::{guarded, now_ms, show_menu, with_app, App};
use super::draw;
use super::util::wide;
use stats_core::config::Level;
use stats_core::numbers::fmt_percent;
use stats_core::place::{self, Anchor, Rect};
use stats_core::wave;

const CLASS: &str = "StatsForWindowsLine";
const TIMER: usize = 1;
pub const COLUMNS: usize = 32;
// Sizes at 96 DPI, along the edge (length) and away from it (depth).
/// The window: room for the island when awake.
const WIN_LEN: i32 = 250;
const WIN_DEPTH: i32 = 34;
/// Resting hairline.
const REST_LEN: i32 = 220;
const REST_THICK: i32 = 4;
/// Awake island.
const AWAKE_LEN: i32 = 244;
const AWAKE_THICK: i32 = 26;
const AWAKE_GAP: i32 = 3;
/// How far from the edge the pointer counts as "on the hairline".
const WAKE_BAND: i32 = 6;
/// The pointer must rest this long on the hairline to wake it, so passing
/// over it or clicking a tab under it never does.
const WAKE_AFTER_MS: u64 = 350;
const SLEEP_AFTER_MS: u64 = 600;
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
    /// Grown into the island and taking clicks.
    pub awake: bool,
    /// The pointer is over the island (polled, not from mouse messages,
    /// because a resting line receives none).
    pub hover: bool,
    dwell_since: Option<u64>,
    away_since: Option<u64>,
    /// 0 = hairline, 1 = island.
    grow: f32,
    click_through: bool,
    timer_ms: u32,
    last_frame: u64,
    last_sample: u64,
    pub started: u64,
    surface: Option<Surface>,
    /// The background shape without its bars, reused while nothing about it
    /// changes, so most frames only draw the bars.
    base: Vec<u32>,
    base_key: Option<(i32, i32, u32, Level, Anchor)>,
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
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            class.as_ptr(),
            wide("Stats activity").as_ptr(),
            WS_POPUP,
            0,
            0,
            WIN_LEN,
            WIN_DEPTH,
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
    Ok(State { hwnd, tooltip, started: now_ms(), click_through: true, ..Default::default() })
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
    app.line.rect = place::line_rect(work, scale(app, WIN_LEN), scale(app, WIN_DEPTH), app.config.bar_position);
    app.line.base_key = None;
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
    app.line.awake = false;
    app.line.hover = false;
    app.line.grow = 0.0;
    set_click_through(app, true);
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

fn set_click_through(app: &mut App, through: bool) {
    if app.line.click_through == through {
        return;
    }
    app.line.click_through = through;
    unsafe {
        let style = GetWindowLongW(app.line.hwnd, GWL_EXSTYLE) as u32;
        let style = if through { style | WS_EX_TRANSPARENT } else { style & !WS_EX_TRANSPARENT };
        SetWindowLongW(app.line.hwnd, GWL_EXSTYLE, style as i32);
    }
}

/// Grow into the island now (pointer rested on it, or a shortcut).
pub fn wake_island(app: &mut App) {
    if !app.line.visible {
        return;
    }
    app.line.awake = true;
    app.line.away_since = None;
    set_click_through(app, false);
    wake(app);
}

/// Forget the hover (another window took over the pointer).
pub fn release(app: &mut App) {
    app.line.hover = false;
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
        "CPU {} · RAM {} · Commit {}\r\nClick for the readings · right-click for the menu",
        fmt_percent(s.cpu),
        fmt_percent(s.memory),
        fmt_percent(s.commit)
    ));
    let mut info = tool_info(app.line.hwnd, &mut text);
    unsafe { SendMessageW(app.line.tooltip, TTM_UPDATETIPTEXTW, 0, &mut info as *mut _ as LPARAM) };
}

/// Pointer tracking by polling: a click-through window gets no mouse
/// messages, and polling the cursor once a frame costs next to nothing.
fn track_pointer(app: &mut App, now: u64) {
    let mut p = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return;
    }
    let anchor = app.config.bar_position;
    let band = place::edge_band(app.line.rect, anchor, scale(app, WAKE_BAND));
    let on_band = band.contains(p.x, p.y);
    let on_window = app.line.rect.contains(p.x, p.y);
    app.line.hover = app.line.awake && on_window;

    if !app.line.awake {
        if on_band {
            let since = *app.line.dwell_since.get_or_insert(now);
            if now.saturating_sub(since) >= WAKE_AFTER_MS {
                app.line.dwell_since = None;
                wake_island(app);
                super::bar::touch(app);
            }
        } else {
            app.line.dwell_since = None;
        }
        return;
    }
    // Awake: stay so while the pointer is on it or the strip is open.
    if on_window || app.strip.open {
        app.line.away_since = None;
        return;
    }
    let since = *app.line.away_since.get_or_insert(now);
    if now.saturating_sub(since) >= SLEEP_AFTER_MS {
        app.line.awake = false;
        app.line.away_since = None;
        set_click_through(app, true);
    }
}

/// One animation step: sample, track the pointer, ease, draw, and pick the
/// next frame rate.
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
    track_pointer(app, now);

    let target = if app.line.awake { 1.0 } else { 0.0 };
    let step = dt / 180.0;
    app.line.grow =
        if app.line.grow < target { (app.line.grow + step).min(target) } else { (app.line.grow - step).max(target) };

    let strip_moving = super::bar::animate(app, now, dt);
    render(app);

    // Keep frames quick while the pointer is resting on the hairline, so it
    // wakes on time.
    let waiting = app.line.dwell_since.is_some() || (app.line.awake && app.line.away_since.is_some());
    let busy = !app.wave.is_settled() || app.line.grow != target || strip_moving || waiting;
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
    anchor: Anchor,
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

    /// Anti-aliased rounded rectangle in pixel coordinates.
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

    /// The same, in edge coordinates: `u` along the edge, `v` away from it.
    /// This is what lets one drawing serve the top and both sides.
    fn edge_rect(&mut self, (u0, v0, u1, v1): (f32, f32, f32, f32), radius: f32, color: COLORREF, alpha: f32) {
        let w = self.w as f32;
        let rect = match self.anchor {
            Anchor::Top => (u0, v0, u1, v1),
            Anchor::Left => (v0, u0, v1, u1),
            Anchor::Right => (w - v1, u0, w - v0, u1),
        };
        self.round_rect(rect, radius, color, alpha);
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
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
    let anchor = app.config.bar_position;
    let length = if anchor == Anchor::Top { w } else { h } as f32;

    // Shape: a hairline on the edge growing into a capsule hanging from it.
    let g = ease(app.line.grow);
    let len = lerp(s(REST_LEN), s(AWAKE_LEN), g);
    let thick = lerp(s(REST_THICK), s(AWAKE_THICK), g);
    let gap = lerp(0.0, s(AWAKE_GAP), g);
    let u0 = (length - len) / 2.0;
    let u1 = u0 + len;
    let (v0, v1) = (gap, gap + thick);
    let mid_v = (v0 + v1) / 2.0;
    let level = app.worst_level();
    let signal = draw::signal_color(level);

    let key = (w, h, (app.line.grow * 64.0) as u32, level, anchor);
    if app.line.base_key == Some(key) && app.line.base.len() == data.len() {
        data.copy_from_slice(&app.line.base);
    } else {
        data.fill(0);
        let mut px = Pixels { w, h, data: &mut *data, anchor };
        // Resting: a faint dark rail so the glow reads on light pages too.
        // Awake: the island, with a fine outline.
        let rail = 0.30 + 0.56 * g;
        if g > 0.02 {
            px.edge_rect((u0 - 1.0, v0 - 1.0, u1 + 1.0, v1 + 1.0), thick / 2.0 + 1.0, draw::LINE, 0.7 * g);
        }
        px.edge_rect((u0, v0, u1, v1), thick / 2.0, draw::BG, rail);
        app.line.base.clear();
        app.line.base.extend_from_slice(data);
        app.line.base_key = Some(key);
    }
    let mut px = Pixels { w, h, data, anchor };

    // Activity: each column is a glow segment at rest and a bar when awake.
    let heights = app.wave.heights();
    let n = heights.len().max(1) as f32;
    let inset = lerp(thick * 0.4, thick * 0.55, g);
    let pitch = (len - 2.0 * inset) / n;
    let along = lerp(pitch * 0.92, (pitch * 0.5).max(1.2), g);
    let max_cross = thick - lerp(1.0, s(8).max(6.0), g);
    let t = now_ms().saturating_sub(app.line.started) as f32 / 1000.0;
    let breath = 0.5 + 0.5 * (t * 1.6).sin();
    let min_cross = lerp(1.2, s(2).max(1.5), g) + 0.5 * breath * g;
    for (i, &v) in heights.iter().enumerate() {
        let cross = (min_cross + v * (max_cross - min_cross)).clamp(min_cross, max_cross.max(min_cross));
        let cu = u0 + inset + pitch * (i as f32 + 0.5);
        // Newest on the right (top) or at the bottom (sides).
        let age = i as f32 / n;
        let glow = 0.35 + 0.65 * v.sqrt();
        let bar = 0.45 + 0.55 * age.powf(0.6);
        let alpha = lerp(glow, bar, g);
        px.edge_rect(
            (cu - along / 2.0, mid_v - cross / 2.0, cu + along / 2.0, mid_v + cross / 2.0),
            along.min(cross) / 2.0,
            signal,
            alpha,
        );
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

/// Click on the awake island: the readings; if they are pinned open, the
/// dashboard (so a pinned strip is never closed by accident).
fn click(app: &mut App) {
    if app.config.bar_pinned && app.strip.open {
        super::dashboard::show(app);
    } else {
        super::bar::toggle(app);
    }
}

unsafe extern "system" fn line_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_TIMER if wparam == TIMER => {
            with_app(frame);
            Some(0)
        }
        WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
        // Only reached while awake (asleep, the window is click-through).
        WM_MOUSEMOVE => {
            with_app(|app| {
                app.line.away_since = None;
                super::bar::touch(app);
            });
            Some(0)
        }
        WM_LBUTTONUP => {
            with_app(|app| {
                wake_island(app);
                click(app);
            });
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

/// For the log, once at start: where the line went.
pub fn describe(app: &App) -> String {
    let r = app.line.rect;
    format!("line at {} ({},{} {}x{})", app.config.bar_position.key(), r.left, r.top, r.width(), r.height())
}
