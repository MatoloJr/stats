//! Deciding when a resource warning is worth a notification.
//!
//! A port of the GNOME extension's `pressure.js` idea, tuned for Windows:
//! committed memory against the commit limit is checked first, because that
//! is the number that makes Windows refuse memory and crash apps. A warning
//! needs three polls in a row above the level, then stays quiet for
//! `notify-quiet-minutes`, except that crossing into critical is always
//! reported (at most once every two minutes).

use crate::config::{Level, Limits};

pub const STREAK: u32 = 3;
const ESCALATE_GAP_MS: u64 = 2 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Commit,
    Memory,
    Cpu,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Commit => "Committed memory",
            Kind::Memory => "Memory",
            Kind::Cpu => "CPU",
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    pub commit: Option<f64>,
    pub memory: Option<f64>,
    pub cpu: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trigger {
    pub kind: Kind,
    pub level: Level,
    pub value: f64,
}

#[derive(Clone, Copy, Debug, Default)]
struct Track {
    streak: u32,
    last_at: Option<u64>,
    last_level: Option<Level>,
}

#[derive(Debug, Default)]
pub struct Guard {
    tracks: [Track; 3],
}

fn index(kind: Kind) -> usize {
    match kind {
        Kind::Commit => 0,
        Kind::Memory => 1,
        Kind::Cpu => 2,
    }
}

impl Guard {
    /// Feed one poll. Returns at most one trigger per poll, the most urgent:
    /// commit, then memory, then CPU.
    pub fn observe(
        &mut self,
        reading: Reading,
        limits: [(Kind, Limits); 3],
        quiet_ms: u64,
        now_ms: u64,
    ) -> Option<Trigger> {
        let mut fired = None;
        for (kind, limit) in limits {
            let value = match kind {
                Kind::Commit => reading.commit,
                Kind::Memory => reading.memory,
                Kind::Cpu => reading.cpu,
            };
            let level = limit.level(value);
            let track = &mut self.tracks[index(kind)];
            if level == Level::Normal {
                track.streak = 0;
                // Clearing the level lets the next episode warn again at once.
                if track.last_at.is_some_and(|at| now_ms.saturating_sub(at) >= quiet_ms) {
                    track.last_level = None;
                }
                continue;
            }
            track.streak = track.streak.saturating_add(1);
            if track.streak < STREAK || fired.is_some() {
                continue;
            }
            let since = track.last_at.map(|at| now_ms.saturating_sub(at));
            let escalated = level == Level::Critical
                && track.last_level != Some(Level::Critical)
                && since.is_none_or(|gap| gap >= ESCALATE_GAP_MS);
            let quiet_over = since.is_none_or(|gap| gap >= quiet_ms);
            if quiet_over || escalated {
                track.last_at = Some(now_ms);
                track.last_level = Some(level);
                fired = Some(Trigger { kind, level, value: value.unwrap_or(0.0) });
            }
        }
        fired
    }
}

/// One program, all its processes added together (Chrome runs dozens).
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Executable name without `.exe`, lower case. The grouping key.
    pub key: String,
    pub count: u32,
    /// Private (committed) bytes. This is what counts against the limit.
    pub private_bytes: u64,
    /// Share of the whole machine's CPU, 0..100.
    pub cpu: f64,
    pub pids: Vec<u32>,
}

/// The program most responsible for `kind`. Memory and commit use private
/// bytes. CPU uses the CPU share.
pub fn offender(groups: &[Group], kind: Kind) -> Option<&Group> {
    match kind {
        Kind::Commit | Kind::Memory => groups.iter().filter(|g| g.private_bytes > 0).max_by_key(|g| g.private_bytes),
        Kind::Cpu => groups
            .iter()
            .filter(|g| g.cpu > 0.0)
            .max_by(|a, b| a.cpu.partial_cmp(&b.cpu).unwrap_or(std::cmp::Ordering::Equal)),
    }
}

/// Add up per-process samples into per-program groups, largest first.
pub fn group(processes: impl IntoIterator<Item = (u32, String, u64, f64)>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for (pid, name, private_bytes, cpu) in processes {
        let key = name.trim().trim_end_matches(".exe").trim_end_matches(".EXE").to_ascii_lowercase();
        if key.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|g| g.key == key) {
            Some(g) => {
                g.count += 1;
                g.private_bytes += private_bytes;
                g.cpu += cpu;
                g.pids.push(pid);
            }
            None => groups.push(Group { key, count: 1, private_bytes, cpu, pids: vec![pid] }),
        }
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.private_bytes));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> [(Kind, Limits); 3] {
        [(Kind::Commit, Limits::new(80, 92)), (Kind::Memory, Limits::new(85, 95)), (Kind::Cpu, Limits::new(90, 98))]
    }

    fn commit(v: f64) -> Reading {
        Reading { commit: Some(v), memory: Some(50.0), cpu: Some(10.0) }
    }

    const QUIET: u64 = 10 * 60 * 1000;

    #[test]
    fn needs_three_polls() {
        let mut guard = Guard::default();
        assert_eq!(guard.observe(commit(85.0), limits(), QUIET, 0), None);
        assert_eq!(guard.observe(commit(85.0), limits(), QUIET, 1000), None);
        let t = guard.observe(commit(85.0), limits(), QUIET, 2000).unwrap();
        assert_eq!((t.kind, t.level), (Kind::Commit, Level::Warning));
        // Quiet afterwards.
        assert_eq!(guard.observe(commit(85.0), limits(), QUIET, 3000), None);
    }

    #[test]
    fn a_dip_resets_the_streak() {
        let mut guard = Guard::default();
        guard.observe(commit(85.0), limits(), QUIET, 0);
        guard.observe(commit(85.0), limits(), QUIET, 1000);
        guard.observe(commit(50.0), limits(), QUIET, 2000);
        assert_eq!(guard.observe(commit(85.0), limits(), QUIET, 3000), None);
    }

    #[test]
    fn critical_escalates_through_quiet_period() {
        let mut guard = Guard::default();
        for t in 0..3 {
            guard.observe(commit(85.0), limits(), QUIET, t * 1000);
        }
        // Critical within the quiet period but under 2 minutes: held back.
        assert_eq!(guard.observe(commit(95.0), limits(), QUIET, 60_000), None);
        let t = guard.observe(commit(95.0), limits(), QUIET, 125_000).unwrap();
        assert_eq!(t.level, Level::Critical);
        // Not repeated while it stays critical.
        assert_eq!(guard.observe(commit(96.0), limits(), QUIET, 400_000), None);
        // Repeated once the quiet period is over.
        assert!(guard.observe(commit(96.0), limits(), QUIET, 125_000 + QUIET).is_some());
    }

    #[test]
    fn commit_wins_over_cpu() {
        let mut guard = Guard::default();
        let hot = Reading { commit: Some(90.0), memory: Some(50.0), cpu: Some(99.0) };
        guard.observe(hot, limits(), QUIET, 0);
        guard.observe(hot, limits(), QUIET, 1);
        assert_eq!(guard.observe(hot, limits(), QUIET, 2).unwrap().kind, Kind::Commit);
        // CPU fires on the next poll since its streak is still running.
        assert_eq!(guard.observe(hot, limits(), QUIET, 3).unwrap().kind, Kind::Cpu);
    }

    #[test]
    fn grouping_sums_processes() {
        let groups = group(vec![
            (1, "chrome.exe".into(), 700, 2.0),
            (2, "chrome.exe".into(), 600, 1.0),
            (3, "Code.exe".into(), 1000, 30.0),
            (4, "".into(), 5000, 0.0),
        ]);
        assert_eq!(groups[0].key, "chrome");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].private_bytes, 1300);
        assert_eq!(offender(&groups, Kind::Commit).unwrap().key, "chrome");
        assert_eq!(offender(&groups, Kind::Cpu).unwrap().key, "code");
    }
}
