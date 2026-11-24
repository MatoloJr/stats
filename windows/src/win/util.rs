//! Small Win32 helpers: wide strings, paths, logging, running commands,
//! start-with-Windows.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// NUL-terminated UTF-16 for Win32 calls.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer (possibly NUL-terminated) to String.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// Copy `text` into a fixed UTF-16 array such as NOTIFYICONDATAW::szTip,
/// truncating so the terminating NUL always fits.
pub fn fill_wide<const N: usize>(dest: &mut [u16; N], text: &str) {
    let mut i = 0;
    for unit in text.encode_utf16() {
        if i + 1 >= N {
            break;
        }
        dest[i] = unit;
        i += 1;
    }
    dest[i] = 0;
}

pub fn env(name: &str) -> Option<String> {
    std::env::var_os(name).map(|v: OsString| v.to_string_lossy().into_owned())
}

pub fn expand(text: &str) -> String {
    stats_core::config::expand_env(text, env)
}

/// `%APPDATA%\Stats`, created if missing.
pub fn data_dir() -> PathBuf {
    let base = env("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let dir = base.join("Stats");
    let _ = fs::create_dir_all(&dir);
    dir
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.ini")
}

pub fn log_path() -> PathBuf {
    data_dir().join("stats.log")
}

static LOG_LOCK: Mutex<()> = Mutex::new(());
const LOG_MAX: u64 = 512 * 1024;

/// Append a line to the log. The log is cut back to empty once it passes
/// 512 KB, so it can never fill a disk.
pub fn log(message: &str) {
    let _guard = LOG_LOCK.lock();
    let path = log_path();
    if fs::metadata(&path).map(|m| m.len() > LOG_MAX).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.old"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let _ = writeln!(file, "[{secs}] {message}");
    }
}

#[cfg(windows)]
fn hide_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_window(_command: &mut Command) {}

/// Run a console program without flashing a window. Returns success and
/// combined output (trimmed, at most a few hundred characters).
pub fn run_hidden(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    hide_window(&mut command);
    match command.output() {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            let text: String = text.trim().chars().take(400).collect();
            if output.status.success() {
                Ok(text)
            } else {
                Err(if text.is_empty() { format!("{program} exited with {}", output.status) } else { text })
            }
        }
        Err(e) => Err(format!("could not start {program}: {e}")),
    }
}

/// Is `program` on PATH? (Checked without starting it.)
pub fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else { return false };
    let exts = ["", ".exe", ".cmd", ".bat"];
    std::env::split_paths(&path).any(|dir| exts.iter().any(|ext| dir.join(format!("{program}{ext}")).is_file()))
}

/// Open a file, folder, URL or program with its default handler.
pub fn shell_open(target: &str, params: Option<&str>) {
    let file = wide(target);
    let params_w = params.map(wide);
    let verb = wide("open");
    unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            params_w.as_ref().map_or(null(), |p| p.as_ptr()),
            null(),
            SW_SHOWNORMAL,
        );
    }
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "Stats";

fn open_run_key(access: REG_SAM_FLAGS) -> Option<HKEY> {
    let key = wide(RUN_KEY);
    let mut handle: HKEY = null_mut();
    let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, access, &mut handle) };
    (status == ERROR_SUCCESS).then_some(handle)
}

pub fn autostart_enabled() -> bool {
    let Some(key) = open_run_key(KEY_QUERY_VALUE) else { return false };
    let name = wide(RUN_VALUE);
    let status = unsafe { RegQueryValueExW(key, name.as_ptr(), null(), null_mut(), null_mut(), null_mut()) };
    unsafe { RegCloseKey(key) };
    status == ERROR_SUCCESS
}

pub fn set_autostart(enable: bool) -> bool {
    let Some(key) = open_run_key(KEY_SET_VALUE) else { return false };
    let name = wide(RUN_VALUE);
    let status = if enable {
        let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let value = wide(&format!("\"{exe}\""));
        unsafe { RegSetValueExW(key, name.as_ptr(), 0, REG_SZ, value.as_ptr() as *const u8, (value.len() * 2) as u32) }
    } else {
        unsafe { RegDeleteValueW(key, name.as_ptr()) }
    };
    unsafe { RegCloseKey(key) };
    status == ERROR_SUCCESS
}

/// Free bytes on the volume holding `path`.
pub fn free_space(path: &str) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root = wide(path);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok = unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut avail, &mut total, &mut free) };
    (ok != 0 && total > 0).then_some((free, total))
}

/// The page file setting from the registry (readable without admin).
pub fn pagefile_setting() -> Option<stats_core::pagefile::PageFile> {
    let key = wide(r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management");
    let value = wide("PagingFiles");
    let mut bytes = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_MULTI_SZ,
            null_mut(),
            null_mut(),
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS || bytes == 0 || bytes > 64 * 1024 {
        return None;
    }
    let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_MULTI_SZ,
            null_mut(),
            buffer.as_mut_ptr() as *mut _,
            &mut bytes,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    // REG_MULTI_SZ: strings separated by NUL, ended by an empty string.
    let entries: Vec<String> =
        buffer.split(|&c| c == 0).map(String::from_utf16_lossy).take_while(|s| !s.is_empty()).collect();
    Some(stats_core::pagefile::parse(&entries))
}
