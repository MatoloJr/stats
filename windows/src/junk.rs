//! Finding build output that can be regenerated (the 88 GB Rust `target`
//! folder problem), and deleting it safely.
//!
//! A folder only counts when the file that proves what it is sits next to it:
//! `target` needs `Cargo.toml`, `build` needs `pubspec.yaml`, and so on. A
//! random folder called `build` or `target` is never touched.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum JunkKind {
    RustTarget,
    NodeModules,
    NextCache,
    FlutterBuild,
    DartTool,
    Gradle,
}

impl JunkKind {
    pub fn label(self) -> &'static str {
        match self {
            JunkKind::RustTarget => "Rust target",
            JunkKind::NodeModules => "node_modules",
            JunkKind::NextCache => "Next.js .next",
            JunkKind::FlutterBuild => "Flutter build",
            JunkKind::DartTool => ".dart_tool",
            JunkKind::Gradle => "Gradle cache",
        }
    }

    /// What brings it back.
    pub fn rebuild_hint(self) -> &'static str {
        match self {
            JunkKind::RustTarget => "rebuilt by the next cargo build",
            JunkKind::NodeModules => "restored by npm/pnpm install",
            JunkKind::NextCache => "rebuilt by next dev/build",
            JunkKind::FlutterBuild | JunkKind::DartTool => "rebuilt by flutter pub get / build",
            JunkKind::Gradle => "rebuilt by the next Gradle build",
        }
    }
}

/// Is `name`, sitting among `siblings` (file and folder names in the same
/// parent, lower case), regenerable build output?
pub fn classify(name: &str, siblings: &HashSet<String>) -> Option<JunkKind> {
    let has = |n: &str| siblings.contains(n);
    match name.to_ascii_lowercase().as_str() {
        "target" if has("cargo.toml") => Some(JunkKind::RustTarget),
        "node_modules" if has("package.json") => Some(JunkKind::NodeModules),
        ".next" if has("package.json") => Some(JunkKind::NextCache),
        "build" if has("pubspec.yaml") => Some(JunkKind::FlutterBuild),
        ".dart_tool" if has("pubspec.yaml") => Some(JunkKind::DartTool),
        ".gradle"
            if has("build.gradle")
                || has("build.gradle.kts")
                || has("settings.gradle")
                || has("settings.gradle.kts") =>
        {
            Some(JunkKind::Gradle)
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct JunkDir {
    pub path: PathBuf,
    pub kind: JunkKind,
    pub bytes: u64,
}

/// Folders never descended into: they are huge, irrelevant, or both.
const SKIP: &[&str] = &[".git", ".hg", ".svn", "$recycle.bin", "appdata", ".cargo", ".rustup", ".pub-cache"];
const MAX_DEPTH: usize = 10;
const MAX_DIRS: usize = 200_000;

/// Lower-case names of everything in a folder, plus its subfolders.
type Listing = (HashSet<String>, Vec<(String, PathBuf)>);

fn names_in(dir: &Path) -> Option<Listing> {
    let entries = fs::read_dir(dir).ok()?;
    let mut names = HashSet::new();
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // file_type() does not follow links, so symlinks and junctions
        // (pnpm uses junctions) are never walked into.
        if let Ok(ft) = entry.file_type() {
            if ft.is_dir() {
                subdirs.push((name.clone(), entry.path()));
            }
        }
        names.insert(name.to_ascii_lowercase());
    }
    Some((names, subdirs))
}

/// Total size of the files under `path`, without following links.
pub fn dir_size(path: &Path, cancel: &AtomicBool) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                total = total.saturating_add(entry.metadata().map(|m| m.len()).unwrap_or(0));
            }
        }
    }
    total
}

/// Walk `roots`, returning build-output folders largest first. Stops early
/// when `cancel` is set. Never descends into a folder it reported.
pub fn scan(roots: &[PathBuf], cancel: &AtomicBool) -> Vec<JunkDir> {
    let mut found = Vec::new();
    let mut visited = 0usize;
    for root in roots {
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if cancel.load(Ordering::Relaxed) || visited >= MAX_DIRS {
                break;
            }
            visited += 1;
            let Some((siblings, subdirs)) = names_in(&dir) else { continue };
            for (name, path) in subdirs {
                if let Some(kind) = classify(&name, &siblings) {
                    let bytes = dir_size(&path, cancel);
                    found.push(JunkDir { path, kind, bytes });
                } else if depth < MAX_DEPTH && !SKIP.contains(&name.to_ascii_lowercase().as_str()) {
                    stack.push((path, depth + 1));
                }
            }
        }
    }
    found.sort_by_key(|d| std::cmp::Reverse(d.bytes));
    found
}

#[derive(Debug, PartialEq, Eq)]
pub enum CleanError {
    OutsideRoots,
    NoLongerBuildOutput,
    Io(String),
}

impl std::fmt::Display for CleanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CleanError::OutsideRoots => write!(f, "it is outside the scanned folders"),
            CleanError::NoLongerBuildOutput => write!(f, "it no longer looks like build output"),
            CleanError::Io(e) => write!(f, "{e}"),
        }
    }
}

fn is_inside(path: &Path, roots: &[PathBuf]) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
    let target = norm(path);
    roots.iter().any(|root| {
        let root = norm(root);
        !root.is_empty() && target.len() > root.len() + 1 && target.starts_with(&format!("{root}\\"))
    })
}

/// Delete one build-output folder after checking again, right before
/// deleting, that it is inside a scan root and still next to the file that
/// identifies it. Returns the bytes freed.
pub fn clean(path: &Path, roots: &[PathBuf]) -> Result<u64, CleanError> {
    if !is_inside(path, roots) || path.components().any(|c| c.as_os_str() == "..") {
        return Err(CleanError::OutsideRoots);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let parent = path.parent().ok_or(CleanError::OutsideRoots)?;
    let (siblings, _) = names_in(parent).ok_or(CleanError::NoLongerBuildOutput)?;
    let meta = fs::symlink_metadata(path).map_err(|e| CleanError::Io(e.to_string()))?;
    if classify(&name, &siblings).is_none() || !meta.is_dir() {
        return Err(CleanError::NoLongerBuildOutput);
    }
    let bytes = dir_size(path, &AtomicBool::new(false));
    // remove_dir_all removes junctions and symlinks themselves, never what
    // they point to.
    fs::remove_dir_all(path).map_err(|e| CleanError::Io(e.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stats-junk-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        File::create(path).unwrap().write_all(&vec![0u8; bytes]).unwrap();
    }

    #[test]
    fn classify_needs_proof() {
        let cargo: HashSet<String> = ["cargo.toml".to_string()].into();
        let none = HashSet::new();
        assert_eq!(classify("target", &cargo), Some(JunkKind::RustTarget));
        assert_eq!(classify("target", &none), None);
        assert_eq!(classify("build", &cargo), None);
    }

    #[test]
    fn scan_and_clean() {
        let root = tmp("scan");
        let app = root.join("clubhouse");
        write(&app.join("Cargo.toml"), 10);
        write(&app.join("target/debug/big.bin"), 5000);
        write(&app.join("target/debug/deps/x.rlib"), 1000);
        // Nested crate inside target must not be reported separately.
        write(&app.join("target/inner/Cargo.toml"), 1);
        write(&app.join("target/inner/target/y"), 1);
        let web = root.join("web");
        write(&web.join("package.json"), 2);
        write(&web.join("node_modules/a/index.js"), 300);
        // A 'build' folder with no pubspec.yaml is someone's real data.
        write(&root.join("docs/build/keep.txt"), 50);

        let found = scan(std::slice::from_ref(&root), &AtomicBool::new(false));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].kind, JunkKind::RustTarget);
        assert_eq!(found[0].bytes, 6002);
        assert_eq!(found[1].kind, JunkKind::NodeModules);

        // Refuses paths outside the roots and plain folders.
        assert_eq!(clean(&root.join("docs/build"), std::slice::from_ref(&root)), Err(CleanError::NoLongerBuildOutput));
        assert_eq!(clean(&app.join("target"), std::slice::from_ref(&web)), Err(CleanError::OutsideRoots));
        assert_eq!(clean(&root, std::slice::from_ref(&root)), Err(CleanError::OutsideRoots));

        assert_eq!(clean(&app.join("target"), std::slice::from_ref(&root)), Ok(6002));
        assert!(!app.join("target").exists());
        assert!(app.join("Cargo.toml").exists());
        assert!(root.join("docs/build/keep.txt").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
