//! Settings file: `%APPDATA%\Stats\config.ini`.
//!
//! Plain `key = value` lines so it can be edited in Notepad. Key names match
//! the GNOME extension's GSettings keys where the setting means the same
//! thing. Unknown keys are ignored and bad values fall back to the default,
//! with a warning written to the log, so a typo can never stop Stats starting.

use crate::numbers::TempUnit;

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
    pub bar_x: Option<i32>,
    pub bar_y: Option<i32>,
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
            bar_x: None,
            bar_y: None,
            bar_opacity: 235,
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
        }
    }
}

/// Written the first time Stats runs. Parsing this text must give exactly
/// `Config::default()` (checked by a test).
pub const DEFAULT_TEXT: &str = r#"# Stats for Windows settings.
# Edit and save: Stats reloads this file within a couple of seconds.
# Lines starting with # are comments. A bad value falls back to its default.

# How often the bar refreshes, in milliseconds (500 to 10000).
poll-interval-ms = 1000

# What the floating bar shows, left to right. Any of:
# cpu, memory, commit, storage, gpu, temperature, network, refresh
bar-metrics = cpu, memory, commit, storage, gpu, temperature, network
# Show disk and network speeds next to the percentages.
bar-show-speeds = true
bar-visible = true
# Bar position in screen pixels. Empty = just above the taskbar, on the right.
# Dragging the bar saves its position here.
bar-x =
bar-y =
# 80 (see-through) to 255 (solid).
bar-opacity = 235
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
            "bar-x" | "bar-y" => {
                let parsed = if value.is_empty() { Some(None) } else { value.parse::<i32>().ok().map(Some) };
                match parsed {
                    Some(v) if key == "bar-x" => config.bar_x = v,
                    Some(v) => config.bar_y = v,
                    None => bad = true,
                }
            }
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
        assert_eq!(parsed.config.bar_opacity, 235);
        assert_eq!(parsed.config.commit, Config::default().commit);
        assert_eq!(parsed.warnings.len(), 4);
    }

    #[test]
    fn lists_and_positions() {
        let parsed = parse("bar-metrics = commit, cpu, bogus, cpu\nbar-x = 1200\nbar-y = \nstorage-drives = c:\\, d\nscan-roots = C:\\a; D:\\b c\n");
        assert_eq!(parsed.config.bar_metrics, vec![Metric::Commit, Metric::Cpu]);
        assert_eq!(parsed.config.bar_x, Some(1200));
        assert_eq!(parsed.config.bar_y, None);
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
