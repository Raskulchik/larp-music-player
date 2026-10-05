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

/// Aggregate listening figures across the whole library history.
#[derive(Debug, Clone, Default)]
pub struct StatsTotals {
    pub listened_ms: u64,
    pub plays: u32,
    pub tracks: u32,
    pub completed: u32,
    pub days: u32,
    pub first_day: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TrackStat {
    pub source: Source,
    pub track_id: String,
    pub title: String,
    pub artist: String,
    pub play_count: u32,
    pub completed_count: u32,
    pub skip_count: u32,
    pub listened_ms: u64,
}

#[derive(Debug, Clone)]
pub struct DailyStat {
    pub day: String,
    pub listened_ms: u64,
    pub plays: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum TopBy {
    Plays,
    Listened,
    Completed,
}

/// Format milliseconds as `12h 34m` / `5m 3s`.
pub fn fmt_listened(ms: u64) -> String {
    let total_min = ms / 60_000;
    if total_min >= 60 {
        format!("{}h {}m", total_min / 60, total_min % 60)
    } else {
        format!("{}m {}s", total_min, (ms % 60_000) / 1000)
    }
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open() -> anyhow::Result<Self> {
        Self::open_at(db_path())
    }

    /// Open (creating if needed) a database at an explicit path. Tests use this
    /// so they never touch the real user database.
    pub fn open_at(path: PathBuf) -> anyhow::Result<Self> {
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
            );

            CREATE TABLE IF NOT EXISTS track_stats (
                source TEXT NOT NULL,
                track_id TEXT NOT NULL,
                title TEXT NOT NULL DEFAULT '',
                artist TEXT NOT NULL DEFAULT '',
                play_count INTEGER NOT NULL DEFAULT 0,
                completed_count INTEGER NOT NULL DEFAULT 0,
                skip_count INTEGER NOT NULL DEFAULT 0,
                listened_ms INTEGER NOT NULL DEFAULT 0,
                first_played_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_played_at TEXT NOT NULL DEFAULT (datetime('now')),
                PRIMARY KEY (source, track_id)
            );

            CREATE TABLE IF NOT EXISTS daily_stats (
                day TEXT NOT NULL PRIMARY KEY,
                listened_ms INTEGER NOT NULL DEFAULT 0,
                plays INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_track_stats_plays
                ON track_stats(play_count DESC);
            CREATE INDEX IF NOT EXISTS idx_track_stats_listened
                ON track_stats(listened_ms DESC);",
        )?;

        Ok(Self { conn })
    }

    pub fn like_track(&self, track: &Track) -> anyhow::Result<()> {
        let source_str = source_str(&track.source);
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
        let source_str = source_str(&source);
        self.conn.execute(
            "DELETE FROM liked WHERE source = ?1 AND track_id = ?2",
            params![source_str, track_id],
        )?;
        Ok(())
    }

    pub fn is_liked(&self, source: &Source, track_id: &str) -> anyhow::Result<bool> {
        let source_str = source_str(&source);
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
        let source_str = source_str(&source);
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
        let source_str = source_str(&track.source);
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
        let source_str = source_str(&source);
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

    // ---------- listening statistics ----------

    /// A track was started (counted even if the user skips it right away).
    pub fn record_play_start(
        &self,
        source: &Source,
        track_id: &str,
        title: &str,
        artist: &str,
    ) -> anyhow::Result<()> {
        let src = source_str(source);
        self.conn.execute(
            "INSERT INTO track_stats (source, track_id, title, artist, play_count)
             VALUES (?1, ?2, ?3, ?4, 1)
             ON CONFLICT(source, track_id) DO UPDATE SET
                play_count = play_count + 1,
                title = excluded.title,
                artist = excluded.artist,
                last_played_at = datetime('now')",
            params![src, track_id, title, artist],
        )?;
        self.conn.execute(
            "INSERT INTO daily_stats (day, plays) VALUES (date('now', 'localtime'), 1)
             ON CONFLICT(day) DO UPDATE SET plays = plays + 1",
            [],
        )?;
        Ok(())
    }

    /// Add listened wall-clock time for a track.
    pub fn record_listened(&self, source: &Source, track_id: &str, ms: u64) -> anyhow::Result<()> {
        if ms == 0 {
            return Ok(());
        }
        let src = source_str(source);
        self.conn.execute(
            "UPDATE track_stats SET listened_ms = listened_ms + ?3
             WHERE source = ?1 AND track_id = ?2",
            params![src, track_id, ms as i64],
        )?;
        self.conn.execute(
            "INSERT INTO daily_stats (day, listened_ms) VALUES (date('now', 'localtime'), ?1)
             ON CONFLICT(day) DO UPDATE SET listened_ms = listened_ms + ?1",
            params![ms as i64],
        )?;
        Ok(())
    }

    /// The track reached its end without being skipped.
    pub fn record_completed(&self, source: &Source, track_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE track_stats SET completed_count = completed_count + 1
             WHERE source = ?1 AND track_id = ?2",
            params![source_str(source), track_id],
        )?;
        Ok(())
    }

    /// The track was left before the end.
    pub fn record_skipped(&self, source: &Source, track_id: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE track_stats SET skip_count = skip_count + 1
             WHERE source = ?1 AND track_id = ?2",
            params![source_str(source), track_id],
        )?;
        Ok(())
    }

    /// All tracks with at least one play, keyed by (source, track_id).
    /// Single query so callers can cache instead of hitting the DB per row.
    pub fn play_counts(&self) -> anyhow::Result<std::collections::HashMap<(String, String), u32>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source, track_id, play_count FROM track_stats WHERE play_count > 0")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                row.get::<_, i64>(2)?.max(0) as u32,
            ))
        })?;
        let mut map = std::collections::HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    pub fn stats_totals(&self) -> anyhow::Result<StatsTotals> {
        let (ms, plays, tracks, completed): (i64, i64, i64, i64) = self.conn.query_row(
            "SELECT COALESCE(SUM(listened_ms), 0),
                    COALESCE(SUM(play_count), 0),
                    COUNT(*),
                    COALESCE(SUM(completed_count), 0)
             FROM track_stats",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        let days: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM daily_stats WHERE plays > 0",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let first: Option<String> = self
            .conn
            .query_row("SELECT MIN(day) FROM daily_stats", [], |r| r.get(0))
            .unwrap_or(None);
        Ok(StatsTotals {
            listened_ms: ms.max(0) as u64,
            plays: plays.max(0) as u32,
            tracks: tracks.max(0) as u32,
            completed: completed.max(0) as u32,
            days: days.max(0) as u32,
            first_day: first,
        })
    }

    /// Top tracks ordered by the given metric.
    pub fn top_tracks(&self, by: TopBy, limit: usize) -> anyhow::Result<Vec<TrackStat>> {
        let (order, desc) = match by {
            TopBy::Plays => ("play_count", true),
            TopBy::Listened => ("listened_ms", true),
            TopBy::Completed => ("completed_count", true),
        };
        let sql = format!(
            "SELECT source, track_id, title, artist, play_count, completed_count,
                    skip_count, listened_ms
             FROM track_stats
             WHERE play_count > 0
             ORDER BY {} {}, listened_ms DESC
             LIMIT {}",
            order,
            if desc { "DESC" } else { "ASC" },
            limit
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let source_str: String = row.get(0)?;
            Ok(TrackStat {
                source: match source_str.as_str() {
                    "yandex" => Source::YandexMusic,
                    "soundcloud" => Source::SoundCloud,
                    "ytmusic" => Source::YouTubeMusic,
                    _ => Source::ITunes,
                },
                track_id: row.get(1)?,
                title: row.get(2)?,
                artist: row.get(3)?,
                play_count: row.get::<_, i64>(4)?.max(0) as u32,
                completed_count: row.get::<_, i64>(5)?.max(0) as u32,
                skip_count: row.get::<_, i64>(6)?.max(0) as u32,
                listened_ms: row.get::<_, i64>(7)?.max(0) as u64,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Most recent days with activity (newest first).
    pub fn daily_stats(&self, limit: usize) -> anyhow::Result<Vec<DailyStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT day, listened_ms, plays FROM daily_stats
             WHERE plays > 0 OR listened_ms > 0
             ORDER BY day DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(DailyStat {
                day: row.get(0)?,
                listened_ms: row.get::<_, i64>(1)?.max(0) as u64,
                plays: row.get::<_, i64>(2)?.max(0) as u32,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

pub fn source_key(source: &Source) -> &'static str {
    match source {
        Source::YandexMusic => "yandex",
        Source::ITunes => "itunes",
        Source::SoundCloud => "soundcloud",
        Source::YouTubeMusic => "ytmusic",
    }
}

fn source_str(source: &Source) -> &'static str {
    source_key(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> Database {
        let path = std::env::temp_dir().join(format!(
            "larp_test_{}_{}_{}.db",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&path);
        Database::open_at(path).expect("open temp db")
    }

    #[test]
    fn playlist_crud() {
        let db = temp_db("playlist");
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

    #[test]
    fn stats_totals_and_ranking() {
        let db = temp_db("stats");
        let tid = format!("__stat_track_{}", std::process::id());
        let other = format!("__stat_other_{}", std::process::id());

        db.record_play_start(&Source::YouTubeMusic, &tid, "Song A", "Artist A")
            .expect("play a");
        db.record_listened(&Source::YouTubeMusic, &tid, 120_000)
            .expect("listen a");
        db.record_completed(&Source::YouTubeMusic, &tid)
            .expect("done a");

        // second listen of the same track
        db.record_play_start(&Source::YouTubeMusic, &tid, "Song A", "Artist A")
            .expect("play a2");
        db.record_listened(&Source::YouTubeMusic, &tid, 30_000)
            .expect("listen a2");
        db.record_skipped(&Source::YouTubeMusic, &tid)
            .expect("skip a");

        // a more-played but shorter track
        for _ in 0..5 {
            db.record_play_start(&Source::ITunes, &other, "Song B", "Artist B")
                .expect("play b");
        }
        db.record_listened(&Source::ITunes, &other, 10_000)
            .expect("listen b");

        let counts = db.play_counts().expect("counts");
        assert_eq!(counts.get(&("ytmusic".to_string(), tid.clone())), Some(&2));
        assert_eq!(counts.get(&("itunes".to_string(), other.clone())), Some(&5));

        let by_plays = db.top_tracks(TopBy::Plays, 10).expect("by plays");
        assert_eq!(
            by_plays[0].track_id, other,
            "most-played track must rank first"
        );
        assert_eq!(by_plays[0].play_count, 5);

        let by_listened = db.top_tracks(TopBy::Listened, 10).expect("by listened");
        assert_eq!(
            by_listened[0].track_id, tid,
            "most-listened track must rank first"
        );
        assert_eq!(by_listened[0].listened_ms, 150_000);

        let t = db.stats_totals().expect("totals");
        assert_eq!(t.plays, 7);
        assert_eq!(t.listened_ms, 160_000);
        assert!(t.days >= 1);

        let days = db.daily_stats(5).expect("days");
        assert!(!days.is_empty());
        assert!(days[0].listened_ms >= 160_000);

        assert_eq!(fmt_listened(150_000), "2m 30s");
        assert_eq!(fmt_listened(3_723_000), "1h 2m");

        // cleanup so repeated runs stay deterministic
        db.conn
            .execute(
                "DELETE FROM track_stats WHERE track_id IN (?1, ?2)",
                params![tid, other],
            )
            .expect("cleanup");
    }
}
