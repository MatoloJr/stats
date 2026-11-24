//! The dashboard: a frosted pop-up centred on the screen, with two tabs.
//! **Overview**: six live tiles, then every reading in folding cards with a
//! button next to each problem that fixes it. **Settings**: every option in
//! `config.ini` as switches, steppers and chips (see `settings.rs`).
//!
//! Keyboard: Esc closes; 1 / 2 or Ctrl+Tab switch tabs; Tab and Shift+Tab
//! move a focus ring, Enter or Space press the focused control; arrows,
//! Page Up / Page Down, Home and End scroll. It also closes on a click
//! outside. Folded cards are remembered in `collapsed-sections`.

use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::actions::{short_path, Action};
use super::app::{guarded, with_app, App, History};
use super::draw::{self, glyph, Canvas, Fonts, MeasureDc};
use super::util::{self, wide};
use super::{bar, hotkeys, line, settings};
use stats_core::advice::{self, Fix};
use stats_core::config::{Level, Metric};
use stats_core::history::Ring;
use stats_core::hotkey::Command;
use stats_core::numbers::{fmt_bytes, fmt_percent, fmt_rate, fmt_temp, TempUnit};
use stats_core::pagefile::PageFile;
use stats_core::place::{self, Anchor};

const CLASS: &str = "StatsForWindowsDashboard";
const WM_MOUSELEAVE: u32 = 0x02A3;
const WIDTH: i32 = 720;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Overview,
    Settings,
}

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
    pub autostart: bool,
    pub tab: Tab,
    /// Keyboard focus: an index into `hits`.
    pub focus: Option<usize>,
    /// Waiting for a key combination for this shortcut.
    pub recording: Option<Command>,
    /// Frosted glass is on.
    pub glass: bool,
}

/// Something a control in the dashboard does. Run by `settings::run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Tab(Tab),
    Position(Anchor),
    Pin,
    Autostart,
    HideFullscreen,
    ShowLine,
    ShowSpeeds,
    Notifications,
    PagefileCheck,
    Unit(TempUnit),
    /// A stepper: setting key and direction (-1 or +1).
    Step(&'static str, i64),
    Metric(Metric),
    Drive(char),
    AddRoot,
    RemoveRoot(usize),
    Record(Command),
    ClearHotkey(Command),
    DefaultHotkey(Command),
    ResetDefaults,
    EditFile,
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
pub enum Spark {
    Cpu,
    Memory,
    Commit,
    Gpu,
    NetDown,
    Disk,
}

pub struct Tile {
    label: String,
    value: String,
    detail: String,
    level: Level,
    spark: Spark,
    max: f32,
    section: &'static str,
}

/// One shortcut's state, for its Settings row.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Off,
    Ready,
    InUse,
}

pub enum Row {
    /// Title, status chip, tabs, settings and close buttons.
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
    /// A segmented choice: options with their control and whether selected.
    Choice {
        label: String,
        detail: String,
        options: Vec<(String, Control, bool)>,
    },
    Toggle {
        label: String,
        detail: String,
        on: bool,
        control: Control,
    },
    /// − value + for one number setting.
    Stepper {
        label: String,
        detail: String,
        value: String,
        key: &'static str,
    },
    /// Warning and critical steppers for one metric on one row.
    Levels {
        label: String,
        warning: (String, &'static str),
        critical: (String, &'static str),
    },
    /// Toggle chips that wrap onto several lines.
    Chips {
        label: String,
        items: Vec<(String, bool, Control)>,
    },
    Folder {
        path: String,
        exists: bool,
        index: usize,
    },
    Buttons(Vec<(char, String, Control)>),
    Shortcut {
        command: Command,
        keys: String,
        status: KeyStatus,
        recording: bool,
    },
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

fn overview_rows(app: &App) -> Vec<Row> {
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

    out
}

fn rows(app: &App) -> Vec<Row> {
    match app.dash_state.tab {
        Tab::Overview => overview_rows(app),
        Tab::Settings => {
            let mut out = vec![Row::Hero];
            out.extend(settings::rows(app));
            out
        }
    }
}

/// Chip rectangles for a `Chips` row, relative to its top-left content
/// corner, wrapping at `width`. Shared by layout and painting.
fn chip_rects(fonts: &Fonts, items: &[(String, bool, Control)], width: i32) -> Vec<RECT> {
    let dc = MeasureDc::new(fonts.normal);
    let (h, gap) = (fonts.px(30), fonts.px(8));
    let (mut x, mut y) = (0, 0);
    let mut out = Vec::with_capacity(items.len());
    for (text, _, _) in items {
        let w = dc.width(text) + fonts.px(40);
        if x > 0 && x + w > width {
            x = 0;
            y += h + gap;
        }
        out.push(RECT { left: x, top: y, right: x + w, bottom: y + h });
        x += w + gap;
    }
    out
}

/// Row heights at the dashboard's DPI.
fn heights(rows: &[Row], fonts: &Fonts, width: i32) -> Vec<i32> {
    let dc = MeasureDc::new(fonts.small);
    let inner = width - 2 * fonts.px(18) - 2 * fonts.px(16);
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
            Row::Choice { .. } | Row::Toggle { .. } | Row::Stepper { .. } | Row::Levels { .. } => fonts.px(48),
            Row::Chips { items, .. } => {
                let bottom = chip_rects(fonts, items, inner).last().map(|r| r.bottom).unwrap_or(0);
                fonts.px(30) + bottom + fonts.px(10)
            }
            Row::Folder { .. } => fonts.px(36),
            Row::Buttons(_) => fonts.px(52),
            Row::Shortcut { .. } => fonts.px(48),
            Row::CardEnd => fonts.px(20),
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
    Ok(hwnd)
}

/// Frosted glass behind the dashboard (also sets rounded corners).
pub fn enable_glass(app: &mut App) {
    app.dash_state.glass = super::glass::enable(app.dash);
}

pub fn toggle(app: &mut App) {
    if app.dash_visible() {
        hide(app);
    } else if super::app::now_ms().saturating_sub(app.dash_state.hidden_at) > 400 {
        show(app);
    }
}

pub fn hide(app: &mut App) {
    if app.dash_state.recording.take().is_some() {
        hotkeys::register_all(app);
    }
    unsafe { ShowWindow(app.dash, SW_HIDE) };
}

/// Open on a given tab (or switch tab if it is already open).
pub fn show_tab(app: &mut App, tab: Tab) {
    app.dash_state.tab = tab;
    show(app);
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
    // Size for the taller tab, so switching tabs never resizes the window.
    let tab = app.dash_state.tab;
    let mut content = 0;
    for t in [Tab::Overview, Tab::Settings] {
        app.dash_state.tab = t;
        let fonts = &app.dash_fonts;
        content = content.max(heights(&rows(app), fonts, width).iter().sum::<i32>() + fonts.px(16));
    }
    app.dash_state.tab = tab;

    // Centred on the line's monitor, never taller than 80% of it.
    let monitor = unsafe { MonitorFromWindow(app.line.hwnd, MONITOR_DEFAULTTOPRIMARY) };
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    unsafe { GetMonitorInfoW(monitor, &mut info) };
    let w = info.rcWork;
    let r = place::centered(place::Rect::new(w.left, w.top, w.right, w.bottom), width, content, 0.8);
    app.dash_state.scroll = 0;
    app.dash_state.hot = None;
    app.dash_state.focus = None;
    unsafe {
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

/// A small button: rounded, hot state, centred label or glyph.
fn small_button(canvas: &mut Canvas, fonts: &Fonts, r: RECT, text: &str, hot: bool, glyph_char: Option<char>) {
    canvas.round_rect(
        r,
        fonts.px(10),
        if hot { draw::BUTTON_HOT } else { draw::BUTTON },
        Some(if hot { draw::ACCENT } else { draw::LINE }),
    );
    match glyph_char {
        Some(g) => canvas.icon(fonts, g, r, if hot { draw::ACCENT } else { draw::TEXT }),
        None => {
            canvas.font(fonts.normal);
            canvas.text_center(text, r, if hot { draw::ACCENT } else { draw::TEXT });
        }
    }
}

fn paint(app: &mut App, hwnd: HWND) {
    let list = rows(app);
    let (status_text, status_level) = status(app);
    let fonts = &app.dash_fonts;
    let mut canvas = if app.dash_state.glass {
        // Window background mostly see-through, cards a little more solid.
        Canvas::begin_glass(hwnd, &[(draw::BG, 185), (draw::PANEL, 218), (draw::PANEL_HOT, 230)])
    } else {
        Canvas::begin(hwnd)
    };
    let full = RECT { left: 0, top: 0, right: canvas.width, bottom: canvas.height };
    canvas.fill(full, draw::BG);
    let sizes = heights(&list, fonts, canvas.width);
    let content: i32 = sizes.iter().sum::<i32>() + fonts.px(16);
    let max_scroll = (content - canvas.height).max(0);
    let config = &app.config;
    let hist = &app.hist;
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
        let mid = (top + bottom) / 2;
        match row {
            Row::Hero => {
                canvas.font(fonts.title);
                let x = canvas.text("Stats", card_l + fonts.px(4), top, bottom, draw::TEXT);
                // Status chip.
                canvas.font(fonts.small);
                let chip_w = canvas.measure(&status_text) + fonts.px(30);
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
                // Close and settings, right to left.
                let size = fonts.px(34);
                let mut bx = card_r;
                let mut offset = 0;
                for (g, control) in [(glyph::CLOSE, Control::Close), (glyph::SETTINGS, Control::Tab(Tab::Settings))] {
                    let b = RECT { left: bx - size, top: mid - size / 2, right: bx, bottom: mid + size / 2 };
                    let selected = control == Control::Tab(Tab::Settings) && state.tab == Tab::Settings;
                    if is_hot(offset) || selected {
                        canvas.round_rect(
                            b,
                            fonts.px(10),
                            if selected { draw::ACCENT_DIM } else { draw::BUTTON_HOT },
                            None,
                        );
                    }
                    let color = if selected {
                        draw::ACCENT
                    } else if is_hot(offset) {
                        draw::TEXT
                    } else {
                        draw::MUTED
                    };
                    canvas.icon(fonts, g, b, color);
                    state.hits.push((b, Hit::Control(control)));
                    bx = b.left - fonts.px(4);
                    offset += 1;
                }
                // Tabs, centred between the chip and the buttons.
                let seg_w = fonts.px(96);
                let seg_h = fonts.px(30);
                let total_w = 2 * seg_w + fonts.px(8);
                let tl = ((chip.right + bx) / 2 - total_w / 2).max(chip.right + fonts.px(12));
                let track = RECT {
                    left: tl,
                    top: mid - seg_h / 2 - fonts.px(4),
                    right: tl + total_w,
                    bottom: mid + seg_h / 2 + fonts.px(4),
                };
                canvas.round_rect(track, fonts.px(12), draw::PANEL, Some(draw::LINE));
                for (i, (label, tab)) in
                    [("Overview", Tab::Overview), ("Settings", Tab::Settings)].into_iter().enumerate()
                {
                    let l = track.left + fonts.px(4) + i as i32 * seg_w;
                    let r = RECT { left: l, top: mid - seg_h / 2, right: l + seg_w, bottom: mid + seg_h / 2 };
                    let selected = state.tab == tab;
                    if selected {
                        canvas.round_rect(r, fonts.px(10), draw::ACCENT_DIM, None);
                    } else if is_hot(offset + i) {
                        canvas.round_rect(r, fonts.px(10), draw::BUTTON_HOT, None);
                    }
                    canvas.font(if selected { fonts.bold } else { fonts.normal });
                    canvas.text_center(label, r, if selected { draw::ACCENT } else { draw::TEXT });
                    state.hits.push((r, Hit::Control(Control::Tab(tab))));
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
                    let values: Vec<f32> = spark_ring(hist, t.spark).values().collect();
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
                let values: Vec<f32> = spark_ring(hist, *which).values().collect();
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

            Row::Choice { label, detail, options } => {
                draw_label(&mut canvas, fonts, label, detail, left, top, bottom);
                let seg_w = fonts.px(104);
                let seg_h = fonts.px(30);
                let n = options.len() as i32;
                let track = RECT {
                    left: right - n * seg_w - fonts.px(8),
                    top: mid - seg_h / 2 - fonts.px(4),
                    right,
                    bottom: mid + seg_h / 2 + fonts.px(4),
                };
                canvas.round_rect(track, fonts.px(12), draw::BG, Some(draw::LINE));
                for (i, (text, control, selected)) in options.iter().enumerate() {
                    let l = track.left + fonts.px(4) + i as i32 * seg_w;
                    let r = RECT { left: l, top: mid - seg_h / 2, right: l + seg_w, bottom: mid + seg_h / 2 };
                    if *selected {
                        canvas.round_rect(r, fonts.px(10), draw::ACCENT_DIM, None);
                    } else if is_hot(i) {
                        canvas.round_rect(r, fonts.px(10), draw::BUTTON_HOT, None);
                    }
                    canvas.font(if *selected { fonts.bold } else { fonts.normal });
                    canvas.text_center(text, r, if *selected { draw::ACCENT } else { draw::TEXT });
                    state.hits.push((r, Hit::Control(*control)));
                }
            }
            Row::Toggle { label, detail, on, control } => {
                let hot = is_hot(0);
                draw_label(&mut canvas, fonts, label, detail, left, top, bottom);
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
            Row::Stepper { label, detail, value, key } => {
                draw_label(&mut canvas, fonts, label, detail, left, top, bottom);
                let hit = stepper(&mut canvas, fonts, right, mid, value, key, (is_hot(0), is_hot(1)));
                state.hits.extend(hit);
            }
            Row::Levels { label, warning, critical } => {
                canvas.font(fonts.normal);
                canvas.text(label, left, top, bottom, draw::TEXT);
                let crit_hits =
                    stepper(&mut canvas, fonts, right, mid, &critical.0, critical.1, (is_hot(0), is_hot(1)));
                let crit_left = crit_hits[0].0.left;
                canvas.font(fonts.small);
                canvas.text_right("Critical", crit_left - fonts.px(8), top, bottom, draw::CRIT);
                let warn_right = crit_left - fonts.px(70);
                let warn_hits =
                    stepper(&mut canvas, fonts, warn_right, mid, &warning.0, warning.1, (is_hot(2), is_hot(3)));
                let warn_left = warn_hits[0].0.left;
                canvas.font(fonts.small);
                canvas.text_right("Warning", warn_left - fonts.px(8), top, bottom, draw::WARN);
                state.hits.extend(crit_hits);
                state.hits.extend(warn_hits);
            }
            Row::Chips { label, items } => {
                canvas.font(fonts.normal);
                canvas.text(label, left, top, top + fonts.px(30), draw::TEXT);
                let origin = (left, top + fonts.px(30));
                for (i, (r, (text, on, control))) in
                    chip_rects(fonts, items, right - left).into_iter().zip(items).enumerate()
                {
                    let r = RECT {
                        left: r.left + origin.0,
                        top: r.top + origin.1,
                        right: r.right + origin.0,
                        bottom: r.bottom + origin.1,
                    };
                    let hot = is_hot(i);
                    let (bg, fg, edge) = if *on {
                        (draw::ACCENT_DIM, draw::ACCENT, draw::ACCENT)
                    } else {
                        (
                            if hot { draw::BUTTON_HOT } else { draw::BG },
                            if hot { draw::TEXT } else { draw::MUTED },
                            draw::LINE,
                        )
                    };
                    canvas.round_rect(r, r.bottom - r.top, bg, Some(edge));
                    let icon = RECT { left: r.left + fonts.px(8), right: r.left + fonts.px(26), ..r };
                    canvas.icon(fonts, if *on { glyph::CHECK } else { glyph::ADD }, icon, fg);
                    canvas.font(if *on { fonts.bold } else { fonts.normal });
                    canvas.text(text, r.left + fonts.px(28), r.top, r.bottom, fg);
                    state.hits.push((r, Hit::Control(*control)));
                }
            }
            Row::Folder { path, exists, index } => {
                canvas.font(fonts.normal);
                let b = RECT { left: right - fonts.px(80), top: mid - fonts.px(14), right, bottom: mid + fonts.px(14) };
                let icon = RECT { left, top, right: left + fonts.px(20), bottom };
                canvas.icon(fonts, glyph::FOLDER, icon, if *exists { draw::ACCENT } else { draw::WARN });
                canvas.font(fonts.normal);
                let text_right = b.left - fonts.px(12);
                canvas.text_clipped(path, icon.right + fonts.px(8), text_right, top, bottom, draw::TEXT);
                if !exists {
                    canvas.font(fonts.small);
                    canvas.text_right("not found", text_right, top, bottom, draw::WARN);
                }
                small_button(&mut canvas, fonts, b, "Remove", is_hot(0), None);
                state.hits.push((b, Hit::Control(Control::RemoveRoot(*index))));
            }
            Row::Buttons(buttons) => {
                let bh = fonts.px(32);
                let mut x = left;
                for (i, (g, text, control)) in buttons.iter().enumerate() {
                    canvas.font(fonts.normal);
                    let w = canvas.measure(text) + fonts.px(44);
                    let b = RECT { left: x, top: mid - bh / 2, right: x + w, bottom: mid + bh / 2 };
                    let hot = is_hot(i);
                    canvas.round_rect(
                        b,
                        fonts.px(10),
                        if hot { draw::BUTTON_HOT } else { draw::BUTTON },
                        Some(if hot { draw::ACCENT } else { draw::LINE }),
                    );
                    let danger = matches!(control, Control::Exit | Control::ResetDefaults);
                    let color = if danger && hot {
                        draw::CRIT
                    } else if hot {
                        draw::ACCENT
                    } else {
                        draw::TEXT
                    };
                    canvas.icon(
                        fonts,
                        *g,
                        RECT { left: b.left + fonts.px(8), right: b.left + fonts.px(28), ..b },
                        color,
                    );
                    canvas.font(fonts.normal);
                    canvas.text(text, b.left + fonts.px(32), b.top, b.bottom, color);
                    state.hits.push((b, Hit::Control(*control)));
                    x = b.right + fonts.px(8);
                }
            }
            Row::Shortcut { command, keys, status, recording } => {
                let detail = match status {
                    KeyStatus::InUse => "In use by another app: choose another",
                    KeyStatus::Off => "Off",
                    KeyStatus::Ready => "",
                };
                draw_label(&mut canvas, fonts, command.label(), detail, left, top, bottom);
                if *status == KeyStatus::InUse {
                    // Re-draw the detail in amber.
                    canvas.font(fonts.small);
                    canvas.text(detail, left, mid, bottom - fonts.px(4), draw::WARN);
                }
                let bh = fonts.px(30);
                let mut x = right;
                for (i, (text, control)) in [
                    ("Default", Control::DefaultHotkey(*command)),
                    ("Clear", Control::ClearHotkey(*command)),
                    ("Change", Control::Record(*command)),
                ]
                .into_iter()
                .enumerate()
                {
                    canvas.font(fonts.normal);
                    let w = canvas.measure(text) + fonts.px(24);
                    let b = RECT { left: x - w, top: mid - bh / 2, right: x, bottom: mid + bh / 2 };
                    small_button(&mut canvas, fonts, b, text, is_hot(i), None);
                    state.hits.push((b, Hit::Control(control)));
                    x = b.left - fonts.px(6);
                }
                // The key combination, as a key cap.
                canvas.font(fonts.mono_bold);
                let shown = if *recording {
                    "Press keys…".to_string()
                } else if keys.is_empty() {
                    "—".to_string()
                } else {
                    keys.clone()
                };
                let w = canvas.measure(&shown) + fonts.px(24);
                let cap =
                    RECT { left: x - fonts.px(6) - w, top: mid - bh / 2, right: x - fonts.px(6), bottom: mid + bh / 2 };
                canvas.round_rect(
                    cap,
                    fonts.px(8),
                    if *recording { draw::ACCENT_DIM } else { draw::BG },
                    Some(if *recording { draw::ACCENT } else { draw::LINE }),
                );
                canvas.font(fonts.mono_bold);
                canvas.text_center(&shown, cap, if *recording { draw::ACCENT } else { draw::TEXT });
            }
            // Drawn in the first pass.
            Row::CardEnd => {}
        }
    }
    let _ = (config, hist);
    // Keyboard focus ring.
    if let Some(i) = state.focus {
        if let Some((r, _)) = state.hits.get(i) {
            let ring = RECT { left: r.left - 3, top: r.top - 3, right: r.right + 3, bottom: r.bottom + 3 };
            let pen = unsafe { CreatePen(PS_SOLID, 2, draw::ACCENT) };
            unsafe {
                let old_pen = SelectObject(canvas.dc, pen);
                let old_brush = SelectObject(canvas.dc, GetStockObject(NULL_BRUSH));
                RoundRect(canvas.dc, ring.left, ring.top, ring.right, ring.bottom, fonts.px(12), fonts.px(12));
                SelectObject(canvas.dc, old_brush);
                SelectObject(canvas.dc, old_pen);
                DeleteObject(pen);
            }
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

/// Label on the left with a small detail line under it.
fn draw_label(canvas: &mut Canvas, fonts: &Fonts, label: &str, detail: &str, left: i32, top: i32, bottom: i32) {
    let mid = (top + bottom) / 2;
    canvas.font(fonts.normal);
    if detail.is_empty() {
        canvas.text(label, left, top, bottom, draw::TEXT);
    } else {
        canvas.text(label, left, top + fonts.px(4), mid + fonts.px(2), draw::TEXT);
        canvas.font(fonts.small);
        canvas.text(detail, left, mid, bottom - fonts.px(4), draw::MUTED);
    }
}

/// − value + ending at `right`. Returns the two button hits (minus, plus).
fn stepper(
    canvas: &mut Canvas,
    fonts: &Fonts,
    right: i32,
    mid: i32,
    value: &str,
    key: &'static str,
    hot: (bool, bool),
) -> Vec<(RECT, Hit)> {
    let (bw, bh, vw) = (fonts.px(30), fonts.px(30), fonts.px(78));
    let plus = RECT { left: right - bw, top: mid - bh / 2, right, bottom: mid + bh / 2 };
    let value_box = RECT { left: plus.left - vw, top: plus.top, right: plus.left, bottom: plus.bottom };
    let minus = RECT { left: value_box.left - bw, top: plus.top, right: value_box.left, bottom: plus.bottom };
    small_button(canvas, fonts, minus, "−", hot.0, None);
    small_button(canvas, fonts, plus, "+", hot.1, None);
    canvas.font(fonts.bold);
    canvas.text_center(value, value_box, draw::TEXT);
    vec![(minus, Hit::Control(Control::Step(key, -1))), (plus, Hit::Control(Control::Step(key, 1)))]
}

fn hit_at(app: &App, x: i32, y: i32) -> Option<usize> {
    app.dash_state.hits.iter().position(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

/// Do what a hit target does. Called with the app released.
fn activate(hwnd: HWND, hit: Hit) {
    match hit {
        Hit::Section(id) => {
            with_app(|app| {
                app.toggle_section(&id);
                unsafe { InvalidateRect(hwnd, null(), 0) };
            });
        }
        Hit::Act(action) => {
            with_app(|app| app.queue(action));
        }
        Hit::Jump(id) => {
            with_app(|app| {
                jump(app, id);
                unsafe { InvalidateRect(hwnd, null(), 0) };
            });
        }
        Hit::Control(c) => settings::run(c),
    }
}

/// While recording a shortcut: turn a key press into a new combination.
/// Returns true when the key was used.
fn record_key(vk: u32) -> bool {
    let Some(command) = with_app(|app| app.dash_state.recording).flatten() else { return false };
    let modifier = [
        VK_CONTROL,
        VK_MENU,
        VK_SHIFT,
        VK_LWIN,
        VK_RWIN,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_LMENU,
        VK_RMENU,
        VK_LSHIFT,
        VK_RSHIFT,
    ]
    .iter()
    .any(|&m| m as u32 == vk);
    if modifier {
        return true;
    }
    with_app(|app| {
        if vk == VK_ESCAPE as u32 {
            app.dash_state.recording = None;
            hotkeys::register_all(app);
        } else {
            let mods = hotkeys::mods_down();
            if let (Some(key), true) = (hotkeys::key_for(vk), mods.any()) {
                app.dash_state.recording = None;
                hotkeys::set(app, command, Some(stats_core::hotkey::Hotkey { mods, key }));
            }
        }
        unsafe { InvalidateRect(app.dash, null(), 0) };
    });
    true
}

/// Keyboard use of the dashboard. Returns true when the key was handled.
fn key(hwnd: HWND, vk: u32) -> bool {
    if record_key(vk) {
        return true;
    }
    let ctrl = unsafe { GetKeyState(VK_CONTROL as i32) } < 0;
    let shift = unsafe { GetKeyState(VK_SHIFT as i32) } < 0;
    let vk16 = vk as u16;
    let tab_to = |tab: Tab| {
        with_app(|app| {
            app.dash_state.tab = tab;
            app.dash_state.scroll = 0;
            app.dash_state.focus = None;
        });
    };
    match vk16 {
        VK_ESCAPE => {
            with_app(hide);
        }
        VK_TAB if ctrl => {
            let tab = with_app(|app| app.dash_state.tab).unwrap_or_default();
            tab_to(if tab == Tab::Overview { Tab::Settings } else { Tab::Overview });
        }
        0x31 => tab_to(Tab::Overview),
        0x32 => tab_to(Tab::Settings),
        VK_OEM_COMMA if ctrl => tab_to(Tab::Settings),
        VK_TAB => {
            with_app(|app| {
                let n = app.dash_state.hits.len();
                if n > 0 {
                    let i = match app.dash_state.focus {
                        None => {
                            if shift {
                                n - 1
                            } else {
                                0
                            }
                        }
                        Some(i) if shift => (i + n - 1) % n,
                        Some(i) => (i + 1) % n,
                    };
                    app.dash_state.focus = Some(i);
                }
            });
        }
        VK_RETURN | VK_SPACE => {
            let hit =
                with_app(|app| app.dash_state.focus.and_then(|i| app.dash_state.hits.get(i).map(|h| h.1.clone())))
                    .flatten();
            if let Some(hit) = hit {
                activate(hwnd, hit);
            }
        }
        VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END => {
            with_app(|app| {
                let page = {
                    let mut r: RECT = unsafe { zeroed() };
                    unsafe { GetClientRect(hwnd, &mut r) };
                    (r.bottom - r.top) * 9 / 10
                };
                let line = app.dash_fonts.px(48);
                let s = &mut app.dash_state;
                s.scroll = match vk16 {
                    VK_UP => s.scroll - line,
                    VK_DOWN => s.scroll + line,
                    VK_PRIOR => s.scroll - page,
                    VK_NEXT => s.scroll + page,
                    VK_HOME => 0,
                    _ => i32::MAX / 2,
                }
                .max(0);
                s.focus = None;
            });
        }
        _ => return false,
    }
    unsafe { InvalidateRect(hwnd, null(), 0) };
    true
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
            let hit = with_app(|app| {
                app.dash_state.focus = None;
                hit_at(app, x, y).map(|i| app.dash_state.hits[i].1.clone())
            })
            .flatten();
            if let Some(hit) = hit {
                activate(hwnd, hit);
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
        // Alt combinations arrive as system keys; take them while recording.
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if key(hwnd, wparam as u32) {
                Some(0)
            } else {
                None
            }
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
