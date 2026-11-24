//! Is the page file big enough? Read from the `PagingFiles` registry value
//! (`HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management`),
//! which any user can read.
//!
//! When Windows manages the size itself it grows the file as needed, so
//! Stats does not complain. A fixed size whose maximum is under half the RAM
//! (the 16 GB laptop with a 4 GB page file) is flagged, as is having no page
//! file at all.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageFile {
    SystemManaged,
    /// Sum of the maximum sizes of every fixed page file, in MB.
    Custom {
        max_mb: u64,
    },
    Disabled,
}

/// Each entry is `path [initial_mb max_mb]`. `?:\pagefile.sys` and sizes of
/// `0 0` (or no sizes) mean "system managed".
pub fn parse(entries: &[String]) -> PageFile {
    let mut max_mb = 0u64;
    let mut any = false;
    for entry in entries.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        any = true;
        let mut parts = entry.split_whitespace();
        let path = parts.next().unwrap_or("");
        let initial = parts.next().and_then(|v| v.parse::<u64>().ok());
        let maximum = parts.next().and_then(|v| v.parse::<u64>().ok());
        match (initial, maximum) {
            _ if path.starts_with('?') => return PageFile::SystemManaged,
            (None, _) | (Some(0), Some(0)) | (Some(0), None) => return PageFile::SystemManaged,
            (Some(initial), maximum) => max_mb += maximum.unwrap_or(initial).max(initial),
        }
    }
    if any {
        PageFile::Custom { max_mb }
    } else {
        PageFile::Disabled
    }
}

/// `setting` is None when the registry could not be read; then fall back to
/// the commit limit: under 1.5x RAM means the page file is under half of RAM.
pub fn too_small(setting: Option<PageFile>, ram_bytes: u64, commit_limit: u64) -> bool {
    if ram_bytes == 0 {
        return false;
    }
    match setting {
        Some(PageFile::SystemManaged) => false,
        Some(PageFile::Disabled) => true,
        Some(PageFile::Custom { max_mb }) => max_mb * 1024 * 1024 < ram_bytes / 2,
        None => (commit_limit as f64) < ram_bytes as f64 * 1.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    const GB: u64 = 1 << 30;

    #[test]
    fn parses_registry_forms() {
        assert_eq!(parse(&s(&[r"?:\pagefile.sys"])), PageFile::SystemManaged);
        assert_eq!(parse(&s(&[r"C:\pagefile.sys 0 0"])), PageFile::SystemManaged);
        assert_eq!(parse(&s(&[r"C:\pagefile.sys"])), PageFile::SystemManaged);
        assert_eq!(parse(&s(&[r"C:\pagefile.sys 4096 4096"])), PageFile::Custom { max_mb: 4096 });
        assert_eq!(
            parse(&s(&[r"C:\pagefile.sys 2048 4096", r"D:\pagefile.sys 1024 2048"])),
            PageFile::Custom { max_mb: 6144 }
        );
        assert_eq!(parse(&s(&[])), PageFile::Disabled);
    }

    #[test]
    fn judges_size() {
        assert!(too_small(Some(PageFile::Custom { max_mb: 4096 }), 16 * GB, 0));
        assert!(!too_small(Some(PageFile::Custom { max_mb: 16384 }), 16 * GB, 0));
        assert!(!too_small(Some(PageFile::SystemManaged), 16 * GB, 0));
        assert!(too_small(Some(PageFile::Disabled), 16 * GB, 0));
        // The laptop from the investigation: 15.7 GB RAM, 19.7 GB limit.
        assert!(too_small(None, 16 * GB, 20 * GB));
        assert!(!too_small(None, 16 * GB, 32 * GB));
    }
}
