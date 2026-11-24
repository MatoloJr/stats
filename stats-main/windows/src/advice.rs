//! Friendly names and one concrete next step for the programs that usually
//! eat memory on a developer laptop.

use crate::config::Level;
use crate::numbers::fmt_bytes;
use crate::pressure::{Group, Kind, Trigger};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Close the program's windows (it can save first), force-end on request.
    CloseApp(String),
    /// `wsl --shutdown`: stops the WSL/Docker virtual machine (vmmem).
    ShutdownWsl,
    /// Nothing safe to do from Stats; the text explains what to do instead.
    None,
}

/// Programs Stats must never offer to close.
const PROTECTED: &[&str] = &[
    "system",
    "registry",
    "idle",
    "smss",
    "csrss",
    "wininit",
    "winlogon",
    "services",
    "lsass",
    "lsaiso",
    "svchost",
    "dwm",
    "explorer",
    "fontdrvhost",
    "sihost",
    "ctfmon",
    "memory compression",
    "msmpeng",
    "nissrv",
    "securityhealthservice",
    "audiodg",
    "spoolsv",
    "searchindexer",
    "runtimebroker",
    "taskhostw",
    "stats",
    "conhost",
    "textinputhost",
    "startmenuexperiencehost",
    "shellexperiencehost",
    "searchhost",
    "lockapp",
    "wudfhost",
    "dashost",
    "secure system",
];

pub fn is_protected(key: &str) -> bool {
    PROTECTED.contains(&key)
}

pub fn display_name(key: &str) -> String {
    let known = match key {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "code" => "VS Code",
        "cursor" => "Cursor",
        "vmmem" | "vmmemwsl" => "WSL / Docker VM",
        "com.docker.backend" | "docker desktop" => "Docker Desktop",
        "java" | "javaw" => "Java (Gradle / Android Studio)",
        "studio64" => "Android Studio",
        "qemu-system-x86_64" | "emulator" => "Android emulator",
        "rustc" => "Rust compiler",
        "cargo" => "Cargo",
        "rust-analyzer" => "rust-analyzer",
        "link" | "rust-lld" | "lld-link" => "Linker",
        "node" => "Node.js",
        "dart" | "dartaotruntime" => "Dart / Flutter",
        "msmpeng" => "Microsoft Defender",
        "memory compression" => "Memory Compression",
        "teams" | "ms-teams" => "Teams",
        "slack" => "Slack",
        "discord" => "Discord",
        "postgres" => "PostgreSQL",
        "python" | "pythonw" => "Python",
        "go" => "Go",
        "gopls" => "gopls",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// What to suggest for this program.
pub fn advise(key: &str, kind: Kind) -> (String, Fix) {
    let close = || Fix::CloseApp(key.to_string());
    match key {
        "vmmem" | "vmmemwsl" => {
            ("Quit Docker Desktop if you're not using it, or shut down WSL.".into(), Fix::ShutdownWsl)
        }
        "chrome" | "msedge" | "firefox" | "brave" => ("Close tabs you don't need.".into(), close()),
        "rustc" | "cargo" | "link" | "rust-lld" | "lld-link" => (
            "A Rust build is running. Set fewer parallel jobs in .cargo\\config.toml ([build] jobs = 4).".into(),
            Fix::None,
        ),
        "java" | "javaw" => {
            ("Gradle daemons stay running after builds. Run 'gradlew --stop' in the project.".into(), close())
        }
        "qemu-system-x86_64" | "emulator" => {
            ("Close the Android emulator when you're not testing on it.".into(), close())
        }
        "node" => ("A Node.js process (dev server or watcher) may be left running.".into(), close()),
        "msmpeng" => ("Defender is scanning. It eases off when the scan finishes.".into(), Fix::None),
        "memory compression" => {
            ("Windows is compressing memory because RAM is full. Close a large app to relieve it.".into(), Fix::None)
        }
        _ if is_protected(key) => ("This is part of Windows, so Stats won't close it.".into(), Fix::None),
        _ => match kind {
            Kind::Cpu => (format!("Close {} if you don't need it right now.", display_name(key)), close()),
            _ => (format!("Close {} to free memory.", display_name(key)), close()),
        },
    }
}

/// Notification title and body for a trigger and the program behind it.
pub fn message(trigger: &Trigger, culprit: Option<&Group>) -> (String, String) {
    let pct = format!("{:.0}%", trigger.value);
    let title = match (trigger.kind, trigger.level) {
        (Kind::Commit, Level::Critical) => format!("Memory almost exhausted ({pct} committed)"),
        (Kind::Commit, _) => format!("Memory is running low ({pct} committed)"),
        (Kind::Memory, Level::Critical) => format!("RAM almost full ({pct})"),
        (Kind::Memory, _) => format!("RAM is high ({pct})"),
        (Kind::Cpu, _) => format!("CPU busy ({pct})"),
    };
    let body = match culprit {
        Some(group) => {
            let name = display_name(&group.key);
            let what = match trigger.kind {
                Kind::Cpu => format!("{name} is using {:.0}% of the CPU", group.cpu),
                _ => format!("{name} is using {}", fmt_bytes(group.private_bytes)),
            };
            let across = if group.count > 1 { format!(" across {} processes", group.count) } else { String::new() };
            let (hint, _) = advise(&group.key, trigger.kind);
            format!("{what}{across}. {hint} Click to open Stats.")
        }
        None => "Click to open Stats and see what is using it.".to_string(),
    };
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(display_name("vmmemwsl"), "WSL / Docker VM");
        assert_eq!(display_name("figma"), "Figma");
    }

    #[test]
    fn never_offers_to_close_windows_parts() {
        assert_eq!(advise("svchost", Kind::Commit).1, Fix::None);
        assert_eq!(advise("explorer", Kind::Commit).1, Fix::None);
        assert_eq!(advise("vmmem", Kind::Commit).1, Fix::ShutdownWsl);
        assert_eq!(advise("figma", Kind::Commit).1, Fix::CloseApp("figma".into()));
    }

    #[test]
    fn message_names_program() {
        let trigger = Trigger { kind: Kind::Commit, level: Level::Critical, value: 93.4 };
        let group =
            Group { key: "chrome".into(), count: 23, private_bytes: 4 * 1024 * 1024 * 1024, cpu: 3.0, pids: vec![] };
        let (title, body) = message(&trigger, Some(&group));
        assert_eq!(title, "Memory almost exhausted (93% committed)");
        assert!(body.starts_with("Chrome is using 4.0 GB across 23 processes. Close tabs"));
    }
}
