//! The Settings tab of the dashboard: every option in `config.ini` as a
//! switch, stepper, chip or button, so nothing needs typing. Each change is
//! written to `config.ini` straight away (keeping the file's comments) and
//! applied at once through `App::apply_config`.

use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::actions::Action;
use super::app::{message_box, save_setting, with_app, App};
use super::dashboard::{self, Control, KeyStatus, Row, Tab};
use super::draw::glyph;
use super::{bar, hotkeys, util};
use stats_core::config::{self, find_stepper, Config, Metric};
use stats_core::hotkey::{Command, Hotkey};
use stats_core::numbers::TempUnit;
use stats_core::place::Anchor;

const DRIVE_FIXED: u32 = 3;

/// Letters of the fixed (internal) drives.
fn fixed_drives() -> Vec<char> {
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (b'A' + i) as char)
        .filter(|letter| unsafe { GetDriveTypeW(util::wide(&format!("{letter}:\\")).as_ptr()) } == DRIVE_FIXED)
        .collect()
}

fn metric_label(m: Metric) -> &'static str {
    match m {
        Metric::Cpu => "CPU",
        Metric::Memory => "RAM",
        Metric::Commit => "Commit",
        Metric::Storage => "Storage",
        Metric::Gpu => "GPU",
        Metric::Temperature => "Temperature",
        Metric::Network => "Network",
        Metric::Refresh => "Refresh rate",
    }
}

fn value_text(c: &Config, key: &'static str) -> String {
    let Some(s) = find_stepper(key) else { return String::new() };
    let v = c.number(key).unwrap_or(0);
    match (key, v) {
        ("scan-interval-hours", 0) => "manual".into(),
        ("auto-clean-temp-days", 0) => "off".into(),
        ("bar-opacity", v) => format!("{}%", v * 100 / 255),
        _ if s.unit.is_empty() => v.to_string(),
        _ => format!("{v} {}", s.unit),
    }
}

pub fn rows(app: &App) -> Vec<Row> {
    let c = &app.config;
    let mut out = Vec::new();
    let collapsed = |id: &str| c.collapsed_sections.iter().any(|x| x == id);
    let header = |out: &mut Vec<Row>, id: &'static str, title: &str, summary: String| -> bool {
        let folded = collapsed(id);
        out.push(Row::Header { id, title: title.into(), summary, collapsed: folded });
        if folded {
            out.push(Row::CardEnd);
        }
        !folded
    };
    let toggle = |label: &str, detail: &str, on: bool, control: Control| Row::Toggle {
        label: label.into(),
        detail: detail.into(),
        on,
        control,
    };
    let step = |key: &'static str, detail: &str| Row::Stepper {
        label: find_stepper(key).map(|s| s.label).unwrap_or(key).into(),
        detail: detail.into(),
        value: value_text(c, key),
        key,
    };

    if header(&mut out, "set-line", "Activity line & readings", c.bar_position.label().into()) {
        out.push(Row::Choice {
            label: "Position".into(),
            detail: "Top is centred; on the sides the line runs vertically".into(),
            options: Anchor::ALL
                .iter()
                .map(|&a| (a.label().to_string(), Control::Position(a), c.bar_position == a))
                .collect(),
        });
        out.push(toggle("Show the line", "Off hides it; the tray icon stays", c.bar_visible, Control::ShowLine));
        out.push(toggle(
            "Pin readings",
            "Keep the readings open instead of fading after 4 s",
            c.bar_pinned,
            Control::Pin,
        ));
        out.push(toggle(
            "Hide in full screen",
            "Games and videos get the whole screen",
            c.hide_when_fullscreen,
            Control::HideFullscreen,
        ));
        out.push(toggle(
            "Show speeds",
            "Disk and network speeds next to the readings",
            c.bar_show_speeds,
            Control::ShowSpeeds,
        ));
        out.push(step("bar-opacity", "How solid the frosted background of the readings is"));
        out.push(Row::Chips {
            label: "Readings shown".into(),
            items: Metric::ALL
                .iter()
                .map(|&m| (metric_label(m).to_string(), c.bar_metrics.contains(&m), Control::Metric(m)))
                .collect(),
        });
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-levels", "Warning levels", "amber · red".into()) {
        for (name, label) in [
            ("commit", "Commit"),
            ("memory", "RAM"),
            ("storage", "Storage"),
            ("cpu", "CPU"),
            ("gpu", "GPU"),
            ("temp", "Temperature"),
        ] {
            let keys: (&'static str, &'static str) = match name {
                "commit" => ("commit-warning", "commit-critical"),
                "memory" => ("memory-warning", "memory-critical"),
                "storage" => ("storage-warning", "storage-critical"),
                "cpu" => ("cpu-warning", "cpu-critical"),
                "gpu" => ("gpu-warning", "gpu-critical"),
                _ => ("temp-warning", "temp-critical"),
            };
            out.push(Row::Levels {
                label: label.into(),
                warning: (value_text(c, keys.0), keys.0),
                critical: (value_text(c, keys.1), keys.1),
            });
        }
        out.push(Row::Note {
            text: "Commit is memory promised to programs against RAM plus the page file. Apps crash at 100%, even when RAM still looks free.".into(),
            color: super::draw::MUTED,
        });
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-updates", "Updates & alerts", value_text(c, "poll-interval-ms")) {
        out.push(step("poll-interval-ms", "Lower is more current, higher uses less power"));
        out.push(step("history-seconds", "How far back the dashboard graphs go"));
        out.push(Row::Choice {
            label: "Temperature unit".into(),
            detail: String::new(),
            options: vec![
                ("°C".into(), Control::Unit(TempUnit::Celsius), c.temperature_unit == TempUnit::Celsius),
                ("°F".into(), Control::Unit(TempUnit::Fahrenheit), c.temperature_unit == TempUnit::Fahrenheit),
            ],
        });
        out.push(toggle(
            "Pressure alerts",
            "Name the program behind high memory or CPU, with a fix",
            c.pressure_notifications,
            Control::Notifications,
        ));
        out.push(step("notify-quiet-minutes", "Minimum time between two alerts about the same thing"));
        out.push(toggle(
            "Page-file check",
            "Warn when the page file is too small for this PC",
            c.pagefile_check,
            Control::PagefileCheck,
        ));
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-storage", "Storage", c.drives_text()) {
        let mut drives = fixed_drives();
        for d in &c.storage_drives {
            if let Some(letter) = d.chars().next() {
                if !drives.contains(&letter) {
                    drives.push(letter);
                }
            }
        }
        out.push(Row::Chips {
            label: "Drives to watch (the first is on the readings)".into(),
            items: drives
                .into_iter()
                .map(|l| {
                    let name = format!("{l}:");
                    let on = c.storage_drives.contains(&name);
                    (name, on, Control::Drive(l))
                })
                .collect(),
        });
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-build", "Build folders", format!("{} folders", c.scan_roots.len())) {
        for (index, root) in c.scan_roots.iter().enumerate() {
            let exists = Path::new(&util::expand(root)).is_dir();
            out.push(Row::Folder { path: root.clone(), exists, index });
        }
        if c.scan_roots.is_empty() {
            out.push(Row::Note {
                text: "No folders: add one to find build output (target, node_modules…).".into(),
                color: super::draw::MUTED,
            });
        }
        out.push(Row::Buttons(vec![
            (glyph::ADD, "Add folder…".into(), Control::AddRoot),
            (glyph::SEARCH, "Scan now".into(), Control::Scan),
        ]));
        out.push(step("scan-interval-hours", "Background rescan; manual = only when you click Scan"));
        out.push(step("junk-alert-gb", "Notify when one build folder grows past this"));
        out.push(step("auto-clean-temp-days", "Daily clean-up of old files in your Temp folder"));
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-keys", "Keyboard shortcuts", "work from any app".into()) {
        for command in Command::ALL {
            let i = command.index();
            let keys = c.hotkeys[i].map(|h| h.label()).unwrap_or_default();
            let status = match (c.hotkeys[i], app.hotkey_ok[i]) {
                (None, _) => KeyStatus::Off,
                (Some(_), Some(false)) => KeyStatus::InUse,
                _ => KeyStatus::Ready,
            };
            out.push(Row::Shortcut { command, keys, status, recording: app.dash_state.recording == Some(command) });
        }
        out.push(Row::Note {
            text: "Inside the dashboard: Esc closes, 1 and 2 switch tabs, Tab moves between controls and Space presses one.".into(),
            color: super::draw::MUTED,
        });
        out.push(Row::CardEnd);
    }

    if header(&mut out, "set-system", "System", String::new()) {
        out.push(toggle(
            "Start with Windows",
            "Run Stats when you sign in",
            app.dash_state.autostart,
            Control::Autostart,
        ));
        out.push(Row::Buttons(vec![
            (glyph::DOCUMENT, "Edit config.ini".into(), Control::EditFile),
            (glyph::DOCUMENT, "Open log".into(), Control::Log),
            (glyph::REFRESH, "Restore defaults".into(), Control::ResetDefaults),
            (glyph::POWER, "Exit Stats".into(), Control::Exit),
        ]));
        out.push(Row::CardEnd);
    }
    out
}

/// Change the settings with `f`, which returns the keys to save; then save
/// them and apply the change.
fn change(f: impl FnOnce(&mut Config) -> Vec<(&'static str, String)>) {
    with_app(|app| {
        let old = app.config.clone();
        let saves = f(&mut app.config);
        for (key, value) in saves {
            save_setting(app, key, &value);
        }
        app.apply_config(&old);
    });
}

/// The Windows folder picker. Runs with the app released (it is modal).
fn pick_folder() -> Option<String> {
    let owner = with_app(|app| {
        app.modal = true;
        app.dash
    })?;
    let title = util::wide("Choose a folder to search for build output");
    let info = BROWSEINFOW {
        hwndOwner: owner,
        pidlRoot: null_mut(),
        pszDisplayName: null_mut(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    let pidl = unsafe { SHBrowseForFolderW(&info) };
    with_app(|app| app.modal = false);
    if pidl.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let ok = unsafe { SHGetPathFromIDListW(pidl, buf.as_mut_ptr()) } != 0;
    unsafe { CoTaskMemFree(pidl as *const _) };
    ok.then(|| util::from_wide(&buf)).filter(|p| !p.is_empty())
}

/// Run a dashboard control. Called with the app released, since some of
/// these open dialogs or other programs, or close Stats.
pub fn run(control: Control) {
    match control {
        Control::Close => {
            with_app(dashboard::hide);
        }
        Control::Tab(tab) => {
            with_app(|app| {
                // The gear also toggles back to the overview.
                let tab = if tab == Tab::Settings && app.dash_state.tab == Tab::Settings { Tab::Overview } else { tab };
                app.dash_state.tab = tab;
                app.dash_state.scroll = 0;
                app.dash_state.focus = None;
            });
        }
        Control::Position(anchor) => change(|c| {
            c.bar_position = anchor;
            vec![("bar-position", anchor.key().into())]
        }),
        Control::Pin => {
            with_app(|app| {
                let pinned = !app.config.bar_pinned;
                bar::set_pinned(app, pinned);
            });
        }
        Control::Autostart => {
            let on = util::autostart_enabled();
            if !util::set_autostart(!on) {
                util::log("could not change the startup setting");
            }
            with_app(|app| app.dash_state.autostart = util::autostart_enabled());
        }
        Control::HideFullscreen => change(|c| {
            c.hide_when_fullscreen = !c.hide_when_fullscreen;
            vec![("hide-when-fullscreen", c.hide_when_fullscreen.to_string())]
        }),
        Control::ShowLine => change(|c| {
            c.bar_visible = !c.bar_visible;
            vec![("bar-visible", c.bar_visible.to_string())]
        }),
        Control::ShowSpeeds => change(|c| {
            c.bar_show_speeds = !c.bar_show_speeds;
            vec![("bar-show-speeds", c.bar_show_speeds.to_string())]
        }),
        Control::Notifications => change(|c| {
            c.pressure_notifications = !c.pressure_notifications;
            vec![("pressure-notifications", c.pressure_notifications.to_string())]
        }),
        Control::PagefileCheck => change(|c| {
            c.pagefile_check = !c.pagefile_check;
            vec![("pagefile-check", c.pagefile_check.to_string())]
        }),
        Control::Unit(unit) => change(|c| {
            c.temperature_unit = unit;
            let text = if unit == TempUnit::Celsius { "celsius" } else { "fahrenheit" };
            vec![("temperature-unit", text.into())]
        }),
        Control::Step(key, dir) => change(|c| match c.step(key, dir) {
            Some(v) => vec![(key, v.to_string())],
            None => Vec::new(),
        }),
        Control::Metric(metric) => change(|c| {
            c.toggle_metric(metric);
            vec![("bar-metrics", c.metrics_text())]
        }),
        Control::Drive(letter) => change(|c| {
            c.toggle_drive(&format!("{letter}:"));
            vec![("storage-drives", c.drives_text())]
        }),
        Control::AddRoot => {
            if let Some(path) = pick_folder() {
                change(|c| {
                    if !c.scan_roots.iter().any(|r| r.eq_ignore_ascii_case(&path)) {
                        c.scan_roots.push(path);
                    }
                    vec![("scan-roots", c.scan_roots_text())]
                });
            }
        }
        Control::RemoveRoot(index) => change(|c| {
            if index < c.scan_roots.len() {
                c.scan_roots.remove(index);
            }
            vec![("scan-roots", c.scan_roots_text())]
        }),
        Control::Record(command) => {
            with_app(|app| {
                app.dash_state.recording = Some(command);
                // Free every combination so the one pressed reaches us.
                hotkeys::unregister_all(app.main);
            });
        }
        Control::ClearHotkey(command) => {
            with_app(|app| {
                app.dash_state.recording = None;
                hotkeys::set(app, command, None);
            });
        }
        Control::DefaultHotkey(command) => {
            with_app(|app| {
                app.dash_state.recording = None;
                hotkeys::set(app, command, Hotkey::parse(command.default_text()));
            });
        }
        Control::ResetDefaults => {
            let Some(owner) = with_app(|app| {
                app.modal = true;
                app.dash
            }) else {
                return;
            };
            let answer = message_box(
                owner,
                "Restore default settings?",
                "Every setting goes back to its default. Your folded sections and the start-with-Windows choice are kept.",
                MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
            );
            with_app(|app| app.modal = false);
            if answer == IDYES {
                with_app(|app| {
                    let old = app.config.clone();
                    let mut fresh = Config { collapsed_sections: old.collapsed_sections.clone(), ..Config::default() };
                    let text = config::set_value(
                        config::DEFAULT_TEXT,
                        "collapsed-sections",
                        &fresh.collapsed_sections.join(", "),
                    );
                    if let Err(e) = std::fs::write(util::config_path(), text) {
                        util::log(&format!("could not restore defaults: {e}"));
                        return;
                    }
                    std::mem::swap(&mut app.config, &mut fresh);
                    app.note_config_written();
                    app.apply_config(&old);
                    app.status = Some(("Settings restored to their defaults.".into(), true));
                });
            }
        }
        Control::EditFile => {
            with_app(|app| app.queue(Action::OpenSettingsFile));
        }
        Control::Log => util::shell_open("notepad.exe", Some(&format!("\"{}\"", util::log_path().display()))),
        Control::Scan => {
            with_app(|app| app.queue(Action::ScanJunk));
        }
        Control::Exit => {
            if let Some(main) = with_app(|app| app.main) {
                unsafe { PostMessageW(main, WM_CLOSE, 0, 0) };
            }
        }
    }
    with_app(|app| unsafe {
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(app.dash, null(), 0);
    });
}
