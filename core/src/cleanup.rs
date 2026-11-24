//! Emptying the Temp folder without touching anything in use.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub freed: u64,
    pub removed: u64,
    /// Files that were in use, too new, or not ours to delete.
    pub skipped: u64,
}

impl Report {
    fn add(&mut self, other: Report) {
        self.freed += other.freed;
        self.removed += other.removed;
        self.skipped += other.skipped;
    }
}

fn old_enough(meta: &fs::Metadata, min_age: Duration, now: SystemTime) -> bool {
    if min_age.is_zero() {
        return true;
    }
    let modified = meta.modified().ok();
    let accessed = meta.accessed().ok();
    let newest = modified.max(accessed).or(modified);
    match newest {
        Some(t) => now.duration_since(t).map(|age| age >= min_age).unwrap_or(false),
        None => false,
    }
}

/// Delete files under `dir` (but not `dir` itself) whose last change is
/// older than `min_age`, then remove folders left empty. Files that are open
/// in another program fail to delete and are counted as skipped. Links are
/// removed as links, never followed.
pub fn clean_older_than(dir: &Path, min_age: Duration, now: SystemTime) -> Report {
    let mut report = Report::default();
    let Ok(entries) = fs::read_dir(dir) else { return report };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            report.skipped += 1;
            continue;
        };
        if meta.is_dir() {
            report.add(clean_older_than(&path, min_age, now));
            // Fails harmlessly when something inside was kept.
            if fs::remove_dir(&path).is_ok() {
                report.removed += 1;
            }
        } else if old_enough(&meta, min_age, now) {
            let size = if meta.is_file() { meta.len() } else { 0 };
            let removed = if meta.file_type().is_symlink() && fs::remove_dir(&path).is_ok() {
                true
            } else {
                fs::remove_file(&path).is_ok()
            };
            if removed {
                report.freed += size;
                report.removed += 1;
            } else {
                report.skipped += 1;
            }
        } else {
            report.skipped += 1;
        }
    }
    report
}

/// Size of everything under `dir`, for the dashboard.
pub fn folder_size(dir: &Path) -> u64 {
    crate::junk::dir_size(dir, &std::sync::atomic::AtomicBool::new(false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn keeps_new_files_and_the_root() {
        let root = std::env::temp_dir().join(format!("stats-clean-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub/deeper")).unwrap();
        fs::File::create(root.join("a.tmp")).unwrap().write_all(&[0; 100]).unwrap();
        fs::File::create(root.join("sub/deeper/b.tmp")).unwrap().write_all(&[0; 50]).unwrap();

        // Everything is brand new: a 7-day rule keeps it all.
        let kept = clean_older_than(&root, Duration::from_secs(7 * 86400), SystemTime::now());
        assert_eq!(kept.removed, 0);
        assert!(root.join("a.tmp").exists());

        // Pretend it is a month later.
        let later = SystemTime::now() + Duration::from_secs(30 * 86400);
        let report = clean_older_than(&root, Duration::from_secs(7 * 86400), later);
        assert_eq!(report.freed, 150);
        assert_eq!(report.removed, 4); // two files and two emptied folders
        assert!(root.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir_all(&root).unwrap();
    }
}
