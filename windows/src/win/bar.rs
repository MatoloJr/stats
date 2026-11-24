//! The readings strip: drops down under the activity line when you click it,
//! showing every reading in one row (`CPU 34% | RAM 68% | Commit 61% | ...`).
//! Click the readings for the dashboard, the pin to keep the strip open. It
//! fades out a few seconds after the pointer leaves, unless pinned.

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::app::{guarded, now_ms, save_setting, show_menu, with_app, App};
use super::draw::{self, glyph, Canvas, Fonts, MeasureDc};
use super::line;
use super::util::wide;
use stats_core::config::{Level, Metric};
use stats_core::fit::{fit, Detail, Segment};
use stats_core::numbers::{fmt_percent, fmt_rate, fmt_temp};
use stats_core::place;

const CLASS: &str = "StatsForWindowsBar";
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Unpinned, the strip closes this long after the pointer leaves it.
const LINGER_MS: u64 = 4000;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Button {
    Pin,
    Open,
}

#[derive(Default)]
pub struct Strip {
    pub open: bool,
    shown: bool,
    hover: bool,
    tracking: bool,
    idle_since: u64,
    hot: Option<Button>,
    /// Laid out with the strip (not while painting), so a click right after
    /// it opens already lands on the right button.
    buttons: Vec<(RECT, Button)>,
    /// Frosted glass is on (Windows 10 1803+ / 11).
    pub glass: bool,
}

pub fn create(instance: HINSTANCE) -> Result<HWND, String> {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(bar_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_HAND) },
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class.as_ptr(),
            wide("Stats readings").as_ptr(),
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
        return Err("could not create the readings strip".into());
    }
    Ok(hwnd)
}

/// Frosted glass behind the strip (also sets rounded corners). Call once.
pub fn enable_glass(app: &mut App) {
    app.strip.glass = super::glass::enable(app.bar);
}

/// How see-through the frosted background is: `bar-opacity` 80..255 maps to
/// a tint of roughly 20%..85%, so the blur always shows a little.
fn tint(app: &App) -> u8 {
    (app.config.bar_opacity as u32 * 215 / 255) as u8
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

/// Sizes at the strip's DPI: (padding, gap between readings, gap inside one,
/// height, button size).
fn metrics(fonts: &Fonts) -> (i32, i32, i32, i32, i32) {
    (fonts.px(14), fonts.px(22), fonts.px(7), fonts.px(34), fonts.px(26))
}

/// Measure the readings, fit them, size the strip and place it under the
/// line.
pub fn relayout(app: &mut App) {
    let fonts: &Fonts = &app.bar_fonts;
    let (pad, gap, inner, height, button) = metrics(fonts);
    let all = pieces(app);
    let dc = MeasureDc::new(fonts.mono);
    let mut segments = Vec::with_capacity(all.len());
    for p in &all {
        dc.select(fonts.mono);
        let name = if p.name.is_empty() { 0 } else { dc.width(&p.name) + inner };
        let speed = if p.speed.is_empty() { 0 } else { dc.width(&p.speed) + inner };
        dc.select(fonts.mono_bold);
        let value = dc.width(&p.value);
        segments.push(Segment { full: name + value + speed, compact: name + value, keep_priority: p.keep });
    }
    drop(dc);
    let work = line::work_area(app.line.hwnd);
    let buttons = 2 * button + fonts.px(4) + gap / 2;
    let max_width = (work.width() as f32 * 0.8) as i32 - 2 * pad - buttons;
    let (detail, shown) = fit(&segments, gap, max_width);
    let full = detail == Detail::Full;
    let content: i32 = shown.iter().map(|&i| if full { segments[i].full } else { segments[i].compact }).sum::<i32>()
        + gap * (shown.len() as i32 - 1).max(0);
    let width = content + 2 * pad + buttons;
    app.bar_layout = Layout { pieces: shown.iter().map(|&i| all[i].clone()).collect(), detail_full: full };

    // Buttons, right to left: open dashboard, then pin.
    let mid = (fonts.px(2) + height) / 2;
    let mut right = width - fonts.px(6);
    app.strip.buttons.clear();
    for which in [Button::Open, Button::Pin] {
        let r = RECT { left: right - button, top: mid - button / 2, right, bottom: mid + button / 2 };
        app.strip.buttons.push((r, which));
        right = r.left - fonts.px(4);
    }

    let r = place::strip_rect(app.line.rect, width, height, work, app.config.bar_position, fonts.px(4));
    let mut now: RECT = unsafe { zeroed() };
    unsafe { GetWindowRect(app.bar, &mut now) };
    if (now.left, now.top, now.right, now.bottom) != (r.left, r.top, r.right, r.bottom) {
        unsafe {
            SetWindowPos(
                app.bar,
                HWND_TOPMOST,
                r.left,
                r.top,
                r.width(),
                r.height(),
                SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            )
        };
    }
    unsafe { InvalidateRect(app.bar, null(), 0) };
}

pub fn open(app: &mut App) {
    if !app.strip.open {
        app.strip.open = true;
        relayout(app);
    }
    touch(app);
    line::wake(app);
}

pub fn close(app: &mut App) {
    app.strip.open = false;
    line::wake(app);
}

/// Click on the line: open, or close if already open.
pub fn toggle(app: &mut App) {
    if app.strip.open {
        close(app);
    } else {
        open(app);
    }
}

/// The dashboard opened on top: Windows may never send the strip its
/// mouse-leave, so forget the hover and fold an unpinned strip away.
pub fn release(app: &mut App) {
    app.strip.hover = false;
    app.strip.tracking = false;
    app.strip.hot = None;
    if !app.config.bar_pinned {
        close(app);
    }
}

/// The pointer did something: restart the linger countdown.
pub fn touch(app: &mut App) {
    app.strip.idle_since = now_ms();
}

pub fn set_pinned(app: &mut App, pinned: bool) {
    app.config.bar_pinned = pinned;
    save_setting(app, "bar-pinned", if pinned { "true" } else { "false" });
    super::util::log(if pinned { "pin on" } else { "pin off" });
    if pinned {
        open(app);
    }
    unsafe { InvalidateRect(app.bar, null(), 0) };
}

/// Called every line frame: show or hide, close after lingering. Returns
/// true while something is still changing.
pub fn animate(app: &mut App, now: u64, _dt: f32) -> bool {
    let strip = &mut app.strip;
    if strip.open && !app.config.bar_pinned && !strip.hover && !app.line.hover {
        if now.saturating_sub(strip.idle_since) > LINGER_MS {
            strip.open = false;
        }
    } else {
        strip.idle_since = now;
    }
    let want = strip.open && app.line.visible;
    if want != strip.shown {
        strip.shown = want;
        unsafe { ShowWindow(app.bar, if want { SW_SHOWNOACTIVATE } else { SW_HIDE }) };
    }
    false
}

/// Opacity (the frost tint) changed in settings.
pub fn apply_opacity(app: &App) {
    unsafe { InvalidateRect(app.bar, null(), 0) };
}

pub fn hide_now(app: &mut App) {
    app.strip.shown = false;
    unsafe { ShowWindow(app.bar, SW_HIDE) };
}

fn paint(app: &mut App, hwnd: HWND) {
    let fonts = &app.bar_fonts;
    let layout = &app.bar_layout;
    let (pad, gap, inner, _, _) = metrics(fonts);
    let mut canvas =
        if app.strip.glass { Canvas::begin_glass(hwnd, &[(draw::BG, tint(app))]) } else { Canvas::begin(hwnd) };
    let full = RECT { left: 0, top: 0, right: canvas.width, bottom: canvas.height };
    canvas.fill(full, draw::BG);
    canvas.fill(
        RECT { left: 0, top: 0, right: canvas.width, bottom: fonts.px(3) },
        draw::signal_color(app.worst_level()),
    );

    let (top, bottom) = (fonts.px(2), canvas.height);
    let mut x = pad;
    for (i, piece) in layout.pieces.iter().enumerate() {
        if i > 0 {
            canvas.font(fonts.mono);
            canvas.text_center("|", RECT { left: x - gap, top, right: x, bottom }, draw::LINE);
        }
        canvas.font(fonts.mono);
        if !piece.name.is_empty() {
            x = canvas.text(&piece.name, x, top, bottom, draw::TEXT) + inner;
        }
        canvas.font(fonts.mono_bold);
        let color = match piece.level {
            Some(Level::Normal) | None => draw::TEXT,
            Some(level) => draw::level_color(level),
        };
        x = canvas.text(&piece.value, x, top, bottom, color);
        if layout.detail_full && !piece.speed.is_empty() {
            canvas.font(fonts.mono);
            x = canvas.text(&piece.speed, x + inner, top, bottom, draw::MUTED);
        }
        x += gap;
    }

    // Pin and open-dashboard buttons (laid out in `relayout`). A pinned strip
    // shows a filled accent pin, so its state is obvious at a glance.
    let pinned = app.config.bar_pinned;
    for &(r, which) in &app.strip.buttons {
        let hot = app.strip.hot == Some(which);
        let (glyph_char, color) = match which {
            Button::Open => (glyph::CHEVRON_DOWN, if hot { draw::TEXT } else { draw::MUTED }),
            Button::Pin if pinned => (glyph::PINNED, draw::ACCENT),
            Button::Pin => (glyph::PIN, if hot { draw::TEXT } else { draw::MUTED }),
        };
        if which == Button::Pin && pinned {
            canvas.round_rect(r, fonts.px(8), draw::ACCENT_DIM, None);
        } else if hot {
            canvas.round_rect(r, fonts.px(8), draw::BUTTON_HOT, None);
        }
        canvas.icon(fonts, glyph_char, r, color);
    }
}

fn button_at(app: &App, x: i32, y: i32) -> Option<Button> {
    app.strip.buttons.iter().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|(_, b)| *b)
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xFFFF) as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_PAINT => with_app(|app| {
            paint(app, hwnd);
            0
        }),
        WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
        WM_MOUSEMOVE => {
            with_app(|app| {
                if !app.strip.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut tme);
                    app.strip.tracking = true;
                }
                app.strip.hover = true;
                touch(app);
                let hot = button_at(app, x, y);
                if hot != app.strip.hot {
                    app.strip.hot = hot;
                    InvalidateRect(hwnd, null(), 0);
                }
            });
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| {
                app.strip.tracking = false;
                app.strip.hover = false;
                app.strip.hot = None;
                touch(app);
                InvalidateRect(hwnd, null(), 0);
            });
            Some(0)
        }
        WM_LBUTTONUP => {
            with_app(|app| match button_at(app, x, y) {
                Some(Button::Pin) => {
                    let pinned = !app.config.bar_pinned;
                    set_pinned(app, pinned);
                }
                Some(Button::Open) | None => super::dashboard::show(app),
            });
            Some(0)
        }
        WM_RBUTTONUP => {
            show_menu();
            Some(0)
        }
        WM_ERASEBKGND => Some(1),
        _ => None,
    })
}
