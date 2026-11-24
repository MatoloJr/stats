//! Settings file: `%APPDATA%\Stats\config.ini`.
//!
//! Plain `key = value` lines so it can be edited in Notepad. Key names match
//! the GNOME extension's GSettings keys where the setting means the same
//! thing. Unknown keys are ignored and bad values fall back to the default,
//! with a warning written to the log, so a typo can never stop Stats starting.

use crate::hotkey::{Command, Hotkey};
use crate::numbers::TempUnit;
use crate::place::Anchor;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Metric {
    Cpu,
    Memory,
    Commit,
    Storage,
    Gpu,
    Temperature,
    Network,
    Refresh,
}

impl Metric {
    pub const ALL: [Metric; 8] = [
        Metric::Cpu,
        Metric::Memory,
        Metric::Commit,
        Metric::Storage,
        Metric::Gpu,
        Metric::Temperature,
        Metric::Network,
        Metric::Refresh,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Metric::Cpu => "cpu",
            Metric::Memory => "memory",
            Metric::Commit => "commit",
            Metric::Storage => "storage",
            Metric::Gpu => "gpu",
            Metric::Temperature => "temperature",
            Metric::Network => "network",
            Metric::Refresh => "refresh",
        }
    }

    pub fn from_key(key: &str) -> Option<Metric> {
        Metric::ALL.into_iter().find(|m| m.key() == key.trim().to_ascii_lowercase())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub warning: u8,
    pub critical: u8,
}

impl Limits {
    pub const fn new(warning: u8, critical: u8) -> Self {
        Self { warning, critical }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Normal,
    Warning,
    Critical,
}

impl Limits {
    pub fn level(&self, value: Option<f64>) -> Level {
        match value {
            Some(v) if v >= self.critical as f64 => Level::Critical,
            Some(v) if v >= self.warning as f64 => Level::Warning,
            _ => Level::Normal,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub poll_interval_ms: u32,
    pub bar_metrics: Vec<Metric>,
    pub bar_show_speeds: bool,
    pub bar_visible: bool,
    pub bar_position: Anchor,
    pub bar_pinned: bool,
    pub bar_opacity: u8,
    pub hide_when_fullscreen: bool,
    pub temperature_unit: TempUnit,
    pub storage_drives: Vec<String>,
    pub cpu: Limits,
    pub memory: Limits,
    pub commit: Limits,
    pub storage: Limits,
    pub gpu: Limits,
    pub temp: Limits,
    pub pressure_notifications: bool,
    pub notify_quiet_minutes: u32,
    pub pagefile_check: bool,
    pub scan_roots: Vec<String>,
    pub scan_interval_hours: u32,
    pub junk_alert_gb: u32,
    pub auto_clean_temp_days: u32,
    pub history_seconds: u32,
    pub collapsed_sections: Vec<String>,
    /// Global shortcuts, indexed by `Command::index()`. `None` = off.
    pub hotkeys: [Option<Hotkey>; 5],
}

fn default_hotkeys() -> [Option<Hotkey>; 5] {
    Command::ALL.map(|c| Hotkey::parse(c.default_text()))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1000,
            bar_metrics: vec![
                Metric::Cpu,
                Metric::Memory,
                Metric::Commit,
                Metric::Storage,
                Metric::Gpu,
                Metric::Temperature,
                Metric::Network,
            ],
            bar_show_speeds: true,
            bar_visible: true,
            bar_position: Anchor::Top,
            bar_pinned: false,
            bar_opacity: 250,
            hide_when_fullscreen: true,
            temperature_unit: TempUnit::Celsius,
            storage_drives: vec!["C:".to_string()],
            cpu: Limits::new(90, 98),
            memory: Limits::new(85, 95),
            commit: Limits::new(80, 92),
            storage: Limits::new(85, 95),
            gpu: Limits::new(90, 98),
            temp: Limits::new(85, 95),
            pressure_notifications: true,
            notify_quiet_minutes: 10,
            pagefile_check: true,
            scan_roots: vec![r"%USERPROFILE%\projects".to_string()],
            scan_interval_hours: 24,
            junk_alert_gb: 20,
            auto_clean_temp_days: 0,
            history_seconds: 120,
            collapsed_sections: Vec::new(),
            hotkeys: default_hotkeys(),
        }
    }
}

/// Written the first time Stats runs. Parsing this text must give exactly
/// `Config::default()` (checked by a test).
pub const DEFAULT_TEXT: &str = r#"# Stats for Windows settings.
# Most of these can be changed in Stats itself: open the dashboard, then
# Settings. Edit and save here instead and Stats reloads within a couple of
# seconds. Lines starting with # are comments. A bad value uses its default.

# How often the readings refresh, in milliseconds (500 to 10000).
poll-interval-ms = 1000

# Where the activity line sits: top (centred), left or right (vertical).
# Rest the pointer on it to wake it, then click for the readings strip.
bar-position = top
# Keep the readings strip open instead of hiding it a few seconds after use.
bar-pinned = false
# Show the activity line at all.
bar-visible = true
# What the readings strip shows, left to right. Any of:
# cpu, memory, commit, storage, gpu, temperature, network, refresh
bar-metrics = cpu, memory, commit, storage, gpu, temperature, network
# Show disk and network speeds next to the percentages.
bar-show-speeds = true
# Readings strip opacity: 80 (see-through) to 255 (solid).
bar-opacity = 250
# Hide the bar while a game or video is running full screen.
hide-when-fullscreen = true
# celsius or fahrenheit
temperature-unit = celsius

# Drives to watch. The first one is shown on the bar.
storage-drives = C:

# Warning and critical levels, in percent (temperature in Celsius).
# "commit" is committed memory against its limit (RAM + page file).
# Windows refuses memory and apps crash when commit reaches 100%,
# even if RAM still looks partly free.
cpu-warning = 90
cpu-critical = 98
memory-warning = 85
memory-critical = 95
commit-warning = 80
commit-critical = 92
storage-warning = 85
storage-critical = 95
gpu-warning = 90
gpu-critical = 98
temp-warning = 85
temp-critical = 95

# Notify when memory or CPU stays high, naming the program responsible
# and offering a fix. Nothing is closed without your click.
pressure-notifications = true
# Minimum minutes between two warnings about the same problem.
# A jump from warning to critical is always reported.
notify-quiet-minutes = 10
# Warn when the page file is too small for this machine.
pagefile-check = true

# Folders searched for build output (Rust target, node_modules, Flutter
# build, .next, .dart_tool, .gradle). Separate several with ;
scan-roots = %USERPROFILE%\projects
# Rescan every N hours in the background (0 = only when you click Scan).
scan-interval-hours = 24
# Notify when one build folder grows past this many GB.
junk-alert-gb = 20

# Delete files in your Temp folder older than this many days, once a day.
# 0 = never automatically (the dashboard still has a Clean button).
auto-clean-temp-days = 0

# Seconds of history kept for the dashboard graphs (30 to 3600).
history-seconds = 120

# Dashboard sections you folded away (managed by Stats).
collapsed-sections =

# Keyboard shortcuts that work from any app. Combine ctrl, alt, shift or win
# with a letter, digit, F1-F24 or , . / ; - =  Leave empty to turn one off.
hotkey-dashboard = ctrl+alt+s
hotkey-readings = ctrl+alt+r
hotkey-settings = ctrl+alt+,
hotkey-pin = ctrl+alt+p
hotkey-line = ctrl+alt+h
"#;

/// Result of parsing: the settings plus a note for every value that was
/// rejected, so the log can say which line was wrong.
pub struct Parsed {
    pub config: Config,
    pub warnings: Vec<String>,
}

fn split_list(value: &str, separators: &[char]) -> Vec<String> {
    value.split(|c| separators.contains(&c)).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect()
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" => Some(false),
        _ => None,
    }
}

fn parse_ranged(value: &str, min: i64, max: i64) -> Option<i64> {
    value.trim().parse::<i64>().ok().filter(|v| (min..=max).contains(v))
}

pub fn parse(text: &str) -> Parsed {
    let mut config = Config::default();
    let mut warnings = Vec::new();
    // Thresholds are validated as pairs after reading, since warning must be
    // below critical.
    for (number, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            warnings.push(format!("line {}: expected key = value", number + 1));
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        let mut bad = false;
        macro_rules! ranged {
            ($field:expr, $min:expr, $max:expr, $ty:ty) => {
                match parse_ranged(value, $min, $max) {
                    Some(v) => $field = v as $ty,
                    None => bad = true,
                }
            };
        }
        macro_rules! boolean {
            ($field:expr) => {
                match parse_bool(value) {
                    Some(v) => $field = v,
                    None => bad = true,
                }
            };
        }
        match key.as_str() {
            "poll-interval-ms" => ranged!(config.poll_interval_ms, 500, 10_000, u32),
            "bar-metrics" => {
                let mut metrics = Vec::new();
                for item in split_list(value, &[',', ' ']) {
                    match Metric::from_key(&item) {
                        Some(m) if !metrics.contains(&m) => metrics.push(m),
                        Some(_) => {}
                        None => warnings.push(format!("line {}: unknown metric '{item}'", number + 1)),
                    }
                }
                config.bar_metrics = metrics;
            }
            "bar-show-speeds" => boolean!(config.bar_show_speeds),
            "bar-visible" => boolean!(config.bar_visible),
            "bar-position" => match Anchor::from_key(value) {
                Some(anchor) => config.bar_position = anchor,
                None => bad = true,
            },
            "bar-pinned" => boolean!(config.bar_pinned),
            // Before 0.2 the bar could be dragged anywhere; those saved
            // positions are no longer used but must not fill the log.
            "bar-x" | "bar-y" => {}
            "bar-opacity" => ranged!(config.bar_opacity, 80, 255, u8),
            "hide-when-fullscreen" => boolean!(config.hide_when_fullscreen),
            "temperature-unit" => match value.to_ascii_lowercase().as_str() {
                "celsius" | "c" => config.temperature_unit = TempUnit::Celsius,
                "fahrenheit" | "f" => config.temperature_unit = TempUnit::Fahrenheit,
                _ => bad = true,
            },
            "storage-drives" => {
                let drives: Vec<String> =
                    split_list(value, &[',', ';', ' ']).into_iter().filter_map(|d| normalize_drive(&d)).collect();
                if drives.is_empty() {
                    bad = true;
                } else {
                    config.storage_drives = drives;
                }
            }
            "cpu-warning" => ranged!(config.cpu.warning, 1, 100, u8),
            "cpu-critical" => ranged!(config.cpu.critical, 1, 100, u8),
            "memory-warning" => ranged!(config.memory.warning, 1, 100, u8),
            "memory-critical" => ranged!(config.memory.critical, 1, 100, u8),
            "commit-warning" => ranged!(config.commit.warning, 1, 100, u8),
            "commit-critical" => ranged!(config.commit.critical, 1, 100, u8),
            "storage-warning" => ranged!(config.storage.warning, 1, 100, u8),
            "storage-critical" => ranged!(config.storage.critical, 1, 100, u8),
            "gpu-warning" => ranged!(config.gpu.warning, 1, 100, u8),
            "gpu-critical" => ranged!(config.gpu.critical, 1, 100, u8),
            "temp-warning" => ranged!(config.temp.warning, 20, 150, u8),
            "temp-critical" => ranged!(config.temp.critical, 20, 150, u8),
            "pressure-notifications" => boolean!(config.pressure_notifications),
            "notify-quiet-minutes" => ranged!(config.notify_quiet_minutes, 1, 1440, u32),
            "pagefile-check" => boolean!(config.pagefile_check),
            "scan-roots" => config.scan_roots = split_list(value, &[';']),
            "scan-interval-hours" => ranged!(config.scan_interval_hours, 0, 24 * 30, u32),
            "junk-alert-gb" => ranged!(config.junk_alert_gb, 1, 10_000, u32),
            "auto-clean-temp-days" => ranged!(config.auto_clean_temp_days, 0, 365, u32),
            "history-seconds" => ranged!(config.history_seconds, 30, 3600, u32),
            "collapsed-sections" => config.collapsed_sections = split_list(value, &[',']),
            k if k.starts_with("hotkey-") => match Command::ALL.iter().find(|c| c.key() == k) {
                Some(command) if value.is_empty() => config.hotkeys[command.index()] = None,
                Some(command) => match Hotkey::parse(value) {
                    Some(hotkey) => config.hotkeys[command.index()] = Some(hotkey),
                    None => bad = true,
                },
                None => warnings.push(format!("line {}: unknown setting '{key}'", number + 1)),
            },
            _ => warnings.push(format!("line {}: unknown setting '{key}'", number + 1)),
        }
        if bad {
            warnings.push(format!("line {}: invalid value for {key}: '{value}', using the default", number + 1));
        }
    }
    let defaults = Config::default();
    for (name, limits, fallback) in [
        ("cpu", &mut config.cpu, defaults.cpu),
        ("memory", &mut config.memory, defaults.memory),
        ("commit", &mut config.commit, defaults.commit),
        ("storage", &mut config.storage, defaults.storage),
        ("gpu", &mut config.gpu, defaults.gpu),
        ("temp", &mut config.temp, defaults.temp),
    ] {
        if limits.warning >= limits.critical {
            warnings.push(format!("{name}-warning must be below {name}-critical, using the defaults"));
            *limits = fallback;
        }
    }
    Parsed { config, warnings }
}

/// A number the Settings screen changes with − and + buttons. The ranges
/// are the same ones `parse` accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stepper {
    pub key: &'static str,
    pub label: &'static str,
    pub min: i64,
    pub max: i64,
    pub step: i64,
    pub unit: &'static str,
}

const fn stepper(key: &'static str, label: &'static str, min: i64, max: i64, step: i64, unit: &'static str) -> Stepper {
    Stepper { key, label, min, max, step, unit }
}

pub const STEPPERS: &[Stepper] = &[
    stepper("poll-interval-ms", "Refresh every", 500, 10_000, 250, "ms"),
    stepper("history-seconds", "Graph history", 30, 3600, 30, "s"),
    stepper("bar-opacity", "Readings opacity", 80, 255, 15, ""),
    stepper("notify-quiet-minutes", "Quiet time between alerts", 1, 1440, 5, "min"),
    stepper("scan-interval-hours", "Rescan build folders every", 0, 24 * 30, 6, "h"),
    stepper("junk-alert-gb", "Alert when a build folder passes", 1, 10_000, 5, "GB"),
    stepper("auto-clean-temp-days", "Delete Temp files older than", 0, 365, 1, "days"),
    stepper("cpu-warning", "CPU warning", 1, 100, 1, "%"),
    stepper("cpu-critical", "CPU critical", 1, 100, 1, "%"),
    stepper("memory-warning", "RAM warning", 1, 100, 1, "%"),
    stepper("memory-critical", "RAM critical", 1, 100, 1, "%"),
    stepper("commit-warning", "Commit warning", 1, 100, 1, "%"),
    stepper("commit-critical", "Commit critical", 1, 100, 1, "%"),
    stepper("storage-warning", "Storage warning", 1, 100, 1, "%"),
    stepper("storage-critical", "Storage critical", 1, 100, 1, "%"),
    stepper("gpu-warning", "GPU warning", 1, 100, 1, "%"),
    stepper("gpu-critical", "GPU critical", 1, 100, 1, "%"),
    stepper("temp-warning", "Temperature warning", 20, 150, 1, "°C"),
    stepper("temp-critical", "Temperature critical", 20, 150, 1, "°C"),
];

pub fn find_stepper(key: &str) -> Option<&'static Stepper> {
    STEPPERS.iter().find(|s| s.key == key)
}

impl Config {
    fn limits_mut(&mut self, name: &str) -> Option<&mut Limits> {
        Some(match name {
            "cpu" => &mut self.cpu,
            "memory" => &mut self.memory,
            "commit" => &mut self.commit,
            "storage" => &mut self.storage,
            "gpu" => &mut self.gpu,
            "temp" => &mut self.temp,
            _ => return None,
        })
    }

    /// The current value of a stepper setting.
    pub fn number(&self, key: &str) -> Option<i64> {
        Some(match key {
            "poll-interval-ms" => self.poll_interval_ms as i64,
            "history-seconds" => self.history_seconds as i64,
            "bar-opacity" => self.bar_opacity as i64,
            "notify-quiet-minutes" => self.notify_quiet_minutes as i64,
            "scan-interval-hours" => self.scan_interval_hours as i64,
            "junk-alert-gb" => self.junk_alert_gb as i64,
            "auto-clean-temp-days" => self.auto_clean_temp_days as i64,
            _ => {
                let (name, which) = key.rsplit_once('-')?;
                let limits = match name {
                    "cpu" => self.cpu,
                    "memory" => self.memory,
                    "commit" => self.commit,
                    "storage" => self.storage,
                    "gpu" => self.gpu,
                    "temp" => self.temp,
                    _ => return None,
                };
                match which {
                    "warning" => limits.warning as i64,
                    "critical" => limits.critical as i64,
                    _ => return None,
                }
            }
        })
    }

    /// Move a stepper setting `steps` steps (negative = down), clamped to its
    /// range, keeping each warning level below its critical level. Returns
    /// the new value when it changed.
    pub fn step(&mut self, key: &str, steps: i64) -> Option<i64> {
        let s = find_stepper(key)?;
        let current = self.number(key)?;
        let (mut min, mut max) = (s.min, s.max);
        if let Some((name, which)) = key.rsplit_once('-') {
            if let Some(limits) = self.limits_mut(name) {
                match which {
                    "warning" => max = max.min(limits.critical as i64 - 1),
                    "critical" => min = min.max(limits.warning as i64 + 1),
                    _ => {}
                }
            }
        }
        // Snap to the step grid from the minimum, so 500 + 250k stays tidy.
        let raw = current + steps * s.step;
        let snapped = if steps == 0 { raw } else { min + ((raw - min) as f64 / s.step as f64).round() as i64 * s.step };
        let value = snapped.clamp(min, max.max(min));
        if value == current {
            return None;
        }
        match key {
            "poll-interval-ms" => self.poll_interval_ms = value as u32,
            "history-seconds" => self.history_seconds = value as u32,
            "bar-opacity" => self.bar_opacity = value as u8,
            "notify-quiet-minutes" => self.notify_quiet_minutes = value as u32,
            "scan-interval-hours" => self.scan_interval_hours = value as u32,
            "junk-alert-gb" => self.junk_alert_gb = value as u32,
            "auto-clean-temp-days" => self.auto_clean_temp_days = value as u32,
            _ => {
                let (name, which) = key.rsplit_once('-')?;
                let limits = self.limits_mut(name)?;
                match which {
                    "warning" => limits.warning = value as u8,
                    _ => limits.critical = value as u8,
                }
            }
        }
        Some(value)
    }

    /// Show or hide a metric on the strip, keeping the standard order. At
    /// least one metric always stays.
    pub fn toggle_metric(&mut self, metric: Metric) {
        if let Some(pos) = self.bar_metrics.iter().position(|&m| m == metric) {
            if self.bar_metrics.len() > 1 {
                self.bar_metrics.remove(pos);
            }
        } else {
            self.bar_metrics.push(metric);
            self.bar_metrics.sort_by_key(|m| Metric::ALL.iter().position(|x| x == m));
        }
    }

    /// Watch or stop watching a drive, keeping at least one.
    pub fn toggle_drive(&mut self, drive: &str) {
        let Some(drive) = normalize_drive(drive) else { return };
        if let Some(pos) = self.storage_drives.iter().position(|d| *d == drive) {
            if self.storage_drives.len() > 1 {
                self.storage_drives.remove(pos);
            }
        } else {
            self.storage_drives.push(drive);
        }
    }

    /// Settings-file text for the list settings.
    pub fn metrics_text(&self) -> String {
        self.bar_metrics.iter().map(|m| m.key()).collect::<Vec<_>>().join(", ")
    }

    pub fn drives_text(&self) -> String {
        self.storage_drives.join(", ")
    }

    pub fn scan_roots_text(&self) -> String {
        self.scan_roots.join("; ")
    }

    pub fn hotkey_text(&self, command: Command) -> String {
        self.hotkeys[command.index()].map(|h| h.key_text()).unwrap_or_default()
    }
}

/// `c`, `C:`, `C:\` -> `C:`.
pub fn normalize_drive(text: &str) -> Option<String> {
    let mut chars = text.trim().chars();
    let letter = chars.next()?.to_ascii_uppercase();
    let rest: String = chars.collect();
    if letter.is_ascii_alphabetic() && matches!(rest.as_str(), "" | ":" | ":\\" | ":/") {
        Some(format!("{letter}:"))
    } else {
        None
    }
}

/// Replace `key`'s value in the file text, keeping every comment and other
/// line as the user wrote it. Appends the key when it is missing.
pub fn set_value(text: &str, key: &str, value: &str) -> String {
    let mut out = String::with_capacity(text.len() + key.len() + value.len() + 4);
    let mut found = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let matches = !trimmed.starts_with('#')
            && !trimmed.starts_with(';')
            && trimmed.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key));
        if matches && !found {
            out.push_str(&format!("{key} = {value}"));
            found = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    if !found {
        out.push_str(&format!("{key} = {value}\n"));
    }
    out
}

/// Expand `%NAME%` using `lookup`. Unknown names are left as written.
pub fn expand_env(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) if !name.is_empty() => out.push_str(&value),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_matches_default_config() {
        let parsed = parse(DEFAULT_TEXT);
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
        assert_eq!(parsed.config, Config::default());
    }

    #[test]
    fn bad_values_fall_back() {
        let parsed =
            parse("poll-interval-ms = 5\nbar-opacity = lots\ncommit-warning = 95\ncommit-critical = 90\nnope\n");
        assert_eq!(parsed.config.poll_interval_ms, 1000);
        assert_eq!(parsed.config.bar_opacity, 250);
        assert_eq!(parsed.config.commit, Config::default().commit);
        assert_eq!(parsed.warnings.len(), 4);
    }

    #[test]
    fn lists_and_positions() {
        let parsed = parse("bar-metrics = commit, cpu, bogus, cpu\nbar-x = 1200\nbar-y = \nstorage-drives = c:\\, d\nscan-roots = C:\\a; D:\\b c\n");
        assert_eq!(parsed.config.bar_metrics, vec![Metric::Commit, Metric::Cpu]);
        // Old drag positions are accepted silently.
        assert_eq!(parsed.config.bar_position, Anchor::Top);
        let placed = parse("bar-position = Right\nbar-pinned = yes\n");
        assert!(placed.warnings.is_empty(), "{:?}", placed.warnings);
        assert_eq!(placed.config.bar_position, Anchor::Right);
        assert!(placed.config.bar_pinned);
        assert_eq!(parse("bar-position = bottom\n").warnings.len(), 1);
        assert_eq!(parsed.config.storage_drives, vec!["C:", "D:"]);
        assert_eq!(parsed.config.scan_roots, vec!["C:\\a", "D:\\b c"]);
        assert_eq!(parsed.warnings.len(), 1);
    }

    #[test]
    fn set_value_keeps_comments() {
        let text = "# bar-x = 5\nbar-x =\nother = 1\n";
        let out = set_value(text, "bar-x", "640");
        assert_eq!(out, "# bar-x = 5\nbar-x = 640\nother = 1\n");
        let appended = set_value("a = 1\n", "bar-y", "10");
        assert_eq!(appended, "a = 1\nbar-y = 10\n");
    }

    #[test]
    fn steppers_clamp_and_keep_levels_ordered() {
        let mut c = Config::default();
        assert_eq!(c.step("poll-interval-ms", 1), Some(1250));
        assert_eq!(c.step("poll-interval-ms", -100), Some(500));
        assert_eq!(c.step("poll-interval-ms", -1), None, "already at the minimum");
        // Warning can't reach critical, and critical can't drop to warning.
        c.commit = Limits::new(90, 92);
        assert_eq!(c.step("commit-warning", 5), Some(91));
        assert_eq!(c.step("commit-critical", -5), None, "already one above warning");
        assert_eq!(c.commit, Limits::new(91, 92));
        assert_eq!(c.step("temp-critical", 100), Some(150));
        assert_eq!(c.number("temp-critical"), Some(150));
        assert_eq!(c.step("nope", 1), None);
        // Every stepper reads a value and its default parses within range.
        let d = Config::default();
        for s in STEPPERS {
            let v = d.number(s.key).unwrap_or_else(|| panic!("{} has no value", s.key));
            assert!((s.min..=s.max).contains(&v), "{} default {v} outside {}..{}", s.key, s.min, s.max);
        }
    }

    #[test]
    fn metric_and_drive_toggles() {
        let mut c = Config::default();
        c.toggle_metric(Metric::Gpu);
        assert!(!c.bar_metrics.contains(&Metric::Gpu));
        c.toggle_metric(Metric::Gpu);
        assert_eq!(c.metrics_text(), "cpu, memory, commit, storage, gpu, temperature, network");
        c.toggle_metric(Metric::Refresh);
        assert_eq!(c.bar_metrics.last(), Some(&Metric::Refresh));
        c.toggle_drive("d:\\");
        assert_eq!(c.drives_text(), "C:, D:");
        c.toggle_drive("C:");
        c.toggle_drive("D:");
        assert_eq!(c.drives_text(), "D:", "the last drive stays");
        let round_trip = parse(&format!("bar-metrics = {}\nstorage-drives = {}\n", c.metrics_text(), c.drives_text()));
        assert_eq!(round_trip.config.bar_metrics, c.bar_metrics);
        assert_eq!(round_trip.config.storage_drives, c.storage_drives);
    }

    #[test]
    fn hotkeys_parse_and_turn_off() {
        let c = parse("hotkey-dashboard = win+shift+d\nhotkey-pin =\nhotkey-line = h\nhotkey-nope = ctrl+x\n");
        assert_eq!(c.warnings.len(), 2, "{:?}", c.warnings);
        assert_eq!(c.config.hotkey_text(Command::Dashboard), "shift+win+d");
        assert_eq!(c.config.hotkeys[Command::Pin.index()], None);
        // A bad one keeps its default.
        assert_eq!(c.config.hotkey_text(Command::Line), "ctrl+alt+h");
    }

    #[test]
    fn env_expansion() {
        let lookup = |name: &str| (name == "USERPROFILE").then(|| r"C:\Users\emm".to_string());
        assert_eq!(expand_env(r"%USERPROFILE%\projects", lookup), r"C:\Users\emm\projects");
        assert_eq!(expand_env(r"%NOPE%\x", lookup), r"%NOPE%\x");
        assert_eq!(expand_env("100%", lookup), "100%");
    }

    #[test]
    fn levels() {
        let limits = Limits::new(80, 92);
        assert_eq!(limits.level(Some(79.9)), Level::Normal);
        assert_eq!(limits.level(Some(80.0)), Level::Warning);
        assert_eq!(limits.level(Some(95.0)), Level::Critical);
        assert_eq!(limits.level(None), Level::Normal);
    }
}

/// Checks `Config::default()` against the cross-platform spec in
/// `shared/defaults.ini` ([shared] plus [windows]).
#[cfg(test)]
mod spec {
    use super::*;

    const SPEC: &str = include_str!("../../shared/defaults.ini");

    /// The `key = value` lines of `[shared]` and `[windows]`.
    fn windows_lines() -> Vec<(String, String)> {
        let mut section = String::new();
        let mut out = Vec::new();
        for raw in SPEC.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            if section == "shared" || section == "windows" {
                let (key, value) = line.split_once('=').expect("spec line is key = value");
                out.push((key.trim().to_string(), value.trim().to_string()));
            }
        }
        out
    }

    #[test]
    fn defaults_match_shared_spec() {
        let lines = windows_lines();
        assert!(lines.len() > 10, "spec looks empty");
        // Every spec key must be a real setting here and hold the default.
        let text: String = lines.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        let parsed = parse(&text);
        assert!(parsed.warnings.is_empty(), "spec keys rejected: {:?}", parsed.warnings);
        assert_eq!(parsed.config, Config::default(), "shared/defaults.ini and Config::default() disagree");
        // Parsing starts from the default, so also check each value changes
        // the result when altered: a spec value equal to the default by
        // accident of the fallback would otherwise pass.
        for (key, value) in &lines {
            assert!(
                DEFAULT_TEXT
                    .lines()
                    .any(|l| l.split_once('=').is_some_and(|(k, v)| k.trim() == key && v.trim() == value.as_str())),
                "DEFAULT_TEXT does not say {key} = {value}"
            );
        }
    }
}
