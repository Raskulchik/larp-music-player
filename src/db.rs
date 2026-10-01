use crate::api::{Source, Track};
use rusqlite::{params, Connection};
use std::path::PathBuf;

fn db_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("music-player-tui")
        .join("liked.db")
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: usize,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open() -> anyhow::Result<Self> {
        let path = db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&path)?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS liked (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                track_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                artwork_url TEXT,
                duration_ms INTEGER,
                preview_url TEXT,
                liked_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(source, track_id)
            );

            CREATE TABLE IF NOT EXISTS playlists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS playlist_tracks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                playlist_id INTEGER NOT NULL,
                source TEXT NOT NULL,
                track_id TEXT NOT NULL,
                title TEXT NOT NULL,
                artist TEXT NOT NULL,
                artwork_url TEXT,
                duration_ms INTEGER,
                preview_url TEXT,
                position INTEGER NOT NULL,
                FOREIGN KEY (playlist_id) REFERENCES playlists(id) ON DELETE CASCADE,
                UNIQUE(playlist_id, source, track_id)
            );",
        )?;

        Ok(Self { conn })
    }

    pub fn like_track(&self, track: &Track) -> anyhow::Result<()> {
        let source_str = match track.source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        self.conn.execute(
            "INSERT OR REPLACE INTO liked (source, track_id, title, artist, artwork_url, duration_ms, preview_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                source_str,
                track.id,
                track.title,
                track.artist,
                track.artwork_url,
                track.duration_ms.map(|d| d as i64),
                track.preview_url,
            ],
        )?;
        Ok(())
    }

    pub fn unlike_track(&self, source: &Source, track_id: &str) -> anyhow::Result<()> {
        let source_str = match source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        self.conn.execute(
            "DELETE FROM liked WHERE source = ?1 AND track_id = ?2",
            params![source_str, track_id],
        )?;
        Ok(())
    }

    pub fn is_liked(&self, source: &Source, track_id: &str) -> anyhow::Result<bool> {
        let source_str = match source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM liked WHERE source = ?1 AND track_id = ?2",
            params![source_str, track_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Patch duration_ms / artwork_url on an existing liked row (used to fix `??:??`).
    pub fn update_liked_meta(
        &self,
        source: &Source,
        track_id: &str,
        duration_ms: Option<u64>,
        artwork_url: Option<&str>,
    ) -> anyhow::Result<()> {
        let source_str = match source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        let dur = duration_ms.map(|d| d as i64);
        self.conn.execute(
            "UPDATE liked SET duration_ms = ?1, artwork_url = ?2 WHERE source = ?3 AND track_id = ?4",
            params![dur, artwork_url, source_str, track_id],
        )?;
        Ok(())
    }

    pub fn get_liked(&self) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, track_id, title, artist, artwork_url, duration_ms, preview_url FROM liked ORDER BY liked_at DESC"
        )?;

        let tracks = stmt
            .query_map([], |row| {
                let source_str: String = row.get(0)?;
                let source = match source_str.as_str() {
                    "yandex" => Source::YandexMusic,
                    "soundcloud" => Source::SoundCloud,
                    "ytmusic" => Source::YouTubeMusic,
                    _ => Source::ITunes,
                };
                Ok(Track {
                    id: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    source,
                    artwork_url: row.get(4)?,
                    duration_ms: row.get::<_, Option<i64>>(5)?.map(|d| d as u64),
                    preview_url: row.get(6)?,
                    album: None,
                    year: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(tracks)
    }

    pub fn create_playlist(&self, name: &str) -> anyhow::Result<i64> {
        self.conn
            .execute("INSERT INTO playlists (name) VALUES (?1)", params![name])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_playlists(&self) -> anyhow::Result<Vec<Playlist>> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name,
                    (SELECT COUNT(*) FROM playlist_tracks pt WHERE pt.playlist_id = p.id) AS cnt
             FROM playlists p ORDER BY p.created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    count: row.get::<_, i64>(2)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn delete_playlist(&self, id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM playlists WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn add_to_playlist(&self, playlist_id: i64, track: &Track) -> anyhow::Result<()> {
        let source_str = match track.source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        let pos: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM playlist_tracks WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )?;
        self.conn.execute(
            "INSERT OR IGNORE INTO playlist_tracks
             (playlist_id, source, track_id, title, artist, artwork_url, duration_ms, preview_url, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                playlist_id,
                source_str,
                track.id,
                track.title,
                track.artist,
                track.artwork_url,
                track.duration_ms.map(|d| d as i64),
                track.preview_url,
                pos,
            ],
        )?;
        Ok(())
    }

    pub fn remove_from_playlist(
        &self,
        playlist_id: i64,
        source: &Source,
        track_id: &str,
    ) -> anyhow::Result<()> {
        let source_str = match source {
            Source::YandexMusic => "yandex",
            Source::ITunes => "itunes",
            Source::SoundCloud => "soundcloud",
            Source::YouTubeMusic => "ytmusic",
        };
        self.conn.execute(
            "DELETE FROM playlist_tracks WHERE playlist_id = ?1 AND source = ?2 AND track_id = ?3",
            params![playlist_id, source_str, track_id],
        )?;
        Ok(())
    }

    pub fn get_playlist_tracks(&self, playlist_id: i64) -> anyhow::Result<Vec<Track>> {
        let mut stmt = self.conn.prepare(
            "SELECT source, track_id, title, artist, artwork_url, duration_ms, preview_url
             FROM playlist_tracks WHERE playlist_id = ?1 ORDER BY position ASC",
        )?;
        let tracks = stmt
            .query_map(params![playlist_id], |row| {
                let source_str: String = row.get(0)?;
                let source = match source_str.as_str() {
                    "yandex" => Source::YandexMusic,
                    "soundcloud" => Source::SoundCloud,
                    "ytmusic" => Source::YouTubeMusic,
                    _ => Source::ITunes,
                };
                Ok(Track {
                    id: row.get(1)?,
                    title: row.get(2)?,
                    artist: row.get(3)?,
                    source,
                    artwork_url: row.get(4)?,
                    duration_ms: row.get::<_, Option<i64>>(5)?.map(|d| d as u64),
                    preview_url: row.get(6)?,
                    album: None,
                    year: None,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(tracks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_crud() {
        let db = Database::open().expect("open db");
        let name = format!("__test_pl_{}", std::process::id());
        let id = db.create_playlist(&name).expect("create");
        let track = Track {
            id: "test-video-id".to_string(),
            title: "Test Song".to_string(),
            artist: "Test Artist".to_string(),
            source: Source::ITunes,
            preview_url: None,
            artwork_url: None,
            duration_ms: Some(180_000),
            album: None,
            year: None,
        };
        db.add_to_playlist(id, &track).expect("add");
        // duplicate add should be ignored (INSERT OR IGNORE)
        db.add_to_playlist(id, &track).expect("add dup");
        let tracks = db.get_playlist_tracks(id).expect("get tracks");
        assert_eq!(tracks.len(), 1, "duplicate track added twice");
        let pls = db.get_playlists().expect("get playlists");
        assert!(pls.iter().any(|p| p.id == id && p.count == 1));
        db.remove_from_playlist(id, &track.source, &track.id)
            .expect("remove");
        assert_eq!(db.get_playlist_tracks(id).expect("count").len(), 0);
        db.delete_playlist(id).expect("delete");
        assert!(!db.get_playlists().expect("list").iter().any(|p| p.id == id));
    }
}
