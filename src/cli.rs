//! Command-line mode and the control socket used by a running TUI.
//!
//! Starting the binary without arguments opens the TUI as usual. Any argument
//! switches to CLI mode. Commands that only need the local database or the
//! online catalogs (`search`, `liked`, `stats`, `local`, ...) work on their
//! own; playback commands are sent to a running TUI over a Unix socket.

use crate::api::{self, Source, Track};
use crate::app::{App, Command as AppCommand, RepeatMode, ViewMode};
use crate::config;
use crate::db::{self, Database, TopBy};
use crate::local;
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

const REMOTE_COMMANDS: &[&str] = &[
    "status", "play", "pause", "resume", "toggle", "next", "prev", "stop", "volume", "seek",
    "shuffle", "repeat", "like", "unlike", "lyrics", "queue", "source", "quit",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliRequest {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl CliResponse {
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            data: None,
        }
    }

    pub fn ok_data(message: impl Into<String>, data: Value) -> Self {
        Self {
            ok: true,
            message: Some(message.into()),
            data: Some(data),
        }
    }

    pub fn err(message: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: Some(message.into()),
            data: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Flags {
    positional: Vec<String>,
    json: bool,
    source: Option<String>,
    limit: Option<usize>,
    by: Option<TopBy>,
    help: bool,
}

fn parse_flags(args: &[String]) -> Result<Flags, String> {
    let mut flags = Flags::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            flags.positional.extend_from_slice(&args[i + 1..]);
            break;
        }
        if arg == "-h" || arg == "--help" {
            flags.help = true;
            i += 1;
            continue;
        }
        if arg == "--json" {
            flags.json = true;
            i += 1;
            continue;
        }
        if arg == "-s" || arg == "--source" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("{arg} needs a value"))?;
            flags.source = Some(value.clone());
            i += 2;
            continue;
        }
        if let Some(value) = arg
            .strip_prefix("--source=")
            .or_else(|| arg.strip_prefix("-s="))
        {
            flags.source = Some(value.to_string());
            i += 1;
            continue;
        }
        if arg == "-n" || arg == "--limit" || arg == "--top" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| format!("{arg} needs a value"))?;
            flags.limit = Some(parse_usize(value, arg)?);
            i += 2;
            continue;
        }
        if let Some(value) = arg
            .strip_prefix("--limit=")
            .or_else(|| arg.strip_prefix("-n="))
            .or_else(|| arg.strip_prefix("--top="))
        {
            flags.limit = Some(parse_usize(value, "--limit")?);
            i += 1;
            continue;
        }
        if arg == "--by" {
            let value = args
                .get(i + 1)
                .ok_or_else(|| "--by needs a value".to_string())?;
            flags.by = Some(parse_top_by(value)?);
            i += 2;
            continue;
        }
        if let Some(value) = arg.strip_prefix("--by=") {
            flags.by = Some(parse_top_by(value)?);
            i += 1;
            continue;
        }
        // `seek -5` is a valid argument, not an unknown option.
        if arg.starts_with('-') && arg.parse::<f64>().is_err() {
            return Err(format!("unknown option '{arg}'"));
        }
        flags.positional.push(arg.clone());
        i += 1;
    }
    Ok(flags)
}

fn parse_usize(value: &str, flag: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{flag} expects a number, got '{value}'"))
}

fn parse_top_by(value: &str) -> Result<TopBy, String> {
    match value.to_ascii_lowercase().as_str() {
        "plays" | "play" => Ok(TopBy::Plays),
        "listened" | "time" => Ok(TopBy::Listened),
        "completed" | "finished" => Ok(TopBy::Completed),
        _ => Err(format!(
            "--by expects plays, listened or completed, got '{value}'"
        )),
    }
}

pub fn parse_source(value: &str) -> Option<Source> {
    match value
        .to_ascii_lowercase()
        .replace('_', "-")
        .replace(' ', "-")
        .as_str()
    {
        "yandex" | "yandex-music" | "yandexmusic" | "ym" => Some(Source::YandexMusic),
        "itunes" | "it" | "apple" => Some(Source::ITunes),
        "soundcloud" | "sc" => Some(Source::SoundCloud),
        "ytmusic" | "yt" | "youtube" | "youtube-music" => Some(Source::YouTubeMusic),
        "local" | "file" => Some(Source::Local),
        _ => None,
    }
}

pub fn source_name(source: Source) -> &'static str {
    match source {
        Source::YandexMusic => "yandex",
        Source::ITunes => "itunes",
        Source::SoundCloud => "soundcloud",
        Source::YouTubeMusic => "ytmusic",
        Source::Local => "local",
    }
}

fn top_by_name(by: TopBy) -> &'static str {
    match by {
        TopBy::Plays => "plays",
        TopBy::Listened => "listened",
        TopBy::Completed => "completed",
    }
}

pub fn repeat_name(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    }
}

fn view_name(view: &ViewMode) -> &'static str {
    match view {
        ViewMode::Search => "search",
        ViewMode::Liked => "liked",
        ViewMode::Lyrics => "lyrics",
        ViewMode::Playlists => "playlists",
        ViewMode::Stats => "stats",
        ViewMode::Local => "local",
    }
}

/// Expand a leading `~` in a user-supplied path.
pub fn expand_path(value: &str) -> PathBuf {
    if value == "~" {
        return std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(value));
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

pub fn fmt_duration(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn duration_label(ms: Option<u64>) -> String {
    ms.map(fmt_duration).unwrap_or_else(|| "--:--".to_string())
}

pub fn track_value(track: &Track) -> Value {
    json!({
        "id": track.id,
        "title": track.title,
        "artist": track.artist,
        "source": source_name(track.source),
        "duration_ms": track.duration_ms,
        "duration": track.duration_ms.map(fmt_duration),
        "album": track.album,
        "year": track.year,
        "artwork_url": track.artwork_url,
    })
}

pub fn tracks_value(tracks: &[Track]) -> Value {
    Value::Array(tracks.iter().map(track_value).collect())
}

fn print_json(value: Value) {
    match serde_json::to_string_pretty(&value) {
        Ok(text) => println!("{text}"),
        Err(e) => eprintln!("error: cannot format JSON: {e}"),
    }
}

fn print_track_list(tracks: &[Track], current: Option<usize>) {
    if tracks.is_empty() {
        println!("No tracks.");
        return;
    }
    for (i, track) in tracks.iter().enumerate() {
        let marker = if current == Some(i) { ">" } else { " " };
        println!(
            "{marker} {:>3}. {} — {}  [{}] ({})",
            i + 1,
            track.artist,
            track.title,
            duration_label(track.duration_ms),
            source_name(track.source),
        );
    }
}

pub fn status_message_and_data(app: &App) -> (String, Value) {
    let state = if app.is_playing {
        "playing"
    } else if app.paused_at_ms.is_some() {
        "paused"
    } else {
        "stopped"
    };
    let track = app.playing_track.as_ref().or_else(|| app.current_track());
    let position = app.position_ms();
    let duration = app
        .track_duration_ms
        .or_else(|| track.and_then(|t| t.duration_ms));
    let volume = (app.volume * 100.0).round() as i32;
    let queue_current = if app.tracks.is_empty() {
        0
    } else {
        app.current_index + 1
    };

    let data = json!({
        "state": state,
        "track": track.map(track_value),
        "position_ms": position,
        "duration_ms": duration,
        "volume": volume,
        "shuffle": app.shuffle,
        "repeat": repeat_name(app.repeat_mode),
        "liked": track.map(|_| app.is_current_liked()),
        "queue_index": queue_current,
        "queue_len": app.tracks.len(),
        "source": source_name(app.current_source),
        "view": view_name(&app.view_mode),
    });

    let mut message = String::new();
    match track {
        Some(track) => {
            message.push_str(&format!(
                "{state}: {} — {} [{}]\n",
                track.artist,
                track.title,
                source_name(track.source),
            ));
            match (position, duration) {
                (Some(pos), Some(dur)) => {
                    message.push_str(&format!("{} / {}\n", fmt_duration(pos), fmt_duration(dur)))
                }
                (Some(pos), None) => message.push_str(&format!("{}\n", fmt_duration(pos))),
                _ => {}
            }
        }
        None => message.push_str("stopped: nothing queued\n"),
    }
    message.push_str(&format!(
        "volume {volume}% · shuffle {} · repeat {}{}",
        if app.shuffle { "on" } else { "off" },
        repeat_name(app.repeat_mode),
        if app.is_current_liked() {
            " · liked"
        } else {
            ""
        },
    ));
    if !app.tracks.is_empty() {
        message.push_str(&format!("\nqueue {}/{}", queue_current, app.tracks.len()));
    }
    (message, data)
}

pub fn queue_message_and_data(app: &App, limit: usize) -> (String, Value) {
    let total = app.tracks.len();
    let shown = total.min(limit);
    let entries: Vec<Value> = app
        .tracks
        .iter()
        .take(shown)
        .enumerate()
        .map(|(i, track)| {
            json!({
                "index": i + 1,
                "current": i == app.current_index,
                "track": track_value(track),
            })
        })
        .collect();
    let current = if total == 0 { 0 } else { app.current_index + 1 };
    let data = json!({
        "current": current,
        "total": total,
        "tracks": entries,
    });

    let mut message = format!("queue {current}/{total}");
    if total == 0 {
        message.push_str("\nNo tracks queued.");
    } else {
        for (i, track) in app.tracks.iter().take(shown).enumerate() {
            let marker = if i == app.current_index { ">" } else { " " };
            message.push_str(&format!(
                "\n{marker} {:>3}. {} — {}  [{}] ({})",
                i + 1,
                track.artist,
                track.title,
                duration_label(track.duration_ms),
                source_name(track.source),
            ));
        }
        if shown < total {
            message.push_str(&format!("\n… {} more", total - shown));
        }
    }
    (message, data)
}

pub fn lyrics_message_and_data(app: &App) -> (String, Value) {
    let lines: Vec<Value> = app
        .lyrics_lines
        .iter()
        .map(|(ms, text)| {
            json!({
                "time_ms": ms,
                "timestamp": fmt_duration(*ms),
                "text": text,
            })
        })
        .collect();
    let data = json!({
        "track": app.current_track().map(track_value),
        "source": app.lyrics_source,
        "lines": lines,
    });

    if app.lyrics_lines.is_empty() {
        let message = match &app.lyrics_error {
            Some(e) => format!("lyrics: {e}"),
            None => "No lyrics loaded yet.".to_string(),
        };
        return (message, data);
    }
    let mut message = String::new();
    for (ms, text) in &app.lyrics_lines {
        message.push_str(&format!("[{}] {}\n", fmt_duration(*ms), text));
    }
    (message.trim_end().to_string(), data)
}

// ---------------------------------------------------------------------------
// CLI entry point
// ---------------------------------------------------------------------------

pub async fn run(args: Vec<String>) -> i32 {
    let command = args[0].to_ascii_lowercase();
    if matches!(command.as_str(), "-v" | "-V" | "--version" | "version") {
        println!("larp-music-player {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }

    let flags = match parse_flags(&args[1..]) {
        Ok(flags) => flags,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("Run `larp-music-player --help` for usage.");
            return 2;
        }
    };

    if matches!(command.as_str(), "-h" | "--help" | "help") || flags.help {
        print_help();
        return 0;
    }

    match command.as_str() {
        "search" => run_search(flags).await,
        "liked" => run_liked(flags),
        "playlists" => run_playlists(flags),
        "playlist" => run_playlist(flags),
        "stats" => run_stats(flags),
        "local" => run_local(flags),
        "info" => run_info(flags),
        "config" => run_config(flags),
        "sources" => run_sources(flags),
        other if REMOTE_COMMANDS.contains(&other) => run_remote(other, flags).await,
        other => {
            eprintln!("error: unknown command '{other}'");
            eprintln!("Run `larp-music-player --help` for usage.");
            2
        }
    }
}

fn print_help() {
    println!(
        "larp-music-player {}
A LARP music player. Run without arguments to open the TUI.

USAGE:
    larp-music-player [COMMAND] [OPTIONS]
    larp-music-player                 open the TUI

GENERAL:
    help, -h, --help [COMMAND]        show this help
    version, -V, --version            show the version
    config                            show config/data paths and configured sources
    sources                           list supported sources

PLAYER CONTROL (needs a running TUI):
    status [--json]                   playback state, current track, volume
    play <query|path> [-s SOURCE]     search and play the first match, or play a local file
    pause                             pause playback
    resume                            resume playback
    toggle                            toggle play/pause
    next, prev                        switch track
    stop                              stop playback
    volume [0-100]                    show or set volume
    seek <seconds>                    seek to an absolute position ('+5' / '-5' is relative)
    shuffle [on|off]                  show or set shuffle
    repeat [off|all|one]              show or set repeat mode
    like, unlike                      change the current track's like
    lyrics                            show lyrics (fetches them if needed)
    queue [--limit N] [--json]        list the current queue
    source [SOURCE]                   show or set the active source
    quit                              quit the running player

LIBRARY AND SEARCH (work without a running player):
    search <query> [-s SOURCE] [-n N] [--json]  search online catalogs
    liked [--json]                    list liked tracks
    playlists [--json]                list playlists
    playlist <id> [--json]            list the tracks in a playlist
    stats [--top N] [--by plays|listened|completed] [--json]  listening statistics
    local [--limit N] [--json]        list the offline/downloaded library
    info <file> [--json]              show tags and metadata of an audio file

SOURCES:
    yandex, itunes, soundcloud, ytmusic, local

EXAMPLES:
    larp-music-player status
    larp-music-player play \"Kino - Gruppa Krovi\"
    larp-music-player search \"Molchat Doma\" -s ytmusic -n 5
    larp-music-player volume 40
    larp-music-player stats --top 10
",
        env!("CARGO_PKG_VERSION")
    );
}

// ---------------------------------------------------------------------------
// Local (standalone) commands
// ---------------------------------------------------------------------------

async fn run_search(flags: Flags) -> i32 {
    let query = flags.positional.join(" ");
    if query.trim().is_empty() {
        eprintln!("error: search needs a query");
        return 2;
    }
    let cfg = config::load();
    let source = match flags.source.as_deref() {
        Some(value) => match parse_source(value) {
            Some(source) => source,
            None => {
                eprintln!(
                    "error: unknown source '{value}' (try yandex, itunes, soundcloud, ytmusic)"
                );
                return 2;
            }
        },
        None if !cfg.yandex_token.is_empty() => Source::YandexMusic,
        None => Source::ITunes,
    };
    if source == Source::Local {
        let mut tracks = local::scan_dir(&paths::downloads_dir());
        let needle = query.to_lowercase();
        tracks.retain(|track| {
            track.title.to_lowercase().contains(&needle)
                || track.artist.to_lowercase().contains(&needle)
                || track.id.to_lowercase().contains(&needle)
        });
        if let Some(limit) = flags.limit {
            tracks.truncate(limit);
        }
        return print_search_result(&query, source, &tracks, flags.json);
    }
    if source == Source::YandexMusic && cfg.yandex_token.is_empty() {
        eprintln!("error: Yandex Music needs a token in ~/.config/music-player-tui/config.json");
        return 1;
    }
    if !cfg.soundcloud_client_id.is_empty() {
        api::set_sc_client_id(&cfg.soundcloud_client_id);
    }

    let result = match source {
        Source::YandexMusic => api::search_yandex(&query, &cfg.yandex_token).await,
        Source::ITunes => api::search_itunes(&query).await,
        Source::SoundCloud => api::search_soundcloud(&query).await,
        Source::YouTubeMusic => api::search_ytmusic(&query).await,
        Source::Local => unreachable!(),
    };

    match result {
        Ok(mut tracks) => {
            if let Some(limit) = flags.limit {
                tracks.truncate(limit);
            }
            print_search_result(&query, source, &tracks, flags.json)
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn print_search_result(query: &str, source: Source, tracks: &[Track], json: bool) -> i32 {
    if json {
        print_json(json!({
            "query": query,
            "source": source_name(source),
            "tracks": tracks_value(tracks),
        }));
    } else if tracks.is_empty() {
        println!("No results for '{query}'.");
    } else {
        print_track_list(tracks, None);
    }
    0
}

fn open_db() -> Result<Database, String> {
    Database::open().map_err(|e| format!("cannot open database: {e}"))
}

fn run_liked(flags: Flags) -> i32 {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match db.get_liked() {
        Ok(tracks) => {
            if flags.json {
                print_json(json!({"tracks": tracks_value(&tracks)}));
            } else {
                print_track_list(&tracks, None);
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_playlists(flags: Flags) -> i32 {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    match db.get_playlists() {
        Ok(playlists) => {
            if flags.json {
                print_json(json!({
                    "playlists": playlists.iter().map(|p| json!({
                        "id": p.id,
                        "name": p.name,
                        "tracks": p.count,
                    })).collect::<Vec<_>>(),
                }));
            } else if playlists.is_empty() {
                println!("No playlists.");
            } else {
                for playlist in playlists {
                    println!(
                        "{:>4}. {} ({} tracks)",
                        playlist.id, playlist.name, playlist.count
                    );
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_playlist(flags: Flags) -> i32 {
    let value = match flags.positional.first() {
        Some(value) => value,
        None => {
            eprintln!("error: playlist needs an id");
            return 2;
        }
    };
    let id: i64 = match value.parse() {
        Ok(id) => id,
        Err(_) => {
            eprintln!("error: invalid playlist id '{value}'");
            return 2;
        }
    };
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let name = db
        .get_playlists()
        .ok()
        .and_then(|list| list.into_iter().find(|p| p.id == id))
        .map(|p| p.name);
    match db.get_playlist_tracks(id) {
        Ok(tracks) => {
            if flags.json {
                print_json(json!({
                    "id": id,
                    "name": name,
                    "tracks": tracks_value(&tracks),
                }));
            } else {
                if let Some(name) = &name {
                    println!("{name} ({id})");
                }
                print_track_list(&tracks, None);
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run_stats(flags: Flags) -> i32 {
    let db = match open_db() {
        Ok(db) => db,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let totals = match db.stats_totals() {
        Ok(totals) => totals,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let limit = flags.limit.unwrap_or(10).min(100);
    let by = flags.by.unwrap_or(TopBy::Plays);
    let top = match db.top_tracks(by, limit) {
        Ok(top) => top,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let daily = match db.daily_stats(7) {
        Ok(daily) => daily,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    if flags.json {
        print_json(json!({
            "totals": {
                "listened_ms": totals.listened_ms,
                "listened": db::fmt_listened(totals.listened_ms),
                "plays": totals.plays,
                "tracks": totals.tracks,
                "completed": totals.completed,
                "days": totals.days,
                "first_day": totals.first_day,
            },
            "top_by": top_by_name(by),
            "top_tracks": top.iter().map(|t| json!({
                "source": source_name(t.source),
                "id": t.track_id,
                "title": t.title,
                "artist": t.artist,
                "plays": t.play_count,
                "completed": t.completed_count,
                "skips": t.skip_count,
                "listened_ms": t.listened_ms,
                "listened": db::fmt_listened(t.listened_ms),
            })).collect::<Vec<_>>(),
            "daily": daily.iter().map(|d| json!({
                "day": d.day,
                "listened_ms": d.listened_ms,
                "listened": db::fmt_listened(d.listened_ms),
                "plays": d.plays,
            })).collect::<Vec<_>>(),
        }));
    } else {
        println!("Listened: {}", db::fmt_listened(totals.listened_ms));
        println!(
            "Plays: {} · Tracks: {} · Completed: {} · Days: {}",
            totals.plays, totals.tracks, totals.completed, totals.days
        );
        if let Some(first) = &totals.first_day {
            println!("First listen: {first}");
        }
        println!("\nTop tracks by {}:", top_by_name(by));
        if top.is_empty() {
            println!("  No listening history yet.");
        } else {
            for (i, stat) in top.iter().enumerate() {
                println!(
                    "{:>3}. {} — {}  · {} plays · {}",
                    i + 1,
                    stat.artist,
                    stat.title,
                    stat.play_count,
                    db::fmt_listened(stat.listened_ms)
                );
            }
        }
        println!("\nLast 7 days:");
        if daily.is_empty() {
            println!("  No listening history yet.");
        } else {
            for day in &daily {
                println!(
                    "  {}  {:>4} plays  {}",
                    day.day,
                    day.plays,
                    db::fmt_listened(day.listened_ms)
                );
            }
        }
    }
    0
}

fn run_local(flags: Flags) -> i32 {
    let dir = paths::downloads_dir();
    let mut tracks = local::scan_dir(&dir);
    let total = tracks.len();
    if let Some(limit) = flags.limit {
        tracks.truncate(limit);
    }
    if flags.json {
        print_json(json!({
            "dir": dir,
            "total": total,
            "tracks": tracks_value(&tracks),
        }));
    } else {
        println!("{} ({} tracks)", dir.display(), total);
        print_track_list(&tracks, None);
    }
    0
}

fn run_info(flags: Flags) -> i32 {
    let value = match flags.positional.first() {
        Some(value) => value,
        None => {
            eprintln!("error: info needs a file path");
            return 2;
        }
    };
    let path = expand_path(value);
    if !path.is_file() {
        eprintln!("error: file not found: {}", path.display());
        return 1;
    }
    match local::track_for(&path) {
        Some(track) => {
            if flags.json {
                print_json(track_value(&track));
            } else {
                println!("Title:  {}", track.title);
                println!("Artist: {}", track.artist);
                println!("Album:  {}", track.album.as_deref().unwrap_or("-"));
                println!(
                    "Year:   {}",
                    track
                        .year
                        .map(|y| y.to_string())
                        .unwrap_or_else(|| "-".into())
                );
                println!("Length: {}", duration_label(track.duration_ms));
                println!("Source: {}", source_name(track.source));
                println!("Path:   {}", track.id);
            }
            0
        }
        None => {
            eprintln!("error: cannot read metadata from {}", path.display());
            1
        }
    }
}

fn run_config(flags: Flags) -> i32 {
    let cfg = config::load();
    let db_path = paths::config_dir().join("liked.db");
    if flags.json {
        print_json(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "config": paths::config_dir().join("config.json"),
            "data": paths::data_dir(),
            "downloads": paths::downloads_dir(),
            "database": db_path,
            "yandex_token": !cfg.yandex_token.is_empty(),
            "soundcloud_client_id": !cfg.soundcloud_client_id.is_empty(),
            "lastfm_api_key": !cfg.lastfm_api_key.is_empty(),
        }));
    } else {
        println!("Version:   {}", env!("CARGO_PKG_VERSION"));
        println!(
            "Config:    {}",
            paths::config_dir().join("config.json").display()
        );
        println!("Data:      {}", paths::data_dir().display());
        println!("Downloads: {}", paths::downloads_dir().display());
        println!("Database:  {}", db_path.display());
        println!(
            "Yandex token:          {}",
            if cfg.yandex_token.is_empty() {
                "not set"
            } else {
                "set"
            }
        );
        println!(
            "SoundCloud client_id:  {}",
            if cfg.soundcloud_client_id.is_empty() {
                "auto-extract"
            } else {
                "set"
            }
        );
        println!(
            "Last.fm API key:       {}",
            if cfg.lastfm_api_key.is_empty() {
                "not set"
            } else {
                "set"
            }
        );
        println!("Socket:    {}", socket_path().display());
    }
    0
}

fn run_sources(flags: Flags) -> i32 {
    let cfg = config::load();
    let entries = vec![
        json!({
            "name": "yandex",
            "label": "Yandex Music",
            "ready": !cfg.yandex_token.is_empty(),
            "detail": if cfg.yandex_token.is_empty() { "token not set" } else { "token set" },
        }),
        json!({
            "name": "itunes",
            "label": "iTunes",
            "ready": true,
            "detail": "no configuration needed",
        }),
        json!({
            "name": "soundcloud",
            "label": "SoundCloud",
            "ready": true,
            "detail": if cfg.soundcloud_client_id.is_empty() { "client_id auto-extracted" } else { "client_id set" },
        }),
        json!({
            "name": "ytmusic",
            "label": "YouTube Music",
            "ready": true,
            "detail": "uses yt-dlp for playback",
        }),
        json!({
            "name": "local",
            "label": "Local files",
            "ready": true,
            "detail": paths::downloads_dir(),
        }),
    ];
    if flags.json {
        print_json(json!({"sources": entries}));
    } else {
        for entry in &entries {
            let ready = if entry["ready"].as_bool().unwrap_or(false) {
                "ok"
            } else {
                "needs setup"
            };
            println!(
                "{:<10} {:<14} {:<12} {}",
                entry["name"].as_str().unwrap_or(""),
                entry["label"].as_str().unwrap_or(""),
                ready,
                entry["detail"].as_str().unwrap_or("")
            );
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Remote commands
// ---------------------------------------------------------------------------

async fn run_remote(command: &str, flags: Flags) -> i32 {
    let request = CliRequest {
        command: command.to_string(),
        args: flags.positional,
        source: flags.source,
        limit: flags.limit,
        json: flags.json,
    };
    match send_request(&request).await {
        Ok(response) => {
            if flags.json {
                let value = response.data.clone().unwrap_or_else(|| {
                    json!({
                        "ok": response.ok,
                        "message": response.message,
                    })
                });
                print_json(value);
            } else if let Some(message) = &response.message {
                if response.ok {
                    println!("{message}");
                } else {
                    eprintln!("{message}");
                }
            }
            if response.ok {
                0
            } else {
                1
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!("Start the player with `larp-music-player` first.");
            eprintln!(
                "Library commands such as `search`, `liked`, `stats` and `local` work without it."
            );
            1
        }
    }
}

async fn send_request(request: &CliRequest) -> Result<CliResponse, String> {
    let path = socket_path();
    let stream = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::UnixStream::connect(&path),
    )
    .await
    .map_err(|_| format!("timed out connecting to {}", path.display()))?
    .map_err(|e| format!("{e} ({})", path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut line = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    write_half
        .write_all(&line)
        .await
        .map_err(|e| format!("cannot send request: {e}"))?;
    let _ = write_half.flush().await;

    let mut reader = BufReader::new(read_half);
    let mut response = String::new();
    tokio::time::timeout(Duration::from_secs(15), reader.read_line(&mut response))
        .await
        .map_err(|_| "timed out waiting for the player".to_string())?
        .map_err(|e| format!("cannot read response: {e}"))?;
    if response.trim().is_empty() {
        return Err("the player closed the connection without a response".to_string());
    }
    serde_json::from_str(response.trim()).map_err(|e| format!("invalid response: {e}"))
}

// ---------------------------------------------------------------------------
// Control socket
// ---------------------------------------------------------------------------

/// `$XDG_RUNTIME_DIR/larp-music-player.sock`, falling back to the config dir.
pub fn socket_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") {
        let path = PathBuf::from(dir).join("larp-music-player.sock");
        if path.as_os_str().len() < 100 {
            return path;
        }
    }
    paths::config_dir().join("player.sock")
}

pub struct SocketGuard {
    path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Start listening for CLI requests. Returns `None` when another instance is
/// already running (the existing socket stays untouched).
pub fn start_server(cmd_tx: mpsc::UnboundedSender<AppCommand>) -> Option<SocketGuard> {
    let path = socket_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    if std::os::unix::net::UnixStream::connect(&path).is_ok() {
        crate::dlog!(
            "[cli] another instance is already listening on {}",
            path.display()
        );
        return None;
    }
    let _ = std::fs::remove_file(&path);

    let listener = match std::os::unix::net::UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(e) => {
            crate::dlog!("[cli] cannot bind {}: {}", path.display(), e);
            return None;
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    if let Err(e) = listener.set_nonblocking(true) {
        crate::dlog!("[cli] cannot make socket non-blocking: {}", e);
        return None;
    }
    let listener = match tokio::net::UnixListener::from_std(listener) {
        Ok(listener) => listener,
        Err(e) => {
            crate::dlog!("[cli] cannot register socket: {}", e);
            return None;
        }
    };

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = cmd_tx.clone();
                    tokio::spawn(async move {
                        serve_connection(stream, tx).await;
                    });
                }
                Err(e) => {
                    crate::dlog!("[cli] accept error: {}", e);
                    break;
                }
            }
        }
    });

    Some(SocketGuard { path })
}

async fn serve_connection(
    mut stream: tokio::net::UnixStream,
    cmd_tx: mpsc::UnboundedSender<AppCommand>,
) {
    let parsed = {
        let mut reader = BufReader::new(&mut stream);
        let mut line = String::new();
        match tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line)).await {
            Ok(Ok(_)) if !line.trim().is_empty() => serde_json::from_str::<CliRequest>(line.trim())
                .map_err(|e| format!("invalid request: {e}")),
            Ok(Ok(_)) => Err("empty request".to_string()),
            Ok(Err(e)) => Err(format!("read error: {e}")),
            Err(_) => Err("timed out reading request".to_string()),
        }
    };

    let request = match parsed {
        Ok(request) => request,
        Err(e) => {
            let _ = write_response(&mut stream, CliResponse::err(e)).await;
            return;
        }
    };

    let (reply_tx, reply_rx) = oneshot::channel();
    if cmd_tx
        .send(AppCommand::Cli {
            request,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = write_response(&mut stream, CliResponse::err("the player is shutting down")).await;
        return;
    }

    let response = match tokio::time::timeout(Duration::from_secs(15), reply_rx).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => CliResponse::err("the player dropped the request"),
        Err(_) => CliResponse::err("the player did not respond in time"),
    };
    let _ = write_response(&mut stream, response).await;
}

async fn write_response(
    stream: &mut tokio::net::UnixStream,
    response: CliResponse,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(&response)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    stream.write_all(&line).await?;
    stream.flush().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn source_aliases() {
        assert_eq!(parse_source("yt"), Some(Source::YouTubeMusic));
        assert_eq!(parse_source("youtube-music"), Some(Source::YouTubeMusic));
        assert_eq!(parse_source("SC"), Some(Source::SoundCloud));
        assert_eq!(parse_source("file"), Some(Source::Local));
        assert_eq!(parse_source("nope"), None);
    }

    #[test]
    fn flags_parse_values() {
        let flags = parse_flags(&args(&[
            "hello world",
            "-s",
            "ytmusic",
            "-n",
            "5",
            "--json",
        ]))
        .unwrap();
        assert_eq!(flags.positional, vec!["hello world"]);
        assert_eq!(flags.source.as_deref(), Some("ytmusic"));
        assert_eq!(flags.limit, Some(5));
        assert!(flags.json);
    }

    #[test]
    fn flags_allow_negative_numbers() {
        let flags = parse_flags(&args(&["-5"])).unwrap();
        assert_eq!(flags.positional, vec!["-5"]);
    }

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(0), "0:00");
        assert_eq!(fmt_duration(65_000), "1:05");
        assert_eq!(fmt_duration(3_665_000), "1:01:05");
    }
}
