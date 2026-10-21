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
    /// Files relocated to the data dir.
    pub moved: usize,
    /// Files left behind because the destination already had them.
    pub skipped: usize,
    /// True when the legacy directory is gone (migration complete).
    pub done: bool,
}

/// Move the legacy `~/.cache/music-player-tui` tree into the data dir.
///
/// Idempotent: it can be called on every start. Files already present in the
/// destination are never overwritten.
pub fn migrate_legacy_cache() -> Migration {
    let legacy = legacy_cache_dir();
    let target = data_dir();
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
        match move_path(&src, &dst) {
            Ok(()) => report.moved += 1,
            Err(e) => crate::dlog!(
                "[paths] move failed {} -> {}: {}",
                src.display(),
                dst.display(),
                e
            ),
        }
    }

    // Drop the legacy dir when it is empty; leftovers are kept for safety.
    if let Ok(mut it) = std::fs::read_dir(legacy) {
        if it.next().is_none() {
            let _ = std::fs::remove_dir(legacy);
            report.done = true;
        }
    }
    report
}

/// Rename a file or directory, falling back to a recursive copy across
/// filesystems.
fn move_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            if src.is_dir() {
                copy_dir(src, dst)?;
                let _ = std::fs::remove_dir_all(src);
            } else {
                std::fs::copy(src, dst)?;
                let _ = std::fs::remove_file(src);
            }
            Ok(())
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
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
    fn migration_moves_files_and_keeps_existing() {
        let root = tmp("migrate");
        let legacy = root.join("cache");
        let target = root.join("data");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("a.pcm"), b"aaa").unwrap();
        std::fs::create_dir_all(legacy.join("downloads")).unwrap();
        std::fs::write(legacy.join("downloads/b.mp3"), b"bbb").unwrap();

        let report = migrate_into(&legacy, &target);
        assert_eq!(report.moved, 2);
        assert!(report.done);
        assert!(target.join("a.pcm").exists());
        assert!(target.join("downloads/b.mp3").exists());
        assert!(!legacy.exists(), "legacy dir should be removed");

        // Re-running must not duplicate or fail.
        let report2 = migrate_into(&legacy, &target);
        assert_eq!(report2.moved, 0);
        assert_eq!(report2.skipped, 0);
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
        assert_eq!(report.moved, 0);
        assert_eq!(std::fs::read(target.join("a.pcm")).unwrap(), b"new");
    }
}
