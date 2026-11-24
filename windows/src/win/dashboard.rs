//! The dashboard popup: every reading, the programs using the most memory,
//! and a button next to each problem that fixes it. Sections fold away by
//! clicking their heading (remembered in `collapsed-sections`). It closes
//! when you click elsewhere or press Esc.

use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::actions::{short_path, Action};
use super::app::{guarded, with_app, App};
use super::draw::{self, Canvas, Fonts, MeasureDc};
use super::util::wide;
use stats_core::advice::{self, Fix};
use stats_core::config::Level;
use stats_core::history::Ring;
use stats_core::numbers::{fmt_bytes, fmt_percent, fmt_rate, fmt_temp};
use stats_core::pagefile::PageFile;

const CLASS: &str = "StatsForWindowsDashboard";
const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDTH: i32 = 460;

#[derive(Default)]
pub struct State {
    pub scroll: i32,
    pub hits: Vec<(RECT, Hit)>,
    pub hot: Option<usize>,
    pub content_height: i32,
    pub tracking: bool,
    /// When it was last closed by clicking elsewhere. Clicking the tray icon
    /// first deactivates the dashboard, then sends the click; without this
    /// the click would reopen it at once.
    pub hidden_at: u64,
}

#[derive(Clone, Debug)]
pub enum Hit {
    Section(String),
    Act(Action),
}

#[derive(Clone, Copy)]
enum Spark {
    Cpu,
    Commit,
    Gpu,
    NetDown,
    Disk,
}

enum Row {
    Header { id: &'static str, title: String, summary: String, collapsed: bool },
    Line { label: String, value: String, level: Level },
    Spark { which: Spark, max: f32, color: COLORREF },
    Act { label: String, value: String, level: Level, button: String, action: Action },
    Note { text: String, color: COLORREF },
}

fn ago(at: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(at).map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

fn rows(app: &App) -> Vec<Row> {
    let s = &app.snap;
    let c = &app.config;
    let collapsed = |id: &str| c.collapsed_sections.iter().any(|x| x == id);
    let mut out = Vec::new();
    let header = |out: &mut Vec<Row>, id: &'static str, title: &str, summary: String| -> bool {
        let folded = collapsed(id);
        out.push(Row::Header { id, title: title.into(), summary, collapsed: folded });
        !folded
    };

    if let Some(label) = &app.busy {
        out.push(Row::Note { text: format!("Working: {label}…"), color: draw::ACCENT });
    }
    if let Some((text, ok)) = &app.status {
        out.push(Row::Note { text: text.clone(), color: if *ok { draw::OK } else { draw::WARN } });
    }

    // Memory first: it is what crashes things.
    let commit_level = c.commit.level(s.commit);
    if header(&mut out, "memory", "Memory", format!("RAM {} · Commit {}", fmt_percent(s.memory), fmt_percent(s.commit)))
    {
        out.push(Row::Line {
            label: "RAM in use".into(),
            value: format!("{} / {} ({})", fmt_bytes(s.ram_used), fmt_bytes(s.ram_total), fmt_percent(s.memory)),
            level: c.memory.level(s.memory),
        });
        out.push(Row::Line {
            label: "Committed (limit)".into(),
            value: format!("{} / {} ({})", fmt_bytes(s.commit_used), fmt_bytes(s.commit_limit), fmt_percent(s.commit)),
            level: commit_level,
        });
        out.push(Row::Spark {
            which: Spark::Commit,
            max: 100.0,
            color: if commit_level == Level::Normal { draw::ACCENT } else { draw::level_color(commit_level) },
        });
        let now = fmt_bytes(s.pagefile_bytes());
        let described = match app.pagefile {
            Some(PageFile::SystemManaged) => format!("{now}, grows as needed"),
            Some(PageFile::Custom { max_mb }) => format!("fixed, max {}", fmt_bytes(max_mb << 20)),
            Some(PageFile::Disabled) => "turned off".to_string(),
            None => now,
        };
        if c.pagefile_check && app.pagefile_small() {
            out.push(Row::Act {
                label: "Page file".into(),
                value: format!("{described} — too small"),
                level: Level::Warning,
                button: "Fix".into(),
                action: Action::OpenPagefileSettings,
            });
        } else {
            out.push(Row::Line { label: "Page file".into(), value: described, level: Level::Normal });
        }
        if let Some(vm) = s.vm_bytes {
            out.push(Row::Act {
                label: "WSL / Docker VM".into(),
                value: fmt_bytes(vm),
                level: Level::Normal,
                button: "Shut down".into(),
                action: Action::ShutdownWsl,
            });
        }
        if let Some((key, pids)) = &app.leftover {
            out.push(Row::Act {
                label: format!("{} didn't close", advice::display_name(key)),
                value: String::new(),
                level: Level::Critical,
                button: "Force end".into(),
                action: Action::ForceEnd { key: key.clone(), pids: pids.clone() },
            });
        }
        if app.groups.is_empty() {
            out.push(Row::Note { text: "Reading programs…".into(), color: draw::MUTED });
        }
        for group in app.groups.iter().filter(|g| g.private_bytes > 0).take(6) {
            let name = advice::display_name(&group.key);
            let label = if group.count > 1 { format!("{name} ×{}", group.count) } else { name };
            let (_, fix) = advice::advise(&group.key, stats_core::pressure::Kind::Memory);
            let action = match fix {
                Fix::CloseApp(key) => Some(("Close", Action::CloseApp { key, pids: group.pids.clone() })),
                Fix::ShutdownWsl => Some(("Shut down", Action::ShutdownWsl)),
                Fix::None => None,
            };
            let value = fmt_bytes(group.private_bytes);
            match action {
                Some((button, action)) => {
                    out.push(Row::Act { label, value, level: Level::Normal, button: button.into(), action })
                }
                None => out.push(Row::Line { label, value, level: Level::Normal }),
            }
        }
        if let Some((trigger, Some(key))) = &app.last_alert {
            let (hint, _) = advice::advise(key, trigger.kind);
            out.push(Row::Note {
                text: format!("Last warning: {}. {hint}", advice::display_name(key)),
                color: draw::MUTED,
            });
        }
    }

    if header(&mut out, "cpu", "CPU", fmt_percent(s.cpu)) {
        out.push(Row::Line { label: "Busy".into(), value: fmt_percent(s.cpu), level: c.cpu.level(s.cpu) });
        out.push(Row::Spark { which: Spark::Cpu, max: 100.0, color: draw::ACCENT });
        let mut by_cpu: Vec<_> = app.groups.iter().filter(|g| g.cpu >= 0.5).collect();
        by_cpu.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
        for group in by_cpu.into_iter().take(3) {
            out.push(Row::Line {
                label: advice::display_name(&group.key),
                value: format!("{:.0}%", group.cpu),
                level: Level::Normal,
            });
        }
    }

    if header(&mut out, "gpu", "GPU", fmt_percent(s.gpu)) {
        out.push(Row::Line { label: "Busy".into(), value: fmt_percent(s.gpu), level: c.gpu.level(s.gpu) });
        if let Some(bytes) = s.gpu_dedicated {
            out.push(Row::Line { label: "Dedicated memory".into(), value: fmt_bytes(bytes), level: Level::Normal });
        }
        out.push(Row::Spark { which: Spark::Gpu, max: 100.0, color: draw::ACCENT });
    }

    if header(&mut out, "temperature", "Temperature", fmt_temp(s.temperature, c.temperature_unit)) {
        match s.temperature {
            Some(t) => out.push(Row::Line {
                label: "Hottest thermal zone".into(),
                value: fmt_temp(Some(t), c.temperature_unit),
                level: c.temp.level(Some(t)),
            }),
            None => out.push(Row::Note {
                text: "This laptop does not expose a temperature sensor to Windows without the maker's own tools."
                    .into(),
                color: draw::MUTED,
            }),
        }
    }

    let first = s.drives.first();
    let summary = first.map(|d| format!("{} {}", d.name, fmt_percent(d.percent))).unwrap_or_default();
    if header(&mut out, "storage", "Storage", summary) {
        for drive in &s.drives {
            out.push(Row::Line {
                label: drive.name.clone(),
                value: format!(
                    "{} / {} used ({})",
                    fmt_bytes(drive.used),
                    fmt_bytes(drive.total),
                    fmt_percent(drive.percent)
                ),
                level: c.storage.level(drive.percent),
            });
        }
        out.push(Row::Line {
            label: "Disk activity".into(),
            value: format!("Read {} · Write {}", fmt_rate(s.disk_read), fmt_rate(s.disk_write)),
            level: Level::Normal,
        });
        out.push(Row::Spark {
            which: Spark::Disk,
            max: app.hist.disk.max().unwrap_or(1.0).max(1.0),
            color: draw::ACCENT,
        });
        out.push(Row::Act {
            label: "Temp folder".into(),
            value: app.temp_bytes.map(fmt_bytes).unwrap_or_else(|| "size found by the next scan".into()),
            level: if app.temp_bytes.unwrap_or(0) > 5 << 30 { Level::Warning } else { Level::Normal },
            button: "Clean".into(),
            action: Action::CleanTemp,
        });
        out.push(Row::Act {
            label: "Package caches".into(),
            value: "npm, go, pip, pnpm".into(),
            level: Level::Normal,
            button: "Clean".into(),
            action: Action::CleanCaches,
        });
        if let Some(bytes) = app.docker_disk {
            out.push(Row::Act {
                label: "Docker disk".into(),
                value: fmt_bytes(bytes),
                level: if bytes > 30 << 30 { Level::Warning } else { Level::Normal },
                button: "Prune".into(),
                action: Action::DockerPrune,
            });
            out.push(Row::Act {
                label: "Give freed space back".into(),
                value: "needs Docker closed".into(),
                level: Level::Normal,
                button: "Compact".into(),
                action: Action::CompactDocker,
            });
        }
        out.push(Row::Act {
            label: "Storage Sense".into(),
            value: "Windows auto-cleanup".into(),
            level: Level::Normal,
            button: "Open".into(),
            action: Action::OpenStorageSettings,
        });
    }

    let total: u64 = app.junk.iter().map(|d| d.bytes).sum();
    let summary = if app.junk_at.is_some() {
        format!("{} in {}", fmt_bytes(total), app.junk.len())
    } else {
        "not scanned".into()
    };
    if header(&mut out, "build", "Build folders", summary) {
        out.push(Row::Act {
            label: "Last scan".into(),
            value: app.junk_at.map(ago).unwrap_or_else(|| "never".into()),
            level: Level::Normal,
            button: "Scan".into(),
            action: Action::ScanJunk,
        });
        let roots = super::app::scan_roots(c);
        if roots.is_empty() {
            out.push(Row::Note {
                text: format!(
                    "None of the scan folders exist ({}). Set scan-roots in Settings.",
                    c.scan_roots.join("; ")
                ),
                color: draw::WARN,
            });
        }
        let alert = c.junk_alert_gb as u64 * (1 << 30);
        for dir in app.junk.iter().take(8) {
            out.push(Row::Act {
                label: short_path(&dir.path),
                value: format!("{} · {}", fmt_bytes(dir.bytes), dir.kind.label()),
                level: if dir.bytes >= alert { Level::Warning } else { Level::Normal },
                button: "Delete".into(),
                action: Action::CleanJunk(dir.clone()),
            });
        }
        let big: Vec<_> = app.junk.iter().filter(|d| d.bytes >= 100 << 20).cloned().collect();
        if big.len() > 1 {
            let sum: u64 = big.iter().map(|d| d.bytes).sum();
            out.push(Row::Act {
                label: format!("All {} over 100 MB", big.len()),
                value: fmt_bytes(sum),
                level: Level::Normal,
                button: "Delete all".into(),
                action: Action::CleanAllJunk(big),
            });
        }
    }

    if header(&mut out, "network", "Network", format!("↓ {}", fmt_rate(s.net_down))) {
        out.push(Row::Line { label: "Download".into(), value: fmt_rate(s.net_down), level: Level::Normal });
        out.push(Row::Line { label: "Upload".into(), value: fmt_rate(s.net_up), level: Level::Normal });
        out.push(Row::Spark {
            which: Spark::NetDown,
            max: app.hist.net_down.max().unwrap_or(1.0).max(1.0),
            color: draw::ACCENT,
        });
    }

    if !app.sensors.displays.is_empty() {
        let first = &app.sensors.displays[0];
        if header(&mut out, "displays", "Displays", format!("{} Hz", first.hz)) {
            for d in &app.sensors.displays {
                out.push(Row::Line {
                    label: d.name.clone(),
                    value: format!("{}×{} · {} Hz", d.width, d.height, d.hz),
                    level: Level::Normal,
                });
            }
        }
    }

    out.push(Row::Act {
        label: "Settings".into(),
        value: "config.ini".into(),
        level: Level::Normal,
        button: "Edit".into(),
        action: Action::OpenSettingsFile,
    });
    out
}

/// Row heights at the dashboard's DPI.
fn heights(rows: &[Row], fonts: &Fonts, width: i32) -> Vec<i32> {
    let dc = MeasureDc::new(fonts.small);
    rows.iter()
        .map(|row| match row {
            Row::Header { .. } => fonts.px(34),
            Row::Line { .. } => fonts.px(22),
            Row::Spark { .. } => fonts.px(30),
            Row::Act { .. } => fonts.px(28),
            Row::Note { text, .. } => dc.paragraph_height(text, width - fonts.px(32)) + fonts.px(8),
        })
        .collect()
}

pub fn create(instance: HINSTANCE) -> Result<HWND, String> {
    let class = wide(CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(dash_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class.as_ptr(),
            wide("Stats").as_ptr(),
            WS_POPUP | WS_BORDER,
            0,
            0,
            WIDTH,
            400,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        return Err("could not create the dashboard window".into());
    }
    let corner = DWMWCP_ROUND;
    unsafe { DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const _ as *const _, 4) };
    Ok(hwnd)
}

pub fn toggle(app: &mut App) {
    if app.dash_visible() {
        hide(app);
    } else if super::app::now_ms().saturating_sub(app.dash_state.hidden_at) > 400 {
        show(app);
    }
}

pub fn hide(app: &mut App) {
    unsafe { ShowWindow(app.dash, SW_HIDE) };
}

pub fn show(app: &mut App) {
    let dpi = unsafe { GetDpiForWindow(if app.bar_hidden { app.main } else { app.bar }) };
    if app.dash_fonts.dpi != dpi.max(96) {
        app.dash_fonts = Fonts::new(dpi);
    }
    let fonts = &app.dash_fonts;
    let width = fonts.px(WIDTH);
    let content: i32 = heights(&rows(app), fonts, width).iter().sum::<i32>() + fonts.px(16);

    // Above the bar, right edges aligned; or above the tray when the bar is
    // hidden. Never taller than 72% of the screen: the rest scrolls.
    let anchor = if app.bar_hidden { app.main } else { app.bar };
    let monitor = unsafe { MonitorFromWindow(anchor, MONITOR_DEFAULTTOPRIMARY) };
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut info) };
    let work = info.rcWork;
    let height = content.min(((work.bottom - work.top) as f32 * 0.72) as i32);
    let gap = fonts.px(8);
    let (mut x, mut y) = (work.right - width - gap, work.bottom - height - gap);
    if !app.bar_hidden {
        let mut bar: RECT = unsafe { zeroed() };
        unsafe { GetWindowRect(app.bar, &mut bar) };
        x = bar.right - width;
        y = if bar.top - height - gap >= work.top { bar.top - height - gap } else { bar.bottom + gap };
    }
    x = x.clamp(work.left, (work.right - width).max(work.left));
    y = y.clamp(work.top, (work.bottom - height).max(work.top));
    app.dash_state.scroll = 0;
    app.dash_state.hot = None;
    unsafe {
        SetWindowPos(app.dash, HWND_TOPMOST, x, y, width, height, SWP_SHOWWINDOW);
        SetForegroundWindow(app.dash);
    }
    // Fresh process list straight away.
    app.refresh_groups();
}

fn paint(app: &mut App, hwnd: HWND) {
    let list = rows(app);
    let fonts = &app.dash_fonts;
    let mut canvas = Canvas::begin(hwnd);
    let full = RECT { left: 0, top: 0, right: canvas.width, bottom: canvas.height };
    canvas.fill(full, draw::BG);
    let sizes = heights(&list, fonts, canvas.width);
    let content: i32 = sizes.iter().sum::<i32>() + fonts.px(16);
    let max_scroll = (content - canvas.height).max(0);
    let state = &mut app.dash_state;
    state.scroll = state.scroll.clamp(0, max_scroll);
    state.content_height = content;
    state.hits.clear();

    let left = fonts.px(16);
    let right = canvas.width - fonts.px(16);
    let mut y = fonts.px(8) - state.scroll;
    for (row, h) in list.iter().zip(sizes) {
        let top = y;
        let bottom = y + h;
        y = bottom;
        if bottom < 0 || top > canvas.height {
            // Still register hits only for visible rows.
            continue;
        }
        match row {
            Row::Header { id, title, summary, collapsed } => {
                let band = RECT { left: 0, top: top + fonts.px(4), right: canvas.width, bottom };
                let hot = state.hot == Some(state.hits.len());
                canvas.fill(band, if hot { draw::PANEL } else { draw::BG });
                canvas.fill(RECT { left, top: top + fonts.px(4), right, bottom: top + fonts.px(5) }, draw::LINE);
                canvas.font(fonts.title);
                let arrow = if *collapsed { "▸" } else { "▾" };
                let x = canvas.text(arrow, left, band.top, band.bottom, draw::MUTED);
                canvas.text(title, x + fonts.px(6), band.top, band.bottom, draw::TEXT);
                canvas.font(fonts.normal);
                canvas.text_right(summary, right, band.top, band.bottom, draw::MUTED);
                state.hits.push((band, Hit::Section((*id).to_string())));
            }
            Row::Line { label, value, level } => {
                canvas.font(fonts.normal);
                canvas.text_clipped(label, left, right - fonts.px(200), top, bottom, draw::MUTED);
                canvas.font(fonts.bold);
                canvas.text_right(value, right, top, bottom, draw::level_color(*level));
            }
            Row::Spark { which, max, color } => {
                let ring: &Ring = match which {
                    Spark::Cpu => &app.hist.cpu,
                    Spark::Commit => &app.hist.commit,
                    Spark::Gpu => &app.hist.gpu,
                    Spark::NetDown => &app.hist.net_down,
                    Spark::Disk => &app.hist.disk,
                };
                let values: Vec<f32> = ring.values().collect();
                let area = RECT { left, top: top + fonts.px(3), right, bottom: bottom - fonts.px(3) };
                canvas.fill(area, draw::PANEL);
                canvas.sparkline(&values, *max, area, *color);
            }
            Row::Act { label, value, level, button, action } => {
                canvas.font(fonts.normal);
                let button_w = canvas.measure(button) + fonts.px(20);
                let b = RECT { left: right - button_w, top: top + fonts.px(3), right, bottom: bottom - fonts.px(3) };
                let hot = state.hot == Some(state.hits.len());
                canvas.round_rect(b, fonts.px(8), if hot { draw::BUTTON_HOT } else { draw::BUTTON }, Some(draw::LINE));
                canvas.text(button, b.left + fonts.px(10), b.top, b.bottom, draw::TEXT);
                let value_right = b.left - fonts.px(10);
                canvas.font(fonts.bold);
                let value_w = canvas.measure(value).min(fonts.px(190));
                canvas.text_clipped(value, value_right - value_w, value_right, top, bottom, draw::level_color(*level));
                canvas.font(fonts.normal);
                canvas.text_clipped(label, left, value_right - value_w - fonts.px(8), top, bottom, draw::MUTED);
                state.hits.push((b, Hit::Act(action.clone())));
            }
            Row::Note { text, color } => {
                canvas.font(fonts.small);
                let rect = RECT { left, top: top + fonts.px(4), right, bottom };
                canvas.paragraph(text, rect, *color, false);
            }
        }
    }
    // Scroll hint when more is below.
    if state.scroll < max_scroll {
        canvas.fill(
            RECT { left: 0, top: canvas.height - fonts.px(2), right: canvas.width, bottom: canvas.height },
            draw::ACCENT,
        );
    }
}

fn hit_at(app: &App, x: i32, y: i32) -> Option<usize> {
    app.dash_state.hits.iter().position(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

unsafe extern "system" fn dash_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let x = (lparam & 0xFFFF) as i16 as i32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_PAINT => with_app(|app| {
            paint(app, hwnd);
            0
        }),
        WM_ERASEBKGND => Some(1),
        WM_MOUSEWHEEL => {
            let delta = ((wparam >> 16) & 0xFFFF) as i16 as i32;
            with_app(|app| {
                let step = app.dash_fonts.px(48);
                app.dash_state.scroll = (app.dash_state.scroll - delta * step / 120).max(0);
                InvalidateRect(hwnd, null(), 0);
            });
            Some(0)
        }
        WM_MOUSEMOVE => {
            with_app(|app| {
                if !app.dash_state.tracking {
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    TrackMouseEvent(&mut tme);
                    app.dash_state.tracking = true;
                }
                let hot = hit_at(app, x, y);
                if hot != app.dash_state.hot {
                    app.dash_state.hot = hot;
                    InvalidateRect(hwnd, null(), 0);
                }
            });
            Some(0)
        }
        WM_MOUSELEAVE => {
            with_app(|app| {
                app.dash_state.tracking = false;
                app.dash_state.hot = None;
                InvalidateRect(hwnd, null(), 0);
            });
            Some(0)
        }
        WM_LBUTTONUP => {
            with_app(|app| {
                if let Some(i) = hit_at(app, x, y) {
                    let hit = app.dash_state.hits[i].1.clone();
                    match hit {
                        Hit::Section(id) => {
                            app.toggle_section(&id);
                            InvalidateRect(hwnd, null(), 0);
                        }
                        Hit::Act(action) => app.queue(action),
                    }
                }
            });
            Some(0)
        }
        WM_ACTIVATE => {
            if (wparam & 0xFFFF) as u32 == WA_INACTIVE {
                with_app(|app| {
                    if !app.modal && app.dash_visible() {
                        app.dash_state.hidden_at = super::app::now_ms();
                        hide(app);
                    }
                });
            }
            Some(0)
        }
        WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
            with_app(hide);
            Some(0)
        }
        WM_CLOSE => {
            with_app(hide);
            Some(0)
        }
        WM_DPICHANGED => {
            with_app(|app| app.dash_fonts = Fonts::new(GetDpiForWindow(hwnd)));
            None
        }
        _ => None,
    })
}
