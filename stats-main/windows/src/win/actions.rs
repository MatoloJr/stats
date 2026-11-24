//! The one-click fixes. Each runs on its own thread so the bar keeps
//! updating, and reports back with a short sentence. Anything that closes a
//! program or deletes files asks first (see `confirm_text`).

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime};

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::PostMessageW;

use super::procs;
use super::util::{self, free_space, run_hidden};
use stats_core::advice::display_name;
use stats_core::cleanup;
use stats_core::junk::{self, JunkDir};
use stats_core::numbers::fmt_bytes;

pub const WM_APP_WAKE: u32 = windows_sys::Win32::UI::WindowsAndMessaging::WM_APP + 2;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    CloseApp { key: String, pids: Vec<u32> },
    ForceEnd { key: String, pids: Vec<u32> },
    ShutdownWsl,
    DockerPrune,
    CompactDocker,
    OpenPagefileSettings,
    ScanJunk,
    CleanJunk(JunkDir),
    CleanAllJunk(Vec<JunkDir>),
    CleanTemp,
    CleanCaches,
    OpenStorageSettings,
    OpenSettingsFile,
}

/// What comes back from worker threads.
pub enum Done {
    Message {
        text: String,
        ok: bool,
        rescan: bool,
    },
    /// Graceful close finished; these processes did not exit.
    Leftover {
        key: String,
        pids: Vec<u32>,
    },
    Scanned {
        dirs: Vec<JunkDir>,
        temp_bytes: u64,
        at: SystemTime,
    },
    AutoTemp {
        freed: u64,
    },
    Procs {
        groups: Vec<stats_core::pressure::Group>,
        alert: Option<stats_core::pressure::Trigger>,
    },
}

/// Sends results to the main thread and wakes it.
#[derive(Clone)]
pub struct Reply {
    pub tx: Sender<Done>,
    /// HWND as an integer: window handles may be used from any thread for
    /// PostMessage, but the raw pointer type is not Send.
    pub hwnd: isize,
}

impl Reply {
    pub fn send(&self, done: Done) {
        if self.tx.send(done).is_ok() {
            unsafe { PostMessageW(self.hwnd as HWND, WM_APP_WAKE, 0, 0) };
        }
    }

    fn say(&self, ok: bool, rescan: bool, text: impl Into<String>) {
        self.send(Done::Message { text: text.into(), ok, rescan });
    }
}

/// Actions that need a Yes before running, with the question to ask.
pub fn confirm_text(action: &Action) -> Option<(String, String)> {
    match action {
        Action::CloseApp { key, pids } => Some((
            format!("Close {}?", display_name(key)),
            format!(
                "Stats will ask {} ({} process{}) to close, as if you clicked its X. It can still ask you to save.",
                display_name(key),
                pids.len(),
                if pids.len() == 1 { "" } else { "es" }
            ),
        )),
        Action::ForceEnd { key, .. } => Some((
            format!("Force-end {}?", display_name(key)),
            "It did not close on request. Ending it now loses anything unsaved in it.".to_string(),
        )),
        Action::ShutdownWsl => Some((
            "Shut down WSL and Docker?".into(),
            "Runs 'wsl --shutdown'. Every WSL distro and all Docker containers stop. Quit Docker Desktop first if you don't want it to start the VM again."
                .into(),
        )),
        Action::DockerPrune => Some((
            "Clean up Docker?".into(),
            "Runs 'docker system prune -f': removes stopped containers, unused networks, dangling images and build cache. Volumes (your databases) and tagged images are kept."
                .into(),
        )),
        Action::CompactDocker => Some((
            "Compact the Docker disk?".into(),
            "Shuts down WSL and shrinks Docker's virtual disk file so freed space goes back to Windows. Windows will ask for administrator permission. Docker Desktop must be closed."
                .into(),
        )),
        Action::CleanJunk(dir) => Some((
            format!("Delete {}?", dir.kind.label()),
            format!(
                "{}\n\nThis frees {} and is {}.",
                dir.path.display(),
                fmt_bytes(dir.bytes),
                dir.kind.rebuild_hint()
            ),
        )),
        Action::CleanAllJunk(dirs) => {
            let total: u64 = dirs.iter().map(|d| d.bytes).sum();
            Some((
                format!("Delete {} build folders?", dirs.len()),
                format!(
                    "Frees {}. Each is rebuilt by its build tool the next time you build that project. Source code is not touched.",
                    fmt_bytes(total)
                ),
            ))
        }
        Action::CleanTemp => Some((
            "Empty the Temp folder?".into(),
            "Deletes everything in your Temp folder that is not in use right now.".into(),
        )),
        Action::CleanCaches => Some((
            "Clear package caches?".into(),
            "Runs the cache clean commands for the tools you have installed: npm cache clean --force, go clean -cache, pip cache purge, pnpm store prune. Packages download again when next needed."
                .into(),
        )),
        _ => None,
    }
}

/// Where Docker Desktop keeps its virtual disk, newest layout first.
pub fn docker_disk() -> Option<PathBuf> {
    let local = PathBuf::from(util::env("LOCALAPPDATA")?);
    [r"Docker\wsl\disk\docker_data.vhdx", r"Docker\wsl\data\ext4.vhdx", r"Docker\wsl\main\ext4.vhdx"]
        .iter()
        .map(|p| local.join(p))
        .find(|p| p.is_file())
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

fn system_free() -> u64 {
    let drive = util::env("SystemDrive").unwrap_or_else(|| "C:".into());
    free_space(&format!("{drive}\\")).map(|(free, _)| free).unwrap_or(0)
}

/// Run an action that does not need the UI thread. `roots` are the scan
/// roots (already expanded) for build-folder safety checks.
pub fn spawn(action: Action, roots: Vec<PathBuf>, reply: Reply) {
    let name = format!("{action:?}").chars().take(40).collect::<String>();
    let result = std::thread::Builder::new().name("stats-action".into()).spawn(move || {
        // A failure in one fix must not take Stats down with it.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(action, &roots, &reply)));
        if outcome.is_err() {
            util::log("an action thread panicked");
            reply.say(false, false, "That action failed unexpectedly. Details are in the log.");
        }
    });
    if let Err(e) = result {
        util::log(&format!("could not start action {name}: {e}"));
    }
}

fn run(action: Action, roots: &[PathBuf], reply: &Reply) {
    match action {
        Action::CloseApp { key, pids } => {
            let asked = procs::close_gracefully(&pids);
            if asked == 0 {
                // Background process with no window: offer force-end.
                reply.send(Done::Leftover { key, pids });
                return;
            }
            // Give it time to save and exit.
            for _ in 0..16 {
                std::thread::sleep(Duration::from_millis(500));
                if procs::still_running(&pids).is_empty() {
                    break;
                }
            }
            let left = procs::still_running(&pids);
            if left.is_empty() {
                reply.say(true, false, format!("{} closed.", display_name(&key)));
            } else {
                reply.send(Done::Leftover { key, pids: left });
            }
        }
        Action::ForceEnd { key, pids } => {
            let (ended, failed) = procs::terminate(&pids);
            if failed == 0 {
                reply.say(
                    true,
                    false,
                    format!("Ended {} ({ended} process{}).", display_name(&key), if ended == 1 { "" } else { "es" }),
                );
            } else {
                reply.say(
                    false,
                    false,
                    format!("Ended {ended}, but {failed} could not be ended (it may need administrator rights)."),
                );
            }
        }
        Action::ShutdownWsl => match run_hidden("wsl", &["--shutdown"]) {
            Ok(_) => reply.say(true, false, "WSL and Docker's VM are shut down."),
            Err(e) => reply.say(false, false, format!("wsl --shutdown failed: {e}")),
        },
        Action::DockerPrune => {
            let before = system_free();
            match run_hidden("docker", &["system", "prune", "-f"]) {
                Ok(_) => {
                    let freed = system_free().saturating_sub(before);
                    reply.say(
                        true,
                        true,
                        format!(
                            "Docker cleaned. Freed inside Docker's disk; Windows gets it back after Compact ({} freed on disk now).",
                            fmt_bytes(freed)
                        ),
                    );
                }
                Err(e) if e.contains("daemon") || e.contains("pipe") => {
                    reply.say(false, false, "Docker isn't running. Start Docker Desktop, then try again.")
                }
                Err(e) => reply.say(false, false, format!("docker system prune failed: {e}")),
            }
        }
        Action::CompactDocker => compact_docker(reply),
        Action::ScanJunk => scan(roots, reply),
        Action::CleanJunk(dir) => match junk::clean(&dir.path, roots) {
            Ok(freed) => {
                reply.say(true, true, format!("Deleted {} ({} freed).", short_path(&dir.path), fmt_bytes(freed)))
            }
            Err(e) => reply.say(false, true, format!("Kept {}: {e}.", short_path(&dir.path))),
        },
        Action::CleanAllJunk(dirs) => {
            let (mut freed, mut kept) = (0u64, 0u32);
            for dir in &dirs {
                match junk::clean(&dir.path, roots) {
                    Ok(bytes) => freed += bytes,
                    Err(e) => {
                        kept += 1;
                        util::log(&format!("kept {}: {e}", dir.path.display()));
                    }
                }
            }
            let tail =
                if kept > 0 { format!(" {kept} could not be deleted (in use?), see the log.") } else { String::new() };
            reply.say(kept == 0, true, format!("Freed {}.{tail}", fmt_bytes(freed)));
        }
        Action::CleanTemp => {
            let report = cleanup::clean_older_than(&temp_dir(), Duration::ZERO, SystemTime::now());
            reply.say(
                true,
                true,
                format!("Temp: freed {}. {} item(s) in use were left alone.", fmt_bytes(report.freed), report.skipped),
            );
        }
        Action::CleanCaches => {
            let before = system_free();
            let mut ran = Vec::new();
            let mut failed = Vec::new();
            let commands: [(&str, &[&str]); 4] = [
                ("npm", &["cache", "clean", "--force"]),
                ("go", &["clean", "-cache"]),
                ("pip", &["cache", "purge"]),
                ("pnpm", &["store", "prune"]),
            ];
            for (program, args) in commands {
                if !util::on_path(program) {
                    continue;
                }
                // npm and pnpm are .cmd shims, which need cmd.exe to start.
                let result = if matches!(program, "npm" | "pnpm") {
                    let mut full = vec!["/c", program];
                    full.extend_from_slice(args);
                    run_hidden("cmd", &full)
                } else {
                    run_hidden(program, args)
                };
                match result {
                    Ok(_) => ran.push(program),
                    Err(e) => {
                        util::log(&format!("{program} cache clean: {e}"));
                        failed.push(program);
                    }
                }
            }
            let freed = system_free().saturating_sub(before);
            let text = if ran.is_empty() && failed.is_empty() {
                "No npm, go, pip or pnpm found on PATH.".to_string()
            } else if failed.is_empty() {
                format!("Cleared {} caches: {} freed.", ran.join(", "), fmt_bytes(freed))
            } else {
                format!(
                    "Cleared {}; {} failed (see log). {} freed.",
                    ran.join(", "),
                    failed.join(", "),
                    fmt_bytes(freed)
                )
            };
            reply.say(failed.is_empty(), true, text);
        }
        // UI-only actions are handled on the main thread.
        Action::OpenPagefileSettings | Action::OpenStorageSettings | Action::OpenSettingsFile => {}
    }
}

pub fn short_path(path: &Path) -> String {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    if parts.len() <= 3 {
        path.display().to_string()
    } else {
        format!("…\\{}", parts[parts.len() - 3..].join("\\"))
    }
}

/// Background scan of build folders plus the Temp folder size.
pub fn scan(roots: &[PathBuf], reply: &Reply) {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let dirs = junk::scan(roots, &cancel);
    let temp_bytes = cleanup::folder_size(&temp_dir());
    reply.send(Done::Scanned { dirs, temp_bytes, at: SystemTime::now() });
}

/// Daily automatic Temp clean, only files older than `days`.
pub fn auto_temp(days: u32, reply: &Reply) {
    let report = cleanup::clean_older_than(&temp_dir(), Duration::from_secs(days as u64 * 86_400), SystemTime::now());
    util::log(&format!("auto temp clean: freed {} ({} skipped)", report.freed, report.skipped));
    reply.send(Done::AutoTemp { freed: report.freed });
}

fn compact_docker(reply: &Reply) {
    use std::mem::{size_of, zeroed};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    let Some(disk) = docker_disk() else {
        reply.say(false, false, "Docker's virtual disk was not found.");
        return;
    };
    if procs::is_running("docker desktop.exe") || procs::is_running("com.docker.backend.exe") {
        reply.say(
            false,
            false,
            "Quit Docker Desktop first (tray whale, Quit Docker Desktop), then click Compact again.",
        );
        return;
    }
    let before = std::fs::metadata(&disk).map(|m| m.len()).unwrap_or(0);
    let script = std::env::temp_dir().join("stats-compact-docker.txt");
    let body = format!(
        "select vdisk file=\"{}\"\r\nattach vdisk readonly\r\ncompact vdisk\r\ndetach vdisk\r\nexit\r\n",
        disk.display()
    );
    if let Err(e) = std::fs::write(&script, body) {
        reply.say(false, false, format!("Could not write the diskpart script: {e}"));
        return;
    }
    let params = util::wide(&format!("/c wsl --shutdown & diskpart /s \"{}\"", script.display()));
    let file = util::wide("cmd.exe");
    let verb = util::wide("runas");
    let mut info: SHELLEXECUTEINFOW = unsafe { zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 || info.hProcess.is_null() {
        let _ = std::fs::remove_file(&script);
        reply.say(false, false, "Compact was cancelled (administrator permission is needed).");
        return;
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        windows_sys::Win32::Foundation::CloseHandle(info.hProcess);
    }
    let _ = std::fs::remove_file(&script);
    let after = std::fs::metadata(&disk).map(|m| m.len()).unwrap_or(before);
    if code == 0 {
        reply.say(
            true,
            true,
            format!(
                "Docker disk compacted: {} → {} ({} returned to Windows).",
                fmt_bytes(before),
                fmt_bytes(after),
                fmt_bytes(before.saturating_sub(after))
            ),
        );
    } else {
        reply.say(
            false,
            true,
            format!("diskpart reported an error (code {code}). The disk may still be in use; try after a restart."),
        );
    }
}
