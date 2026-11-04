//! XDG base-directory helpers.
//!
//! Music (decoded PCM cache and downloaded tracks) lives in the *data* dir
//! (`~/.local/share/music-player-tui`), not in `~/.cache` — the cache is
//! meant to be disposable, while this data is worth keeping.

use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "music-player-tui";

fn home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// `$XDG_DATA_HOME/music-player-tui` (default `~/.local/share/music-player-tui`).
/// Holds the decoded audio cache and the offline downloads.
pub fn data_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join(APP_NAME);
        }
    }
    home().join(".local").join("share").join(APP_NAME)
}

/// Pre-v0.3.0 location: `~/.cache/music-player-tui`.
pub fn legacy_cache_dir() -> PathBuf {
    home().join(".cache").join(APP_NAME)
}

/// `$XDG_CONFIG_HOME/music-player-tui` (default `~/.config/music-player-tui`).
pub fn config_dir() -> PathBuf {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.is_empty() {
            return PathBuf::from(x).join(APP_NAME);
        }
    }
    home().join(".config").join(APP_NAME)
}

/// The offline library: music downloaded by the app (`<data>/downloads`).
pub fn downloads_dir() -> PathBuf {
    data_dir().join("downloads")
}

/// Result of the one-off cache relocation.
#[derive(Debug, Default, Clone, Copy)]
pub struct Migration {
    /// Files copied to the data dir; the legacy originals are retained.
    pub copied: usize,
    /// Files left behind because the destination already had them.
    pub skipped: usize,
    /// True when the legacy directory is gone (migration complete).
    pub done: bool,
}

/// Copy legacy downloaded songs from `~/.cache/music-player-tui/downloads`
/// into the current data directory.
///
/// Idempotent: it can be called on every start. Files already present in the
/// destination are never overwritten, and the legacy originals are retained.
pub fn migrate_legacy_cache() -> Migration {
    let legacy = legacy_cache_dir().join("downloads");
    let target = downloads_dir();
    migrate_into(&legacy, &target)
}

/// The actual relocation, parameterised so it can be tested.
pub fn migrate_into(legacy: &Path, target: &Path) -> Migration {
    let mut report = Migration::default();
    if !legacy.exists() || legacy == target {
        report.done = true;
        return report;
    }

    if let Err(e) = std::fs::create_dir_all(target) {
        crate::dlog!("[paths] cannot create {}: {}", target.display(), e);
        return report;
    }

    let entries = match std::fs::read_dir(legacy) {
        Ok(e) => e,
        Err(e) => {
            crate::dlog!("[paths] cannot read {}: {}", legacy.display(), e);
            return report;
        }
    };

    for entry in entries.flatten() {
        let src = entry.path();
        let dst = target.join(entry.file_name());
        if dst.exists() {
            report.skipped += 1;
            continue;
        }
        match copy_path(&src, &dst, &mut report) {
            Ok(()) => {}
            Err(e) => crate::dlog!(
                "[paths] copy failed {} -> {}: {}",
                src.display(),
                dst.display(),
                e
            ),
        }
    }
    report.done = true;
    report
}

/// Copy a file tree without replacing existing destination files or deleting
/// anything from the legacy cache.
fn copy_path(src: &Path, dst: &Path, report: &mut Migration) -> std::io::Result<()> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_path(&entry.path(), &dst.join(entry.file_name()), report)?;
        }
        return Ok(());
    }
    if dst.exists() {
        report.skipped += 1;
        return Ok(());
    }
    if src.file_name().and_then(|name| name.to_str()).map(|name| name.ends_with(".part")).unwrap_or(false) {
        report.skipped += 1;
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(src, dst)?;
    report.copied += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "larp_paths_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn migration_copies_files_and_keeps_both_trees() {
        let root = tmp("migrate");
        let legacy = root.join("cache");
        let target = root.join("data");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("a.pcm"), b"aaa").unwrap();
        std::fs::create_dir_all(legacy.join("downloads")).unwrap();
        std::fs::write(legacy.join("downloads/b.mp3"), b"bbb").unwrap();

        let report = migrate_into(&legacy, &target);
        assert_eq!(report.copied, 2);
        assert!(report.done);
        assert!(target.join("a.pcm").exists());
        assert!(target.join("downloads/b.mp3").exists());
        assert!(legacy.join("a.pcm").exists(), "legacy source must be kept");
        assert!(legacy.join("downloads/b.mp3").exists(), "legacy download must be kept");

        // Re-running must not duplicate or fail.
        let report2 = migrate_into(&legacy, &target);
        assert_eq!(report2.copied, 0);
        assert_eq!(report2.skipped, 2);
    }

    #[test]
    fn migration_never_overwrites_existing_files() {
        let root = tmp("migrate_dup");
        let legacy = root.join("cache");
        let target = root.join("data");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(legacy.join("a.pcm"), b"old").unwrap();
        std::fs::write(target.join("a.pcm"), b"new").unwrap();

        let report = migrate_into(&legacy, &target);
        assert_eq!(report.skipped, 1);
        assert_eq!(report.copied, 0);
        assert_eq!(std::fs::read(target.join("a.pcm")).unwrap(), b"new");
        assert_eq!(std::fs::read(legacy.join("a.pcm")).unwrap(), b"old");
    }
}
