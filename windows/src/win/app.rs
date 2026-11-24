//! Application state, the poll loop, and the hidden main window that owns
//! the timer, the tray icon and the messages from worker threads.
//!
//! Everything UI-related lives on the main thread in one `App`, reached
//! through `with_app`. Calls that run a nested message loop (message boxes,
//! menus, window dragging) are always made with the `App` released, so a
//! message arriving meanwhile can never hit a double borrow.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::SystemTime;

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::InvalidateRect;
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::{SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::actions::{self, Action, Done, Reply, WM_APP_WAKE};
use super::draw::Fonts;
use super::sensors::{CpuMeter, Sensors, Snapshot};
use super::util::{self, wide};
use super::{bar, dashboard, icon, line, procs, tray};
use stats_core::advice;
use stats_core::config::{self, Config, Level};
use stats_core::history::Ring;
use stats_core::junk::JunkDir;
use stats_core::numbers::fmt_bytes;
use stats_core::pagefile::{self, PageFile};
use stats_core::place::Anchor;
use stats_core::pressure::{self, Group, Guard, Kind, Trigger};
use stats_core::wave::Wave;

pub const WM_APP_ACTION: u32 = WM_APP + 3;
pub const WM_APP_SHOW_DASH: u32 = WM_APP + 4;
pub const MAIN_CLASS: &str = "StatsForWindowsMain";
const TIMER_ID: usize = 1;
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 7;
const WTS_SESSION_UNLOCK: usize = 8;

const MINUTE: u64 = 60_000;
const HOUR: u64 = 60 * MINUTE;
const DAY: u64 = 24 * HOUR;

pub struct History {
    pub cpu: Ring,
    pub memory: Ring,
    pub commit: Ring,
    pub gpu: Ring,
    pub net_down: Ring,
    pub net_up: Ring,
    pub disk: Ring,
}

impl History {
    fn new(capacity: usize) -> Self {
        History {
            cpu: Ring::new(capacity),
            memory: Ring::new(capacity),
            commit: Ring::new(capacity),
            gpu: Ring::new(capacity),
            net_down: Ring::new(capacity),
            net_up: Ring::new(capacity),
            disk: Ring::new(capacity),
        }
    }

    fn resize(&mut self, capacity: usize) {
        for ring in [
            &mut self.cpu,
            &mut self.memory,
            &mut self.commit,
            &mut self.gpu,
            &mut self.net_down,
            &mut self.net_up,
            &mut self.disk,
        ] {
            ring.resize(capacity);
        }
    }
}

pub struct App {
    pub main: HWND,
    /// The readings strip.
    pub bar: HWND,
    pub dash: HWND,
    /// Dimmed backdrop behind the dashboard.
    pub scrim: HWND,
    pub line: line::State,
    pub strip: bar::Strip,
    pub wave: Wave,
    pub cpu_meter: CpuMeter,
    /// The session is locked: nothing is drawn.
    locked: bool,
    pub config: Config,
    config_mtime: Option<SystemTime>,
    pub sensors: Sensors,
    pub snap: Snapshot,
    pub hist: History,
    guard: Guard,
    proc_tx: Sender<Option<Trigger>>,
    pub groups: Vec<Group>,
    groups_at: u64,
    pub reply: Reply,
    rx: Receiver<Done>,
    pub busy: Option<String>,
    pub junk: Vec<JunkDir>,
    pub junk_at: Option<SystemTime>,
    pub temp_bytes: Option<u64>,
    pub docker_disk: Option<u64>,
    pub status: Option<(String, bool)>,
    pub leftover: Option<(String, Vec<u32>)>,
    pub last_alert: Option<(Trigger, Option<String>)>,
    pub pagefile: Option<PageFile>,
    icon: HICON,
    icon_state: (Level, u8),
    tray_ok: bool,
    taskbar_created: u32,
    notified: HashMap<String, u64>,
    last_slow: u64,
    last_config_check: u64,
    started: u64,
    pub bar_fonts: Fonts,
    pub dash_fonts: Fonts,
    pub bar_layout: bar::Layout,
    pub dash_state: dashboard::State,
    pub modal: bool,
    pending: Option<Action>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Run `f` with the app, or return None when it is already in use further up
/// the stack (a re-entrant message) or not created yet.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(f),
        Err(_) => None,
    })
}

pub fn now_ms() -> u64 {
    unsafe { GetTickCount64() }
}

/// Wrap a window procedure body so a bug is logged instead of aborting the
/// process (a panic cannot unwind across the Win32 boundary).
pub fn guarded(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    body: impl FnOnce() -> Option<LRESULT>,
) -> LRESULT {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(Some(result)) => result,
        Ok(None) => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        Err(_) => {
            util::log(&format!("recovered from a panic while handling message {msg:#x}"));
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

pub fn scan_roots(config: &Config) -> Vec<PathBuf> {
    config.scan_roots.iter().map(|r| PathBuf::from(util::expand(r))).filter(|p| p.is_dir()).collect()
}

fn load_config() -> (Config, Option<SystemTime>) {
    let path = util::config_path();
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, config::DEFAULT_TEXT) {
            util::log(&format!("could not write {}: {e}", path.display()));
        }
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let parsed = config::parse(&text);
    for warning in &parsed.warnings {
        util::log(&format!("config: {warning}"));
    }
    let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    (parsed.config, mtime)
}

/// Rewrite one setting in config.ini, keeping the rest of the file as is.
pub fn save_setting(app: &mut App, key: &str, value: &str) {
    let path = util::config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| config::DEFAULT_TEXT.to_string());
    let updated = config::set_value(&text, key, value);
    if let Err(e) = std::fs::write(&path, updated) {
        util::log(&format!("could not save {key}: {e}"));
    }
    app.config_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
}

fn start_process_thread(reply: Reply) -> Sender<Option<Trigger>> {
    let (tx, rx) = channel::<Option<Trigger>>();
    let spawned = std::thread::Builder::new().name("stats-procs".into()).spawn(move || {
        let mut scanner = procs::ProcessScanner::new();
        while let Ok(mut alert) = rx.recv() {
            // Collapse a backlog into one scan, keeping any alert.
            while let Ok(next) = rx.try_recv() {
                alert = alert.or(next);
            }
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let first = scanner.scan();
                if alert.is_some_and(|t| t.kind == Kind::Cpu) {
                    // CPU shares need two samples a moment apart.
                    std::thread::sleep(std::time::Duration::from_millis(800));
                    scanner.scan()
                } else {
                    first
                }
            }));
            match result {
                Ok(groups) => reply.send(Done::Procs { groups, alert }),
                Err(_) => util::log("process scan panicked; skipped"),
            }
        }
    });
    if let Err(e) = spawned {
        util::log(&format!("could not start the process thread: {e}"));
    }
    tx
}

pub fn create(instance: HINSTANCE) -> Result<(), String> {
    let (config, config_mtime) = load_config();
    let main = create_main_window(instance)?;
    let bar = bar::create(instance)?;
    let (dash, scrim) = dashboard::create(instance)?;
    let line = line::create(instance)?;
    // Pause the animation while the screen is locked.
    unsafe {
        windows_sys::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(
            main,
            windows_sys::Win32::System::RemoteDesktop::NOTIFY_FOR_THIS_SESSION,
        )
    };

    let (tx, rx) = channel();
    let reply = Reply { tx, hwnd: main as isize };
    let proc_tx = start_process_thread(reply.clone());
    let dpi = unsafe { GetDpiForWindow(line.hwnd) };
    let capacity = (config.history_seconds * 1000 / config.poll_interval_ms).max(2) as usize;
    let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
    let icon = icon::make(Level::Normal, 0.3);
    let tray_ok = tray::add(main, icon, "Stats");

    let app = App {
        main,
        bar,
        dash,
        scrim,
        line,
        strip: bar::Strip::default(),
        wave: Wave::new(line::COLUMNS),
        cpu_meter: CpuMeter::default(),
        locked: false,
        sensors: Sensors::new(),
        snap: Snapshot::default(),
        hist: History::new(capacity),
        guard: Guard::default(),
        proc_tx,
        groups: Vec::new(),
        groups_at: 0,
        reply,
        rx,
        busy: None,
        junk: Vec::new(),
        junk_at: None,
        temp_bytes: None,
        docker_disk: None,
        status: None,
        leftover: None,
        last_alert: None,
        pagefile: util::pagefile_setting(),
        icon,
        icon_state: (Level::Normal, 3),
        tray_ok,
        taskbar_created,
        notified: HashMap::new(),
        last_slow: 0,
        last_config_check: 0,
        started: now_ms(),
        bar_fonts: Fonts::new(dpi),
        dash_fonts: Fonts::new(dpi),
        bar_layout: bar::Layout::default(),
        dash_state: dashboard::State::default(),
        modal: false,
        pending: None,
        config,
        config_mtime,
    };
    let interval = app.config.poll_interval_ms;
    APP.with(|cell| *cell.borrow_mut() = Some(app));
    unsafe { SetTimer(main, TIMER_ID, interval, None) };
    with_app(|app| {
        app.tick();
        if app.config.bar_pinned {
            bar::open(app);
        }
    });
    Ok(())
}

fn create_main_window(instance: HINSTANCE) -> Result<HWND, String> {
    let class = wide(MAIN_CLASS);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(main_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..unsafe { std::mem::zeroed() }
    };
    unsafe { RegisterClassW(&wc) };
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            class.as_ptr(),
            wide("Stats").as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };
    if hwnd.is_null() {
        Err("could not create the main window".into())
    } else {
        Ok(hwnd)
    }
}

impl App {
    fn notify(&mut self, key: &str, quiet_ms: u64, title: &str, body: &str, critical: bool) {
        let now = now_ms();
        if let Some(&at) = self.notified.get(key) {
            if now.saturating_sub(at) < quiet_ms {
                return;
            }
        }
        self.notified.insert(key.to_string(), now);
        util::log(&format!("notify: {title} - {body}"));
        if self.tray_ok {
            tray::balloon(self.main, title, body, critical);
        }
    }

    pub fn dash_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.dash) != 0 }
    }

    pub fn tick(&mut self) {
        let now = now_ms();
        self.snap = self.sensors.sample(&self.config.storage_drives);
        let s = &self.snap;
        self.hist.cpu.push(s.cpu);
        self.hist.memory.push(s.memory);
        self.hist.commit.push(s.commit);
        self.hist.gpu.push(s.gpu);
        self.hist.net_down.push(s.net_down);
        self.hist.net_up.push(s.net_up);
        self.hist.disk.push(s.disk_io());

        if self.config.pressure_notifications {
            let reading = pressure::Reading { commit: s.commit, memory: s.memory, cpu: s.cpu };
            let limits =
                [(Kind::Commit, self.config.commit), (Kind::Memory, self.config.memory), (Kind::Cpu, self.config.cpu)];
            let quiet = self.config.notify_quiet_minutes as u64 * MINUTE;
            if let Some(trigger) = self.guard.observe(reading, limits, quiet, now) {
                let _ = self.proc_tx.send(Some(trigger));
            }
        }
        if self.dash_visible() && now.saturating_sub(self.groups_at) >= 3000 {
            self.groups_at = now;
            let _ = self.proc_tx.send(None);
        }
        if now.saturating_sub(self.last_config_check) >= 2000 {
            self.last_config_check = now;
            self.reload_config_if_changed();
        }
        if now.saturating_sub(self.last_slow) >= MINUTE || self.last_slow == 0 {
            self.last_slow = now;
            self.slow_checks(now);
        }
        self.update_overlay();
        self.update_tray();
        if self.dash_visible() {
            unsafe { InvalidateRect(self.dash, null(), 0) };
        }
    }

    /// Once a minute: disk space, page file, Docker disk, scheduled scans.
    fn slow_checks(&mut self, now: u64) {
        self.docker_disk = actions::docker_disk().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len());

        // Wait a minute after start so login noise settles.
        let settled = now.saturating_sub(self.started) >= MINUTE;

        if let Some(drive) = self.snap.drives.first().cloned() {
            let level = self.config.storage.level(drive.percent);
            if settled && level != Level::Normal {
                let reclaim: u64 = self.junk.iter().map(|d| d.bytes).sum::<u64>() + self.temp_bytes.unwrap_or(0);
                let tail = if reclaim > 1 << 30 {
                    format!(" About {} of build folders and Temp files can be freed.", fmt_bytes(reclaim))
                } else {
                    " Open Stats to see what can be cleaned.".to_string()
                };
                let quiet = if level == Level::Critical { HOUR } else { 6 * HOUR };
                self.notify(
                    &format!("storage-{}-{level:?}", drive.name),
                    quiet,
                    &format!("{} is {:.0}% full", drive.name, drive.percent.unwrap_or(0.0)),
                    &format!("{} free.{tail}", fmt_bytes(drive.total.saturating_sub(drive.used))),
                    level == Level::Critical,
                );
            }
        }

        self.pagefile = util::pagefile_setting();
        if settled && self.config.pagefile_check && self.pagefile_small() {
            self.notify(
                "pagefile",
                DAY,
                "Page file is too small",
                &format!(
                    "Memory can only grow to {} before Windows starts crashing apps. Raise the page file to 16 GB or more. Click for the fix.",
                    fmt_bytes(self.snap.commit_limit)
                ),
                false,
            );
        }

        let interval = self.config.scan_interval_hours as u64 * HOUR;
        let due = |key: &str, every: u64, notified: &HashMap<String, u64>| {
            notified.get(key).is_none_or(|&at| now.saturating_sub(at) >= every)
        };
        if interval > 0 && self.busy.is_none() && settled && due("scan-run", interval, &self.notified) {
            self.notified.insert("scan-run".into(), now);
            self.start_background_scan();
        }
        let days = self.config.auto_clean_temp_days;
        if days > 0 && self.busy.is_none() && settled && due("auto-temp", DAY, &self.notified) {
            self.notified.insert("auto-temp".into(), now);
            self.busy = Some("Cleaning old Temp files".into());
            let reply = self.reply.clone();
            let _ =
                std::thread::Builder::new().name("stats-temp".into()).spawn(move || actions::auto_temp(days, &reply));
        }
    }

    pub fn start_background_scan(&mut self) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some("Scanning build folders".into());
        let roots = scan_roots(&self.config);
        let reply = self.reply.clone();
        let spawned = std::thread::Builder::new().name("stats-scan".into()).spawn(move || {
            lower_thread_priority();
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| actions::scan(&roots, &reply))).is_err() {
                util::log("build-folder scan panicked");
                reply.send(Done::Message { text: "The scan failed; see the log.".into(), ok: false, rescan: false });
            }
        });
        if spawned.is_err() {
            self.busy = None;
        }
    }

    fn reload_config_if_changed(&mut self) {
        let mtime = std::fs::metadata(util::config_path()).and_then(|m| m.modified()).ok();
        if mtime == self.config_mtime {
            return;
        }
        let (config, mtime) = load_config();
        self.config_mtime = mtime;
        if config == self.config {
            return;
        }
        let interval_changed = config.poll_interval_ms != self.config.poll_interval_ms;
        let moved = config.bar_position != self.config.bar_position;
        let pinned = config.bar_pinned && !self.config.bar_pinned;
        let capacity = (config.history_seconds * 1000 / config.poll_interval_ms).max(2) as usize;
        self.config = config;
        self.hist.resize(capacity);
        if interval_changed {
            unsafe { SetTimer(self.main, TIMER_ID, self.config.poll_interval_ms, None) };
        }
        bar::apply_opacity(self);
        if moved {
            line::place(self);
        }
        if pinned {
            bar::open(self);
        }
        self.status = Some(("Settings reloaded.".into(), true));
        util::log("settings reloaded");
    }

    fn fullscreen_app_running(&self) -> bool {
        let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
        if unsafe { SHQueryUserNotificationState(&mut state) } != 0 {
            return false;
        }
        // 2 = busy (full-screen app), 3 = Direct3D full screen, 4 = presentation.
        matches!(state, 2..=4)
    }

    /// Show or hide the activity line and the strip, once a poll.
    pub fn update_overlay(&mut self) {
        // While our own dashboard is open its full-screen backdrop would
        // read as "a full-screen app is running", so skip that check then.
        let hide = self.locked
            || !self.config.bar_visible
            || (self.config.hide_when_fullscreen && !self.dash_visible() && self.fullscreen_app_running());
        if hide {
            line::hide(self);
            bar::hide_now(self);
            return;
        }
        line::show(self);
        line::update_tooltip(self);
        if self.strip.open {
            bar::relayout(self);
        }
        // Stay above the taskbar and other topmost windows.
        unsafe {
            for hwnd in [self.line.hwnd, self.bar] {
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
                );
            }
        }
    }

    pub fn worst_level(&self) -> Level {
        let c = &self.config;
        let s = &self.snap;
        [c.commit.level(s.commit), c.memory.level(s.memory), c.storage.level(s.drives.first().and_then(|d| d.percent))]
            .into_iter()
            .max()
            .unwrap_or(Level::Normal)
    }

    fn update_tray(&mut self) {
        if !self.tray_ok {
            return;
        }
        let level = self.worst_level();
        let fill_bucket = (self.snap.commit.unwrap_or(30.0) / 100.0 * 6.0).round() as u8;
        let s = &self.snap;
        let tip = format!(
            "Stats\nCPU {} · RAM {} · Commit {}\n{} {}",
            stats_core::numbers::fmt_percent(s.cpu),
            stats_core::numbers::fmt_percent(s.memory),
            stats_core::numbers::fmt_percent(s.commit),
            s.drives.first().map(|d| d.name.as_str()).unwrap_or(""),
            stats_core::numbers::fmt_percent(s.drives.first().and_then(|d| d.percent)),
        );
        let new_icon = if (level, fill_bucket) != self.icon_state {
            self.icon_state = (level, fill_bucket);
            let old = self.icon;
            self.icon = icon::make(level, fill_bucket as f64 / 6.0);
            icon::destroy(old);
            Some(self.icon)
        } else {
            None
        };
        tray::update(self.main, new_icon, &tip);
    }

    fn drain(&mut self) {
        while let Ok(done) = self.rx.try_recv() {
            self.handle(done);
        }
        if self.dash_visible() {
            unsafe { InvalidateRect(self.dash, null(), 0) };
        }
    }

    fn handle(&mut self, done: Done) {
        match done {
            Done::Message { text, ok, rescan } => {
                util::log(&format!("action: {text}"));
                self.busy = None;
                if !self.dash_visible() && self.tray_ok {
                    tray::balloon(self.main, if ok { "Done" } else { "Couldn't finish" }, &text, false);
                }
                self.status = Some((text, ok));
                if rescan {
                    self.start_background_scan();
                }
            }
            Done::Leftover { key, pids } => {
                self.busy = None;
                self.status = Some((
                    format!("{} is still running. Use Force end if it is stuck.", advice::display_name(&key)),
                    false,
                ));
                self.leftover = Some((key, pids));
            }
            Done::Scanned { dirs, temp_bytes, at } => {
                self.busy = None;
                let limit = self.config.junk_alert_gb as u64 * (1 << 30);
                let big: Vec<(String, u64)> =
                    dirs.iter().filter(|d| d.bytes >= limit).map(|d| (d.path.display().to_string(), d.bytes)).collect();
                self.junk = dirs;
                self.junk_at = Some(at);
                self.temp_bytes = Some(temp_bytes);
                for (path, bytes) in big {
                    let short = actions::short_path(std::path::Path::new(&path));
                    self.notify(
                        &format!("junk-{path}"),
                        DAY,
                        &format!("A build folder grew to {}", fmt_bytes(bytes)),
                        &format!("{short} can be deleted safely; it is rebuilt on the next build. Click to review."),
                        false,
                    );
                }
            }
            Done::AutoTemp { freed } => {
                self.busy = None;
                if freed >= 1 << 30 {
                    self.status = Some((format!("Old Temp files cleaned: {} freed.", fmt_bytes(freed)), true));
                }
            }
            Done::Procs { groups, alert } => {
                self.groups = groups;
                if let Some(trigger) = alert {
                    let culprit = pressure::offender(&self.groups, trigger.kind).cloned();
                    let (title, body) = advice::message(&trigger, culprit.as_ref());
                    self.last_alert = Some((trigger, culprit.map(|g| g.key)));
                    // The pressure guard already rate-limits these.
                    util::log(&format!("notify: {title} - {body}"));
                    if self.tray_ok {
                        tray::balloon(self.main, &title, &body, trigger.level == Level::Critical);
                    }
                }
            }
        }
    }

    pub fn pagefile_small(&self) -> bool {
        pagefile::too_small(self.pagefile, self.snap.physical_total, self.snap.commit_limit)
    }

    /// Re-read the process list now (the dashboard just opened).
    pub fn refresh_groups(&mut self) {
        self.groups_at = now_ms();
        let _ = self.proc_tx.send(None);
    }

    /// Ask Windows to run `action` after the current message returns.
    pub fn queue(&mut self, action: Action) {
        self.pending = Some(action);
        unsafe { PostMessageW(self.main, WM_APP_ACTION, 0, 0) };
    }

    pub fn toggle_section(&mut self, id: &str) {
        let list = &mut self.config.collapsed_sections;
        if let Some(pos) = list.iter().position(|s| s == id) {
            list.remove(pos);
        } else {
            list.push(id.to_string());
        }
        let value = list.join(", ");
        save_setting(self, "collapsed-sections", &value);
    }
}

fn lower_thread_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    // Background mode also lowers disk I/O priority, so a scan never slows
    // the apps you are using.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
}

/// Run the queued action: confirm if needed (with the app released), then
/// do it.
fn run_pending() {
    let Some((action, owner, busy)) = with_app(|app| {
        let owner = if app.dash_visible() { app.dash } else { app.main };
        app.pending.take().map(|a| (a, owner, app.busy.clone()))
    })
    .flatten() else {
        return;
    };

    match &action {
        Action::OpenSettingsFile => {
            util::shell_open("notepad.exe", Some(&format!("\"{}\"", util::config_path().display())));
            return;
        }
        Action::OpenStorageSettings => {
            util::shell_open("ms-settings:storagesense", None);
            return;
        }
        Action::OpenPagefileSettings => {
            util::shell_open("SystemPropertiesPerformance.exe", None);
            message_box(
                owner,
                "Raise the page file",
                "In the window that opened: Advanced tab → Virtual memory → Change…\n\nEither tick \"Automatically manage paging file size\", or choose C:, Custom size, and enter 16384 for Initial and 24576 for Maximum. Click Set, then OK, and restart.",
                MB_OK | MB_ICONINFORMATION,
            );
            return;
        }
        Action::ScanJunk => {
            with_app(|app| {
                if app.busy.is_some() {
                    app.status = Some((format!("Busy: {}…", app.busy.clone().unwrap_or_default()), false));
                } else {
                    app.start_background_scan();
                }
            });
            return;
        }
        _ => {}
    }

    if let Some(label) = busy {
        with_app(|app| app.status = Some((format!("Busy: {label}… try again when it finishes."), false)));
        return;
    }
    if let Some((title, text)) = actions::confirm_text(&action) {
        with_app(|app| app.modal = true);
        let answer = message_box(owner, &title, &text, MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2);
        with_app(|app| app.modal = false);
        if answer != IDYES {
            return;
        }
    }
    with_app(|app| {
        if let Action::ForceEnd { .. } | Action::CloseApp { .. } = action {
            app.leftover = None;
        }
        app.busy = Some(busy_label(&action));
        app.status = None;
        actions::spawn(action, scan_roots(&app.config), app.reply.clone());
        if app.dash_visible() {
            unsafe { InvalidateRect(app.dash, null(), 0) };
        }
    });
}

fn busy_label(action: &Action) -> String {
    match action {
        Action::CloseApp { key, .. } => format!("Closing {}", advice::display_name(key)),
        Action::ForceEnd { key, .. } => format!("Ending {}", advice::display_name(key)),
        Action::ShutdownWsl => "Shutting down WSL".into(),
        Action::DockerPrune => "Cleaning Docker".into(),
        Action::CompactDocker => "Compacting the Docker disk".into(),
        Action::CleanJunk(_) | Action::CleanAllJunk(_) => "Deleting build folders".into(),
        Action::CleanTemp => "Emptying Temp".into(),
        Action::CleanCaches => "Clearing package caches".into(),
        _ => "Working".into(),
    }
}

pub fn message_box(owner: HWND, title: &str, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let title = wide(title);
    let text = wide(text);
    unsafe { MessageBoxW(owner, text.as_ptr(), title.as_ptr(), flags | MB_SETFOREGROUND | MB_TOPMOST) }
}

const MENU_DASH: usize = 1;
const MENU_BAR: usize = 2;
const MENU_AUTOSTART: usize = 3;
const MENU_SETTINGS: usize = 4;
const MENU_PIN: usize = 5;
const MENU_LOG: usize = 6;
const MENU_SCAN: usize = 7;
const MENU_EXIT: usize = 9;
/// Position items: MENU_POSITION + index in Anchor::ALL.
const MENU_POSITION: usize = 20;

/// Tray or bar right-click menu. Runs with the app released.
pub fn show_menu() {
    let Some((main, bar_visible, pinned, position)) =
        with_app(|app| (app.main, app.config.bar_visible, app.config.bar_pinned, app.config.bar_position))
    else {
        return;
    };
    let autostart = util::autostart_enabled();
    let choice = unsafe {
        let menu = CreatePopupMenu();
        let add = |id: usize, text: &str, checked: bool| {
            let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
            AppendMenuW(menu, flags, id, wide(text).as_ptr());
        };
        add(MENU_DASH, "Open dashboard", false);
        add(MENU_SCAN, "Scan build folders now", false);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let positions = CreatePopupMenu();
        for (i, anchor) in Anchor::ALL.into_iter().enumerate() {
            let flags = MF_STRING | if anchor == position { MF_CHECKED } else { MF_UNCHECKED };
            AppendMenuW(positions, flags, MENU_POSITION + i, wide(anchor.label()).as_ptr());
        }
        AppendMenuW(menu, MF_POPUP, positions as usize, wide("Line position").as_ptr());
        add(MENU_PIN, "Pin readings", pinned);
        add(MENU_BAR, "Show activity line", bar_visible);
        add(MENU_AUTOSTART, "Start with Windows", autostart);
        add(MENU_SETTINGS, "Settings…", false);
        add(MENU_LOG, "Open log", false);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        add(MENU_EXIT, "Exit Stats", false);
        let mut point: POINT = std::mem::zeroed();
        GetCursorPos(&mut point);
        // Required so the menu closes when you click elsewhere.
        SetForegroundWindow(main);
        let choice =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, point.x, point.y, 0, main, null());
        DestroyMenu(menu);
        // Documented companion to SetForegroundWindow for tray menus.
        PostMessageW(main, WM_NULL, 0, 0);
        choice as usize
    };
    match choice {
        MENU_DASH => {
            with_app(dashboard::show);
        }
        MENU_SCAN => {
            with_app(|app| app.queue(Action::ScanJunk));
        }
        MENU_BAR => {
            with_app(|app| {
                app.config.bar_visible = !app.config.bar_visible;
                let value = app.config.bar_visible.to_string();
                save_setting(app, "bar-visible", &value);
                app.update_overlay();
            });
        }
        MENU_PIN => {
            with_app(|app| bar::set_pinned(app, !pinned));
        }
        n if (MENU_POSITION..MENU_POSITION + Anchor::ALL.len()).contains(&n) => {
            with_app(|app| dashboard::set_position(app, Anchor::ALL[n - MENU_POSITION]));
        }
        MENU_AUTOSTART => {
            if !util::set_autostart(!autostart) {
                util::log("could not change the startup setting");
            }
        }
        MENU_SETTINGS => {
            with_app(|app| app.queue(Action::OpenSettingsFile));
        }
        MENU_LOG => util::shell_open("notepad.exe", Some(&format!("\"{}\"", util::log_path().display()))),
        MENU_EXIT => unsafe {
            DestroyWindow(main);
        },
        _ => {}
    }
}

unsafe extern "system" fn main_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    guarded(hwnd, msg, wparam, lparam, || {
        match msg {
            WM_TIMER if wparam == TIMER_ID => {
                with_app(|app| app.tick());
                Some(0)
            }
            WM_APP_WAKE => {
                with_app(|app| app.drain());
                Some(0)
            }
            WM_APP_ACTION => {
                run_pending();
                Some(0)
            }
            WM_APP_SHOW_DASH => {
                with_app(dashboard::show);
                Some(0)
            }
            super::tray::WM_APP_TRAY => {
                let event = (lparam & 0xFFFF) as u32;
                const NIN_SELECT: u32 = WM_USER;
                const NIN_KEYSELECT: u32 = WM_USER + 1;
                const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;
                match event {
                    WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => {
                        with_app(dashboard::toggle);
                    }
                    NIN_BALLOONUSERCLICK => {
                        with_app(dashboard::show);
                    }
                    WM_CONTEXTMENU | WM_RBUTTONUP => show_menu(),
                    _ => {}
                }
                Some(0)
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE | WM_DPICHANGED => {
                with_app(|app| {
                    app.sensors.refresh_displays();
                    line::place(app);
                    app.update_overlay();
                });
                None
            }
            WM_WTSSESSION_CHANGE => {
                if wparam == WTS_SESSION_LOCK || wparam == WTS_SESSION_UNLOCK {
                    with_app(|app| {
                        app.locked = wparam == WTS_SESSION_LOCK;
                        util::log(if app.locked { "session locked: line paused" } else { "session unlocked" });
                        app.update_overlay();
                    });
                }
                Some(0)
            }
            WM_QUERYENDSESSION => Some(1),
            WM_DESTROY => {
                with_app(|app| {
                    windows_sys::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification(app.main);
                    tray::remove(app.main);
                    icon::destroy(app.icon);
                });
                PostQuitMessage(0);
                Some(0)
            }
            _ => {
                let created = with_app(|app| app.taskbar_created).unwrap_or(0);
                if created != 0 && msg == created {
                    // Explorer restarted: put the icon back.
                    with_app(|app| {
                        app.tray_ok = tray::add(app.main, app.icon, "Stats");
                    });
                    return Some(0);
                }
                None
            }
        }
    })
}
