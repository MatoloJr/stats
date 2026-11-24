//! The dashboard: a pop-up centred on the screen over a dimmed backdrop.
//! Six live tiles up top, then every reading in folding cards with a button
//! next to each problem that fixes it, then quick controls (position, pin,
//! start with Windows...). It closes on Esc, the close button, or a click
//! outside. Folded cards are remembered in `collapsed-sections`.

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
use super::app::{guarded, save_setting, with_app, App, History};
use super::draw::{self, glyph, Canvas, Fonts, MeasureDc};
use super::util::{self, wide};
use super::{bar, line};
use stats_core::advice::{self, Fix};
use stats_core::config::Level;
use stats_core::history::Ring;
use stats_core::numbers::{fmt_bytes, fmt_percent, fmt_rate, fmt_temp};
use stats_core::pagefile::PageFile;
use stats_core::place::{self, Anchor};

const CLASS: &str = "StatsForWindowsDashboard";
const SCRIM_CLASS: &str = "StatsForWindowsScrim";
const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDTH: i32 = 720;
/// Backdrop darkness, 0..255.
const SCRIM_ALPHA: u8 = 125;

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
    /// Start-with-Windows state, read when the dashboard opens.
    autostart: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Position(Anchor),
    Pin,
    Autostart,
    HideFullscreen,
    Settings,
    Log,
    Scan,
    Exit,
    Close,
}

#[derive(Clone, Debug)]
pub enum Hit {
    Section(String),
    Act(Action),
    Control(Control),
    /// A tile: scroll to that card.
    Jump(&'static str),
}

#[derive(Clone, Copy)]
enum Spark {
    Cpu,
    Memory,
    Commit,
    Gpu,
    NetDown,
    Disk,
}

struct Tile {
    label: String,
    value: String,
    detail: String,
    level: Level,
    spark: Spark,
    max: f32,
    section: &'static str,
}

enum Row {
    /// Title, status chip, settings and close buttons.
    Hero,
    Tiles(Vec<Tile>),
    Header {
        id: &'static str,
        title: String,
        summary: String,
        collapsed: bool,
    },
    Line {
        label: String,
        value: String,
        level: Level,
    },
    Spark {
        which: Spark,
        max: f32,
        color: COLORREF,
    },
    Act {
        label: String,
        value: String,
        level: Level,
        button: String,
        action: Action,
    },
    Note {
        text: String,
        color: COLORREF,
    },
    Positions,
    Toggle {
        label: String,
        detail: String,
        on: bool,
        control: Control,
    },
    Links,
    /// Bottom of a card and the space before the next one.
    CardEnd,
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

/// One phrase for the header chip: the most serious thing going on.
fn status(app: &App) -> (String, Level) {
    let s = &app.snap;
    let c = &app.config;
    let drive = s.drives.first();
    let candidates = [
        (c.commit.level(s.commit), "Commit high".to_string()),
        (c.memory.level(s.memory), "RAM high".to_string()),
        (
            c.storage.level(drive.and_then(|d| d.percent)),
            format!("{} almost full", drive.map(|d| d.name.as_str()).unwrap_or("Disk")),
        ),
        (c.cpu.level(s.cpu), "CPU busy".to_string()),
        (c.temp.level(s.temperature), "Running hot".to_string()),
        (c.gpu.level(s.gpu), "GPU busy".to_string()),
    ];
    candidates
        .into_iter()
        .filter(|(level, _)| *level != Level::Normal)
        .max_by_key(|(level, _)| *level)
        .map(|(level, text)| (text, level))
        .unwrap_or_else(|| ("All good".to_string(), Level::Normal))
}

fn tiles(app: &App) -> Vec<Tile> {
    let s = &app.snap;
    let c = &app.config;
    let busiest = app
        .groups
        .iter()
        .filter(|g| g.cpu >= 1.0)
        .max_by(|a, b| a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal))
        .map(|g| format!("Busiest: {}", advice::display_name(&g.key)))
        .unwrap_or_else(|| "Processor load".into());
    let drive = s.drives.first();
    vec![
        Tile {
            label: "CPU".into(),
            value: fmt_percent(s.cpu),
            detail: busiest,
            level: c.cpu.level(s.cpu),
            spark: Spark::Cpu,
            max: 100.0,
            section: "cpu",
        },
        Tile {
            label: "RAM".into(),
            value: fmt_percent(s.memory),
            detail: format!("{} of {}", fmt_bytes(s.ram_used), fmt_bytes(s.ram_total)),
            level: c.memory.level(s.memory),
            spark: Spark::Memory,
            max: 100.0,
            section: "memory",
        },
        Tile {
            label: "Commit".into(),
            value: fmt_percent(s.commit),
            detail: format!("{} of {} limit", fmt_bytes(s.commit_used), fmt_bytes(s.commit_limit)),
            level: c.commit.level(s.commit),
            spark: Spark::Commit,
            max: 100.0,
            section: "memory",
        },
        Tile {
            label: drive.map(|d| format!("Disk {}", d.name)).unwrap_or_else(|| "Disk".into()),
            value: fmt_percent(drive.and_then(|d| d.percent)),
            detail: drive
                .map(|d| format!("{} free · {}", fmt_bytes(d.total.saturating_sub(d.used)), fmt_rate(s.disk_io())))
                .unwrap_or_else(|| "n/a".into()),
            level: c.storage.level(drive.and_then(|d| d.percent)),
            spark: Spark::Disk,
            max: app.hist.disk.max().unwrap_or(1.0).max(1.0),
            section: "storage",
        },
        Tile {
            label: "GPU".into(),
            value: fmt_percent(s.gpu),
            detail: s.gpu_dedicated.map(|b| format!("{} dedicated", fmt_bytes(b))).unwrap_or_else(|| "Graphics".into()),
            level: c.gpu.level(s.gpu),
            spark: Spark::Gpu,
            max: 100.0,
            section: "gpu",
        },
        Tile {
            label: "Network".into(),
            value: format!("↓ {}", fmt_rate(s.net_down)),
            detail: format!("↑ {}", fmt_rate(s.net_up)),
            level: Level::Normal,
            spark: Spark::NetDown,
            max: app.hist.net_down.max().unwrap_or(1.0).max(1.0),
            section: "network",
        },
    ]
}

fn rows(app: &App) -> Vec<Row> {
    let s = &app.snap;
    let c = &app.config;
    let collapsed = |id: &str| c.collapsed_sections.iter().any(|x| x == id);
    let mut out = vec![Row::Hero, Row::Tiles(tiles(app))];
    // Opens a card; returns whether its content should follow.
    let header = |out: &mut Vec<Row>, id: &'static str, title: &str, summary: String| -> bool {
        let folded = collapsed(id);
        out.push(Row::Header { id, title: title.into(), summary, collapsed: folded });
        if folded {
            out.push(Row::CardEnd);
        }
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
        out.push(Row::Spark { which: Spark::Commit, max: 100.0, color: draw::signal_color(commit_level) });
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
        out.push(Row::CardEnd);
    }

    if header(&mut out, "cpu", "CPU", fmt_percent(s.cpu)) {
        out.push(Row::Line { label: "Busy".into(), value: fmt_percent(s.cpu), level: c.cpu.level(s.cpu) });
        out.push(Row::Spark { which: Spark::Cpu, max: 100.0, color: draw::signal_color(c.cpu.level(s.cpu)) });
        let mut by_cpu: Vec<_> = app.groups.iter().filter(|g| g.cpu >= 0.5).collect();
        by_cpu.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
        for group in by_cpu.into_iter().take(3) {
            out.push(Row::Line {
                label: advice::display_name(&group.key),
                value: format!("{:.0}%", group.cpu),
                level: Level::Normal,
            });
        }
        out.push(Row::CardEnd);
    }

    if header(&mut out, "gpu", "GPU", fmt_percent(s.gpu)) {
        out.push(Row::Line { label: "Busy".into(), value: fmt_percent(s.gpu), level: c.gpu.level(s.gpu) });
        if let Some(bytes) = s.gpu_dedicated {
            out.push(Row::Line { label: "Dedicated memory".into(), value: fmt_bytes(bytes), level: Level::Normal });
        }
        out.push(Row::Spark { which: Spark::Gpu, max: 100.0, color: draw::ACCENT });
        out.push(Row::CardEnd);
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
        out.push(Row::CardEnd);
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
        out.push(Row::CardEnd);
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
        out.push(Row::CardEnd);
    }

    if header(&mut out, "network", "Network", format!("↓ {}", fmt_rate(s.net_down))) {
        out.push(Row::Line { label: "Download".into(), value: fmt_rate(s.net_down), level: Level::Normal });
        out.push(Row::Line { label: "Upload".into(), value: fmt_rate(s.net_up), level: Level::Normal });
        out.push(Row::Spark {
            which: Spark::NetDown,
            max: app.hist.net_down.max().unwrap_or(1.0).max(1.0),
            color: draw::ACCENT,
        });
        out.push(Row::CardEnd);
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
            out.push(Row::CardEnd);
        }
    }

    // What the right-click menu used to hold.
    if header(&mut out, "controls", "Quick controls", c.bar_position.label().to_string()) {
        out.push(Row::Positions);
        out.push(Row::Toggle {
            label: "Pin readings".into(),
            detail: "Keep the strip under the line open".into(),
            on: c.bar_pinned,
            control: Control::Pin,
        });
        out.push(Row::Toggle {
            label: "Start with Windows".into(),
            detail: "Run Stats when you sign in".into(),
            on: app.dash_state.autostart,
            control: Control::Autostart,
        });
        out.push(Row::Toggle {
            label: "Hide in full screen".into(),
            detail: "Games and videos get the whole screen".into(),
            on: c.hide_when_fullscreen,
            control: Control::HideFullscreen,
        });
        out.push(Row::Links);
        out.push(Row::CardEnd);
    }
    out
}

/// Row heights at the dashboard's DPI.
fn heights(rows: &[Row], fonts: &Fonts, width: i32) -> Vec<i32> {
    let dc = MeasureDc::new(fonts.small);
    rows.iter()
        .map(|row| match row {
            Row::Hero => fonts.px(60),
            Row::Tiles(t) => {
                let lines = (t.len() as i32 + 2) / 3;
                lines * fonts.px(104) + fonts.px(6)
            }
            Row::Header { .. } => fonts.px(42),
            Row::Line { .. } => fonts.px(26),
            Row::Spark { .. } => fonts.px(34),
            Row::Act { .. } => fonts.px(32),
            Row::Note { text, .. } => dc.paragraph_height(text, width - fonts.px(64)) + fonts.px(10),
            Row::Positions => fonts.px(44),
            Row::Toggle { .. } => fonts.px(44),
            Row::Links => fonts.px(52),
            Row::CardEnd => fonts.px(20),
        })
        .collect()
}

pub fn create(instance: HINSTANCE) -> Result<(HWND, HWND), String> {
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
            WS_POPUP,
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

    let scrim_class = wide(SCRIM_CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(scrim_proc),
        hInstance: instance,
        lpszClassName: scrim_class.as_ptr(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: unsafe { GetStockObject(BLACK_BRUSH) } as HBRUSH,
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let scrim = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_NOACTIVATE,
            scrim_class.as_ptr(),
            wide("Stats backdrop").as_ptr(),
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
    if scrim.is_null() {
        return Err("could not create the dashboard backdrop".into());
    }
    unsafe { SetLayeredWindowAttributes(scrim, 0, SCRIM_ALPHA, LWA_ALPHA) };
    Ok((hwnd, scrim))
}

pub fn toggle(app: &mut App) {
    if app.dash_visible() {
        hide(app);
    } else if super::app::now_ms().saturating_sub(app.dash_state.hidden_at) > 400 {
        show(app);
    }
}

pub fn hide(app: &mut App) {
    unsafe {
        ShowWindow(app.dash, SW_HIDE);
        ShowWindow(app.scrim, SW_HIDE);
    }
}

pub fn show(app: &mut App) {
    let dpi = unsafe { GetDpiForWindow(app.line.hwnd) };
    if app.dash_fonts.dpi != dpi.max(96) {
        app.dash_fonts = Fonts::new(dpi);
    }
    app.dash_state.autostart = util::autostart_enabled();
    line::release(app);
    bar::release(app);
    let fonts = &app.dash_fonts;
    let width = fonts.px(WIDTH);
    let content: i32 = heights(&rows(app), fonts, width).iter().sum::<i32>() + fonts.px(16);

    // Centred on the line's monitor, never taller than 80% of it.
    let monitor = unsafe { MonitorFromWindow(app.line.hwnd, MONITOR_DEFAULTTOPRIMARY) };
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut info) };
    let w = info.rcWork;
    let r = place::centered(place::Rect::new(w.left, w.top, w.right, w.bottom), width, content, 0.8);
    let m = info.rcMonitor;
    app.dash_state.scroll = 0;
    app.dash_state.hot = None;
    unsafe {
        SetWindowPos(
            app.scrim,
            HWND_TOPMOST,
            m.left,
            m.top,
            m.right - m.left,
            m.bottom - m.top,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        SetWindowPos(app.dash, HWND_TOPMOST, r.left, r.top, r.width(), r.height(), SWP_SHOWWINDOW);
        SetForegroundWindow(app.dash);
        InvalidateRect(app.dash, null(), 0);
    }
    // Fresh process list straight away.
    app.refresh_groups();
}

/// Scroll so the card `id` starts near the top.
fn jump(app: &mut App, id: &str) {
    let list = rows(app);
    let fonts = &app.dash_fonts;
    let mut y = fonts.px(8);
    for (row, h) in list.iter().zip(heights(&list, fonts, fonts.px(WIDTH))) {
        if matches!(row, Row::Header { id: rid, .. } if *rid == id) {
            app.dash_state.scroll = (y - fonts.px(8)).max(0);
            return;
        }
        y += h;
    }
}

fn spark_ring(hist: &History, which: Spark) -> &Ring {
    match which {
        Spark::Cpu => &hist.cpu,
        Spark::Memory => &hist.memory,
        Spark::Commit => &hist.commit,
        Spark::Gpu => &hist.gpu,
        Spark::NetDown => &hist.net_down,
        Spark::Disk => &hist.disk,
    }
}

fn paint(app: &mut App, hwnd: HWND) {
    let list = rows(app);
    let (status_text, status_level) = status(app);
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

    let margin = fonts.px(18);
    let (card_l, card_r) = (margin, canvas.width - margin);
    let left = card_l + fonts.px(16);
    let right = card_r - fonts.px(16);
    let radius = fonts.px(14);

    // Card backgrounds first, one rounded shape per card, so no row is ever
    // painted over by its card's corners.
    let mut y = fonts.px(8) - state.scroll;
    let mut card_top = None;
    for (row, &h) in list.iter().zip(&sizes) {
        match row {
            Row::Header { .. } => card_top = Some(y),
            Row::CardEnd => {
                if let Some(start) = card_top.take() {
                    let end = y + fonts.px(10);
                    if end > 0 && start < canvas.height {
                        canvas.round_rect(
                            RECT { left: card_l, top: start, right: card_r, bottom: end },
                            radius,
                            draw::PANEL,
                            None,
                        );
                    }
                }
            }
            _ => {}
        }
        y += h;
    }

    let mut y = fonts.px(8) - state.scroll;
    for (row, h) in list.iter().zip(sizes) {
        let top = y;
        let bottom = y + h;
        y = bottom;
        if bottom < 0 || top > canvas.height {
            continue;
        }
        // Hit-test index this row's first target will get.
        let next = state.hits.len();
        let hot_now = state.hot;
        let is_hot = move |offset: usize| hot_now == Some(next + offset);
        match row {
            Row::Hero => {
                canvas.font(fonts.title);
                let x = canvas.text("Stats", card_l + fonts.px(4), top, bottom, draw::TEXT);
                // Status chip.
                canvas.font(fonts.small);
                let chip_w = canvas.measure(&status_text) + fonts.px(30);
                let mid = (top + bottom) / 2;
                let chip = RECT {
                    left: x + fonts.px(12),
                    top: mid - fonts.px(12),
                    right: x + fonts.px(12) + chip_w,
                    bottom: mid + fonts.px(12),
                };
                let (chip_bg, chip_fg) = match status_level {
                    Level::Normal => (draw::ACCENT_DIM, draw::ACCENT),
                    level => (draw::BUTTON, draw::level_color(level)),
                };
                canvas.round_rect(chip, fonts.px(24), chip_bg, None);
                let dot = RECT {
                    left: chip.left + fonts.px(10),
                    top: mid - fonts.px(3),
                    right: chip.left + fonts.px(16),
                    bottom: mid + fonts.px(3),
                };
                canvas.round_rect(dot, fonts.px(6), chip_fg, None);
                canvas.text(&status_text, dot.right + fonts.px(6), chip.top, chip.bottom, chip_fg);
                // Settings and close.
                let size = fonts.px(34);
                let mut bx = card_r;
                for (i, (g, control)) in
                    [(glyph::CLOSE, Control::Close), (glyph::SETTINGS, Control::Settings)].into_iter().enumerate()
                {
                    let b = RECT { left: bx - size, top: mid - size / 2, right: bx, bottom: mid + size / 2 };
                    if is_hot(i) {
                        canvas.round_rect(b, fonts.px(10), draw::BUTTON_HOT, None);
                    }
                    canvas.icon(fonts, g, b, if is_hot(i) { draw::TEXT } else { draw::MUTED });
                    state.hits.push((b, Hit::Control(control)));
                    bx = b.left - fonts.px(4);
                }
            }
            Row::Tiles(tiles) => {
                let gap = fonts.px(10);
                let tile_w = (card_r - card_l - 2 * gap) / 3;
                let tile_h = fonts.px(96);
                for (i, t) in tiles.iter().enumerate() {
                    let col = (i % 3) as i32;
                    let line = (i / 3) as i32;
                    let r = RECT {
                        left: card_l + col * (tile_w + gap),
                        top: top + line * (tile_h + fonts.px(8)),
                        right: card_l + col * (tile_w + gap) + tile_w,
                        bottom: top + line * (tile_h + fonts.px(8)) + tile_h,
                    };
                    let hot = is_hot(i);
                    let edge = if t.level == Level::Normal { None } else { Some(draw::level_color(t.level)) };
                    canvas.round_rect(r, radius, if hot { draw::PANEL_HOT } else { draw::PANEL }, edge);
                    let inset = fonts.px(14);
                    canvas.font(fonts.small);
                    canvas.text(&t.label, r.left + inset, r.top + fonts.px(8), r.top + fonts.px(26), draw::MUTED);
                    canvas.font(fonts.big);
                    canvas.text_clipped(
                        &t.value,
                        r.left + inset,
                        r.right - inset,
                        r.top + fonts.px(24),
                        r.top + fonts.px(54),
                        draw::level_color(t.level),
                    );
                    canvas.font(fonts.small);
                    canvas.text_clipped(
                        &t.detail,
                        r.left + inset,
                        r.right - inset,
                        r.top + fonts.px(52),
                        r.top + fonts.px(70),
                        draw::MUTED,
                    );
                    let values: Vec<f32> = spark_ring(&app.hist, t.spark).values().collect();
                    let spark = RECT {
                        left: r.left + inset,
                        top: r.bottom - fonts.px(24),
                        right: r.right - inset,
                        bottom: r.bottom - fonts.px(8),
                    };
                    canvas.sparkline(&values, t.max, spark, draw::signal_color(t.level));
                    state.hits.push((r, Hit::Jump(t.section)));
                }
            }
            Row::Header { id, title, summary, collapsed } => {
                let band = RECT { left: card_l, top, right: card_r, bottom };
                let hot = is_hot(0);
                if hot {
                    let inset = fonts.px(4);
                    canvas.round_rect(
                        RECT {
                            left: card_l + inset,
                            top: top + inset,
                            right: card_r - inset,
                            bottom: bottom - inset / 2,
                        },
                        radius - inset,
                        draw::PANEL_HOT,
                        None,
                    );
                }
                let chevron = if *collapsed { glyph::CHEVRON_RIGHT } else { glyph::CHEVRON_DOWN };
                let icon_box = RECT { left, top, right: left + fonts.px(16), bottom };
                canvas.icon(fonts, chevron, icon_box, draw::MUTED);
                canvas.font(fonts.title);
                canvas.text(title, icon_box.right + fonts.px(8), top, bottom, draw::TEXT);
                canvas.font(fonts.normal);
                canvas.text_right(summary, right, top, bottom, draw::MUTED);
                state.hits.push((band, Hit::Section((*id).to_string())));
            }
            Row::Line { label, value, level } => {
                canvas.font(fonts.normal);
                canvas.text_clipped(label, left, right - fonts.px(260), top, bottom, draw::MUTED);
                canvas.font(fonts.bold);
                canvas.text_right(value, right, top, bottom, draw::level_color(*level));
            }
            Row::Spark { which, max, color } => {
                let values: Vec<f32> = spark_ring(&app.hist, *which).values().collect();
                let area = RECT { left, top: top + fonts.px(4), right, bottom: bottom - fonts.px(4) };
                canvas.round_rect(area, fonts.px(8), draw::BG, None);
                canvas.sparkline(
                    &values,
                    *max,
                    RECT { left: area.left + fonts.px(4), right: area.right - fonts.px(4), ..area },
                    *color,
                );
            }
            Row::Act { label, value, level, button, action } => {
                canvas.font(fonts.normal);
                let button_w = canvas.measure(button) + fonts.px(24);
                let b = RECT { left: right - button_w, top: top + fonts.px(4), right, bottom: bottom - fonts.px(4) };
                let hot = is_hot(0);
                canvas.round_rect(
                    b,
                    fonts.px(10),
                    if hot { draw::BUTTON_HOT } else { draw::BUTTON },
                    Some(if hot { draw::ACCENT } else { draw::LINE }),
                );
                canvas.text_center(button, b, if hot { draw::ACCENT } else { draw::TEXT });
                let value_right = b.left - fonts.px(12);
                canvas.font(fonts.bold);
                let value_w = canvas.measure(value).min(fonts.px(240));
                canvas.text_clipped(value, value_right - value_w, value_right, top, bottom, draw::level_color(*level));
                canvas.font(fonts.normal);
                canvas.text_clipped(label, left, value_right - value_w - fonts.px(10), top, bottom, draw::MUTED);
                state.hits.push((b, Hit::Act(action.clone())));
            }
            Row::Note { text, color } => {
                canvas.font(fonts.small);
                let rect =
                    RECT { left: card_l + fonts.px(4), top: top + fonts.px(4), right: card_r - fonts.px(4), bottom };
                canvas.paragraph(text, rect, *color, false);
            }
            Row::Positions => {
                canvas.font(fonts.normal);
                canvas.text("Line position", left, top, bottom, draw::TEXT);
                let seg_w = fonts.px(78);
                let seg_h = fonts.px(30);
                let mid = (top + bottom) / 2;
                let total = RECT {
                    left: right - 3 * seg_w - fonts.px(8),
                    top: mid - seg_h / 2 - fonts.px(4),
                    right,
                    bottom: mid + seg_h / 2 + fonts.px(4),
                };
                canvas.round_rect(total, fonts.px(12), draw::BG, Some(draw::LINE));
                for (i, anchor) in Anchor::ALL.into_iter().enumerate() {
                    let l = total.left + fonts.px(4) + i as i32 * seg_w;
                    let r = RECT { left: l, top: mid - seg_h / 2, right: l + seg_w, bottom: mid + seg_h / 2 };
                    let selected = app.config.bar_position == anchor;
                    let hot = is_hot(i);
                    if selected {
                        canvas.round_rect(r, fonts.px(10), draw::ACCENT_DIM, None);
                    } else if hot {
                        canvas.round_rect(r, fonts.px(10), draw::BUTTON_HOT, None);
                    }
                    canvas.font(if selected { fonts.bold } else { fonts.normal });
                    canvas.text_center(anchor.label(), r, if selected { draw::ACCENT } else { draw::TEXT });
                    state.hits.push((r, Hit::Control(Control::Position(anchor))));
                }
            }
            Row::Toggle { label, detail, on, control } => {
                let hot = is_hot(0);
                let mid = (top + bottom) / 2;
                canvas.font(fonts.normal);
                canvas.text(label, left, top + fonts.px(4), mid + fonts.px(2), draw::TEXT);
                canvas.font(fonts.small);
                canvas.text(detail, left, mid, bottom - fonts.px(4), draw::MUTED);
                // Switch.
                let (sw, sh) = (fonts.px(40), fonts.px(22));
                let s = RECT { left: right - sw, top: mid - sh / 2, right, bottom: mid + sh / 2 };
                let track = if *on {
                    draw::ACCENT
                } else if hot {
                    draw::BUTTON_HOT
                } else {
                    draw::BUTTON
                };
                canvas.round_rect(s, sh, track, Some(if *on { draw::ACCENT } else { draw::LINE }));
                let knob = fonts.px(16);
                let kx = if *on { s.right - knob - fonts.px(3) } else { s.left + fonts.px(3) };
                canvas.round_rect(
                    RECT { left: kx, top: mid - knob / 2, right: kx + knob, bottom: mid + knob / 2 },
                    knob,
                    if *on { draw::BG } else { draw::MUTED },
                    None,
                );
                state.hits.push((RECT { left: card_l, top, right: card_r, bottom }, Hit::Control(*control)));
            }
            Row::Links => {
                let mid = (top + bottom) / 2;
                let bh = fonts.px(32);
                let mut x = left;
                for (i, (g, text, control)) in [
                    (glyph::SEARCH, "Scan build folders", Control::Scan),
                    (glyph::SETTINGS, "Edit settings", Control::Settings),
                    (glyph::DOCUMENT, "Open log", Control::Log),
                    (glyph::POWER, "Exit Stats", Control::Exit),
                ]
                .into_iter()
                .enumerate()
                {
                    canvas.font(fonts.normal);
                    let w = canvas.measure(text) + fonts.px(44);
                    let b = RECT { left: x, top: mid - bh / 2, right: x + w, bottom: mid + bh / 2 };
                    let hot = is_hot(i);
                    canvas.round_rect(
                        b,
                        fonts.px(10),
                        if hot { draw::BUTTON_HOT } else { draw::BUTTON },
                        Some(draw::LINE),
                    );
                    let color = if control == Control::Exit && hot {
                        draw::CRIT
                    } else if hot {
                        draw::ACCENT
                    } else {
                        draw::TEXT
                    };
                    canvas.icon(
                        fonts,
                        g,
                        RECT { left: b.left + fonts.px(8), right: b.left + fonts.px(28), ..b },
                        color,
                    );
                    canvas.font(fonts.normal);
                    canvas.text(text, b.left + fonts.px(32), b.top, b.bottom, color);
                    state.hits.push((b, Hit::Control(control)));
                    x = b.right + fonts.px(8);
                }
            }
            // Drawn in the first pass.
            Row::CardEnd => {}
        }
    }
    // Scroll hint when more is below.
    if state.scroll < max_scroll {
        canvas.fill(
            RECT { left: 0, top: canvas.height - fonts.px(3), right: canvas.width, bottom: canvas.height },
            draw::ACCENT,
        );
    }
}

fn hit_at(app: &App, x: i32, y: i32) -> Option<usize> {
    app.dash_state.hits.iter().position(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

/// Run a quick control. Called with the app released, since some of these
/// open other programs or close Stats.
fn control(which: Control) {
    match which {
        Control::Close => {
            with_app(hide);
        }
        Control::Settings => {
            with_app(|app| app.queue(Action::OpenSettingsFile));
        }
        Control::Scan => {
            with_app(|app| app.queue(Action::ScanJunk));
        }
        Control::Log => util::shell_open("notepad.exe", Some(&format!("\"{}\"", util::log_path().display()))),
        Control::Exit => {
            if let Some(main) = with_app(|app| app.main) {
                unsafe { PostMessageW(main, WM_CLOSE, 0, 0) };
            }
        }
        Control::Autostart => {
            let on = util::autostart_enabled();
            if !util::set_autostart(!on) {
                util::log("could not change the startup setting");
            }
            with_app(|app| app.dash_state.autostart = util::autostart_enabled());
        }
        Control::Pin => {
            with_app(|app| {
                let pinned = !app.config.bar_pinned;
                bar::set_pinned(app, pinned);
            });
        }
        Control::HideFullscreen => {
            with_app(|app| {
                app.config.hide_when_fullscreen = !app.config.hide_when_fullscreen;
                let value = app.config.hide_when_fullscreen.to_string();
                save_setting(app, "hide-when-fullscreen", &value);
            });
        }
        Control::Position(anchor) => {
            with_app(|app| set_position(app, anchor));
        }
    }
    with_app(|app| unsafe {
        InvalidateRect(app.dash, null(), 0);
    });
}

pub fn set_position(app: &mut App, anchor: Anchor) {
    app.config.bar_position = anchor;
    save_setting(app, "bar-position", anchor.key());
    line::place(app);
    if app.strip.open {
        bar::relayout(app);
    }
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
                    let cursor = if hot.is_some() { IDC_HAND } else { IDC_ARROW };
                    SetCursor(LoadCursorW(null_mut(), cursor));
                    InvalidateRect(hwnd, null(), 0);
                }
            });
            Some(0)
        }
        WM_SETCURSOR => {
            let hot = with_app(|app| app.dash_state.hot.is_some()).unwrap_or(false);
            SetCursor(LoadCursorW(null_mut(), if hot { IDC_HAND } else { IDC_ARROW }));
            Some(1)
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
            let hit = with_app(|app| hit_at(app, x, y).map(|i| app.dash_state.hits[i].1.clone())).flatten();
            match hit {
                Some(Hit::Section(id)) => {
                    with_app(|app| {
                        app.toggle_section(&id);
                        InvalidateRect(hwnd, null(), 0);
                    });
                }
                Some(Hit::Act(action)) => {
                    with_app(|app| app.queue(action));
                }
                Some(Hit::Jump(id)) => {
                    with_app(|app| {
                        jump(app, id);
                        InvalidateRect(hwnd, null(), 0);
                    });
                }
                Some(Hit::Control(c)) => control(c),
                None => {}
            }
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

/// The dimmed backdrop: a click on it closes the dashboard.
unsafe extern "system" fn scrim_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    guarded(hwnd, msg, wparam, lparam, || match msg {
        WM_MOUSEACTIVATE => Some(MA_NOACTIVATE as LRESULT),
        WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
            with_app(|app| {
                app.dash_state.hidden_at = super::app::now_ms();
                hide(app);
            });
            Some(0)
        }
        _ => None,
    })
}
