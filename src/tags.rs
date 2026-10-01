use crate::dlog;
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::prelude::*;
use lofty::tag::{Accessor, Tag, TagType};
use std::path::Path;

pub fn tag_file(
    path: &Path,
    artist: &str,
    title: &str,
    album: Option<&str>,
    year: Option<u16>,
) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }

    let mut tagged_file = lofty::read_from_path(path)?;

    if tagged_file.primary_tag().is_none() {
        tagged_file.insert_tag(Tag::new(TagType::Id3v2));
    }
    let tag = tagged_file.primary_tag_mut().expect("primary tag exists");

    if !title.is_empty() {
        tag.set_title(title.to_string());
    }
    if !artist.is_empty() {
        tag.set_artist(artist.to_string());
    }
    if let Some(album) = album {
        if !album.is_empty() {
            tag.set_album(album.to_string());
        }
    }
    if let Some(year) = year {
        tag.set_date(lofty::tag::items::Timestamp {
            year,
            ..Default::default()
        });
    }

    tagged_file.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

pub fn tag_track_file(
    path: &Path,
    track: &crate::api::Track,
    album: Option<&str>,
    year: Option<u16>,
) {
    match tag_file(path, &track.artist, &track.title, album, year) {
        Ok(()) => dlog!("[tags] tagged {}", path.display()),
        Err(e) => dlog!("[tags] tagging failed for {}: {}", path.display(), e),
    }
}

#[cfg(test)]
mod tests {
    use super::tag_file;
    use lofty::prelude::*;

    #[test]
    fn tag_and_read() {
        let path = std::path::Path::new("/tmp/opencode/test.mp3");
        if !path.exists() {
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
                    "/tmp/opencode/test.mp3",
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !ok {
                eprintln!("skipping: ffmpeg not available");
                return;
            }
        }
        tag_file(
            path,
            "Test Artist",
            "Test Title",
            Some("Test Album"),
            Some(2001),
        )
        .unwrap();

        let tf = lofty::read_from_path(path).unwrap();
        let tag = tf.primary_tag().unwrap();
        assert_eq!(tag.artist().as_deref(), Some("Test Artist"));
        assert_eq!(tag.title().as_deref(), Some("Test Title"));
        assert_eq!(tag.album().as_deref(), Some("Test Album"));
        let ts = tag.date().unwrap();
        assert_eq!(ts.year, 2001);
    }
}
