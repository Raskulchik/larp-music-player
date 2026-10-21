//! Local music library: the music the app downloaded for offline playback.
//!
//! The files live in `<data dir>/downloads` (`~/.local/share/music-player-tui/downloads`)
//! and are named `Artist - Title.mp3`, with tags written at download time.

use crate::api::{Source, Track};
use std::path::{Path, PathBuf};

/// File extensions we treat as playable audio.
const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "wav", "ogg", "oga", "opus", "m4a", "mp4", "aac", "wma", "aiff", "aif", "alac",
    "ape", "wv", "mka", "tta", "dsf",
];

/// Maximum directory depth walked below the music root.
const MAX_DEPTH: usize = 8;

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .map(|e| AUDIO_EXTS.contains(&e.as_str()))
        .unwrap_or(false)
}

/// Recursively collect audio files below `root` (hidden folders are skipped).
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            } else if ft.is_file() && is_audio(&path) {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// Read tags of a single audio file into a `Track`.
///
/// The track id is the absolute file path, so it stays valid as long as the
/// file stays in place and can be used to play the file back later.
pub fn track_for(path: &Path) -> Option<Track> {
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path_str = abs.to_string_lossy().into_owned();
    let stem = abs
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path_str.clone());

    let mut title = stem.clone();
    let mut artist = String::new();
    let mut album = None;
    let mut year = None;
    let mut duration_ms = None;

    if let Ok(tagged) = lofty::read_from_path(&abs) {
        use lofty::prelude::*;
        if let Some(tag) = tagged.primary_tag() {
            if let Some(t) = tag.title() {
                let t = t.trim();
                if !t.is_empty() {
                    title = t.to_string();
                }
            }
            if let Some(a) = tag.artist() {
                let a = a.trim();
                if !a.is_empty() {
                    artist = a.to_string();
                }
            }
            if let Some(al) = tag.album() {
                let al = al.trim();
                if !al.is_empty() {
                    album = Some(al.to_string());
                }
            }
            if let Some(date) = tag.date() {
                year = Some(date.year);
            }
        }
        let dur = tagged.properties().duration();
        duration_ms = Some(dur.as_millis() as u64);
    }

    if artist.is_empty() {
        // Fall back to the parent folder name.
        artist = abs
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Unknown artist".to_string());
    }

    Some(Track {
        id: path_str.clone(),
        title,
        artist,
        source: Source::Local,
        preview_url: Some(path_str),
        artwork_url: None,
        duration_ms,
        album,
        year,
    })
}

/// Scan a music folder and return the tracks sorted by artist and title.
pub fn scan_dir(root: &Path) -> Vec<Track> {
    let mut tracks: Vec<Track> = collect_files(root)
        .into_iter()
        .filter_map(|p| track_for(&p))
        .collect();
    tracks.sort_by(|a, b| {
        a.artist
            .to_lowercase()
            .cmp(&b.artist.to_lowercase())
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    tracks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mp3(path: &Path) {
        let ok = std::process::Command::new("ffmpeg")
            .args([
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-codec:a",
                "libmp3lame",
                "-q:a",
                "9",
            ])
            .arg(path)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "ffmpeg must be available for this test");
    }

    fn tmp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "larp_local_{}_{}_{}",
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
    fn scan_finds_audio_and_ignores_the_rest() {
        let dir = tmp_dir("scan");
        std::fs::create_dir_all(dir.join("Artist")).unwrap();
        make_mp3(&dir.join("Artist/song.mp3"));
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        std::fs::write(dir.join("cover.jpg"), b"x").unwrap();

        let files = collect_files(&dir);
        assert_eq!(files.len(), 1, "only audio files must be collected");

        let tracks = scan_dir(&dir);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].source, Source::Local);
        assert!(tracks[0].id.ends_with("song.mp3"));
        assert!(tracks[0].preview_url.is_some());
        assert!(tracks[0].duration_ms.is_some());
    }

    #[test]
    fn track_falls_back_to_filename_and_parent_folder() {
        let dir = tmp_dir("fallback");
        std::fs::create_dir_all(dir.join("Some Artist")).unwrap();
        make_mp3(&dir.join("Some Artist/unknown title.mp3"));

        let track = track_for(&dir.join("Some Artist/unknown title.mp3")).expect("track");
        assert_eq!(track.title, "unknown title");
        assert_eq!(track.artist, "Some Artist");
    }
}
