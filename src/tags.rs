use crate::dlog;
use lofty::config::WriteOptions;
use lofty::file::AudioFile;
use lofty::picture::{MimeType, Picture, PictureType};
use lofty::prelude::*;
use lofty::tag::{Accessor, Tag, TagType};
use std::path::Path;

#[cfg(test)]
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
    artwork: Option<Vec<u8>>,
) {
    match tag_track_file_inner(path, track, album, year, artwork) {
        Ok(()) => dlog!("[tags] tagged {}", path.display()),
        Err(e) => dlog!("[tags] tagging failed for {}: {}", path.display(), e),
    }
}

pub fn has_cover(path: &Path) -> bool {
    lofty::read_from_path(path)
        .ok()
        .and_then(|file| file.primary_tag().map(|tag| !tag.pictures().is_empty()))
        .unwrap_or(false)
}

fn tag_track_file_inner(
    path: &Path,
    track: &crate::api::Track,
    album: Option<&str>,
    year: Option<u16>,
    artwork: Option<Vec<u8>>,
) -> anyhow::Result<()> {
    if !path.exists() {
        anyhow::bail!("file not found: {}", path.display());
    }
    let mut tagged_file = lofty::read_from_path(path)?;
    if tagged_file.primary_tag().is_none() {
        tagged_file.insert_tag(Tag::new(TagType::Id3v2));
    }
    let tag = tagged_file.primary_tag_mut().expect("primary tag exists");
    if !track.title.is_empty() {
        tag.set_title(track.title.clone());
    }
    if !track.artist.is_empty() {
        tag.set_artist(track.artist.clone());
    }
    if let Some(album) = album.filter(|album| !album.is_empty()) {
        tag.set_album(album.to_string());
    } else if let Some(album) = track.album.as_deref().filter(|album| !album.is_empty()) {
        tag.set_album(album.to_string());
    }
    if let Some(year) = year.or(track.year) {
        tag.set_date(lofty::tag::items::Timestamp {
            year,
            ..Default::default()
        });
    }
    if let Some(bytes) = artwork.filter(|bytes| !bytes.is_empty()) {
        let mime_type = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            MimeType::Png
        } else {
            MimeType::Jpeg
        };
        tag.set_picture(
            0,
            Picture::unchecked(bytes)
                .mime_type(mime_type)
                .pic_type(PictureType::CoverFront)
                .build(),
        );
    }
    tagged_file.save_to_path(path, WriteOptions::default())?;
    Ok(())
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
