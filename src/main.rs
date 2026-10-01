mod api;
mod app;
mod config;
mod db;
mod discord_rpc;
mod log;
mod mpris;
mod player;
mod tags;
mod ui;
mod update;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use tokio::sync::mpsc;

use app::App;
use app::InputMode;
use app::PlayAction;

fn discord_set(discord: &Option<discord_rpc::DiscordRpc>, app: &App) {
    if let Some(ref d) = discord {
        if let Some(track) = app.current_track() {
            let source_name = match track.source {
                api::Source::YandexMusic => "Yandex Music",
                api::Source::SoundCloud => "SoundCloud",
                api::Source::ITunes => "iTunes",
                api::Source::YouTubeMusic => "YouTube Music",
            };
            d.set_activity(
                &track.title,
                &track.artist,
                source_name,
                track.artwork_url.as_deref(),
                track.duration_ms,
                "Playing",
            );
        }
    }
}

fn discord_state(
    discord: &Option<discord_rpc::DiscordRpc>,
    state: &str,
    position_ms: Option<u64>,
    show_timer: bool,
) {
    if let Some(ref d) = discord {
        d.set_state(state, position_ms, show_timer);
    }
}

fn save_all_config(app: &App, cfg: &config::Config) {
    config::save(&config::Config {
        yandex_token: app.token.clone(),
        soundcloud_client_id: app.sc_client_id.clone(),
        discord_client_id: cfg.discord_client_id.clone(),
        lastfm_api_key: cfg.lastfm_api_key.clone(),
        liked_shuffle: app.liked_shuffle,
        last_update_check: cfg.last_update_check,
    });
}

fn discord_resume(discord: &Option<discord_rpc::DiscordRpc>, app: &App) {
    discord_state(discord, "Playing", app.position_ms(), true);
}

fn downloads_dir() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".cache")
        .join("music-player-tui")
        .join("downloads")
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}

fn lower_char(c: char) -> char {
    match c {
        'A'..='Z' => c.to_ascii_lowercase(),
        'А' => 'а',
        'Б' => 'б',
        'В' => 'в',
        'Г' => 'г',
        'Д' => 'д',
        'Е' => 'е',
        'Ё' => 'ё',
        'Ж' => 'ж',
        'З' => 'з',
        'И' => 'и',
        'Й' => 'й',
        'К' => 'к',
        'Л' => 'л',
        'М' => 'м',
        'Н' => 'н',
        'О' => 'о',
        'П' => 'п',
        'Р' => 'р',
        'С' => 'с',
        'Т' => 'т',
        'У' => 'у',
        'Ф' => 'ф',
        'Х' => 'х',
        'Ц' => 'ц',
        'Ч' => 'ч',
        'Ш' => 'ш',
        'Щ' => 'щ',
        'Ъ' => 'ъ',
        'Ы' => 'ы',
        'Ь' => 'ь',
        'Э' => 'э',
        'Ю' => 'ю',
        'Я' => 'я',
        _ => c,
    }
}

fn check_bluetooth_device() -> Option<String> {
    let out = std::process::Command::new("bluetoothctl")
        .args(["devices", "Connected"])
        .output();
    match out {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let line = text.lines().next()?;
            let mut parts = line.split_whitespace();
            if parts.next()? != "Device" {
                return None;
            }
            parts.next()?;
            let name = parts.collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                None
            } else {
                Some(name)
            }
        }
        Err(_) => None,
    }
}

fn local_liked_path(app: &App) -> Option<std::path::PathBuf> {
    let track = app.current_track()?;
    if !app.db.is_liked(&track.source, &track.id).unwrap_or(false) {
        return None;
    }
    let filename = sanitize_filename(&format!("{} - {}.mp3", track.artist, track.title));
    let path = downloads_dir().join(&filename);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn start_update_check(cfg: &config::Config, cmd_tx: &mpsc::UnboundedSender<app::Command>) {
    let now = config::now_secs();
    // Check at most once every 24h.
    if now.saturating_sub(cfg.last_update_check) < 24 * 60 * 60 {
        return;
    }
    let mut fresh = config::Config {
        yandex_token: String::new(),
        soundcloud_client_id: String::new(),
        discord_client_id: cfg.discord_client_id.clone(),
        lastfm_api_key: cfg.lastfm_api_key.clone(),
        liked_shuffle: cfg.liked_shuffle,
        last_update_check: now,
    };
    let saved = cmd_tx.clone();
    tokio::spawn(async move {
        if let Some((tag, url)) = update::check().await {
            let _ = saved.send(app::Command::UpdateAvailable { tag, url });
        }
    });
    config::save(&fresh);
    fresh.last_update_check = now;
}

fn start_download_liked(app: &App, cmd_tx: &mpsc::UnboundedSender<app::Command>, silent: bool) {
    let liked = match app.db.get_liked() {
        Ok(t) => t,
        Err(_) => return,
    };
    if liked.is_empty() {
        return;
    }
    let tx = cmd_tx.clone();
    let token = app.token.clone();
    tokio::spawn(async move {
        let _ = tokio::fs::create_dir_all(downloads_dir()).await;
        let total = liked.len();
        let mut downloaded = 0u32;
        let mut failed = 0u32;
        for (i, track) in liked.iter().enumerate() {
            if !silent {
                let _ = tx.send(app::Command::DownloadLikedProgress {
                    current: i + 1,
                    total,
                });
            }
            let filename = sanitize_filename(&format!("{} - {}.mp3", track.artist, track.title));
            let dest = downloads_dir().join(&filename);
            if dest.exists() {
                downloaded += 1;
                continue;
            }
            let result = match track.source {
                api::Source::ITunes => {
                    if let Some(ref url) = track.preview_url {
                        download_file(url, &dest).await
                    } else {
                        Err("no preview url".into())
                    }
                }
                api::Source::SoundCloud => download_soundcloud(&track.id, &dest).await,
                api::Source::YouTubeMusic => download_ytmusic(&track.id, &dest).await,
                api::Source::YandexMusic => download_yandex(&track.id, &token, &dest).await,
            };
            match result {
                Ok(()) => {
                    tag_downloaded(&track, &dest).await;
                    downloaded += 1;
                }
                Err(_) => failed += 1,
            }
        }
        let _ = tx.send(app::Command::DownloadLikedDone {
            downloaded: downloaded as usize,
            failed: failed as usize,
        });
    });
}

fn meta_cred(source: api::Source, app: &App) -> Option<String> {
    match source {
        api::Source::YandexMusic => {
            let t = app.token.split('&').next().unwrap_or("").to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        }
        api::Source::SoundCloud => {
            let c = app.sc_client_id.clone();
            if c.is_empty() {
                None
            } else {
                Some(c)
            }
        }
        _ => None,
    }
}

fn artwork_is_big(source: api::Source, url: Option<&str>) -> bool {
    let u = match url {
        Some(u) => u,
        None => return false,
    };
    match source {
        api::Source::YouTubeMusic => u.contains("w1080") || u.contains("hq720") || !u.contains("w"),
        api::Source::ITunes => u.contains("600x600bb"),
        api::Source::SoundCloud => u.contains("t500x500"),
        api::Source::YandexMusic => u.contains("1000x1000"),
    }
}

fn start_liked_backfill(app: &App, cmd_tx: &mpsc::UnboundedSender<app::Command>) {
    let liked = match app.db.get_liked() {
        Ok(t) => t,
        Err(_) => return,
    };
    for track in liked {
        if track.duration_ms.is_some() && artwork_is_big(track.source, track.artwork_url.as_deref())
        {
            continue;
        }
        fetch_meta_into(cmd_tx, track, app);
    }
}

fn fetch_meta_into(cmd_tx: &mpsc::UnboundedSender<app::Command>, track: api::Track, app: &App) {
    let tx = cmd_tx.clone();
    let source = track.source;
    let id = track.id.clone();
    let have_art = track.artwork_url.clone();
    let duration = track.duration_ms;
    let cred = meta_cred(source, app);
    tokio::spawn(async move {
        if let Some(meta) = api::get_meta(source, &id, cred.as_deref()).await {
            let duration = duration.or(meta.duration_ms);
            let artwork = match have_art {
                Some(u) if artwork_is_big(source, Some(&u)) => Some(u),
                _ => meta.artwork_url,
            };
            let _ = tx.send(app::Command::MetaUpdated {
                source,
                track_id: id,
                duration_ms: duration,
                artwork_url: artwork,
            });
        }
    });
}

async fn download_file(url: &str, dest: &std::path::Path) -> Result<(), String> {
    let url = url.to_string();
    let dest = dest.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("curl")
            .args(["-sL", "-o", dest.to_str().unwrap_or("")])
            .arg(&url)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!("curl failed"))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn download_soundcloud(track_id: &str, dest: &std::path::Path) -> Result<(), String> {
    let track_url = format!("https://api.soundcloud.com/tracks/{}", track_id);
    let dest_path = dest.with_extension("").to_str().unwrap_or("").to_string();
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("yt-dlp")
            .args([
                "--no-playlist",
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "128K",
                "-f",
                "bestaudio",
            ])
            .args(["--force-overwrites"])
            .args(["-o", &format!("{}.%(ext)s", dest_path)])
            .arg(&track_url)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            Err(format!("yt-dlp failed: {}", output.status))
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn download_ytmusic(track_id: &str, dest: &std::path::Path) -> Result<(), String> {
    let track_url = format!("https://music.youtube.com/watch?v={}", track_id);
    let dest_path = dest.with_extension("").to_str().unwrap_or("").to_string();
    tokio::task::spawn_blocking(move || {
        let output = std::process::Command::new("yt-dlp")
            .args([
                "--no-playlist",
                "-x",
                "--audio-format",
                "mp3",
                "--audio-quality",
                "128K",
            ])
            .args(["--extractor-args", "youtube:player_client=mweb"])
            .args(["--force-overwrites"])
            .args(["-o", &format!("{}.%(ext)s", dest_path)])
            .arg(&track_url)
            .output()
            .map_err(|e| e.to_string())?;
        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(stderr
                .lines()
                .rev()
                .find(|l| l.starts_with("ERROR:"))
                .unwrap_or("unknown error")
                .to_string())
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

async fn download_yandex(
    track_id: &str,
    token: &str,
    dest: &std::path::Path,
) -> Result<(), String> {
    let url = api::get_yandex_download_url(track_id, token)
        .await
        .map_err(|e| e.to_string())?;
    download_file(&url, dest).await
}

fn start_radio(app: &App, cmd_tx: &mpsc::UnboundedSender<app::Command>, api_key: String) {
    let track = match app.current_track() {
        Some(t) => t.clone(),
        None => {
            return;
        }
    };
    let source = app.current_source;
    let token = app.token.clone();
    let tx = cmd_tx.clone();

    tokio::spawn(async move {
        let _ = tx.send(app::Command::Status(format!(
            "Radio: {} — {} → Last.fm...",
            track.artist, track.title
        )));
        let similar = match api::lastfm_similar(&api_key, &track.artist, &track.title).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx.send(app::Command::Status(format!("Last.fm error: {}", e)));
                return;
            }
        };

        let mut found: Vec<api::Track> = Vec::new();
        for (artist, title) in similar.into_iter().take(15) {
            if found.len() >= 20 {
                break;
            }
            let query = format!("{} {}", title, artist);
            let result = match source {
                api::Source::YandexMusic => api::search_yandex(&query, &token).await,
                api::Source::ITunes => api::search_itunes(&query).await,
                api::Source::SoundCloud => api::search_soundcloud(&query).await,
                api::Source::YouTubeMusic => api::search_ytmusic(&query).await,
            };
            if let Ok(tracks) = result {
                if let Some(t) = tracks.into_iter().next() {
                    found.push(t);
                }
            }
        }

        let _ = tx.send(app::Command::RadioFound(found));
    });
}

async fn tag_downloaded(track: &api::Track, dest: &std::path::Path) {
    let artist = track.artist.clone();
    let title = track.title.clone();
    let meta = api::get_mb_metadata(&artist, &title).await.ok().flatten();
    tags::tag_track_file(
        dest,
        track,
        meta.as_ref().and_then(|m| m.album.as_deref()),
        meta.as_ref().and_then(|m| m.year),
    );
}

fn track_idx_at(app: &App, track_area: &Rect, y: u16) -> Option<usize> {
    let inner_y = y.saturating_sub(track_area.y + 1);
    if inner_y < 1 {
        return None;
    }
    let item_height: u16 = 3;
    let item_idx = ((inner_y - 1) / item_height) as usize;
    let visible_items = ((track_area.height.saturating_sub(2)) / item_height) as usize;
    let max_scroll = app.tracks.len().saturating_sub(visible_items);
    let scroll = app
        .current_index
        .saturating_sub(visible_items / 2)
        .min(max_scroll);
    let track_idx = scroll + item_idx;
    if track_idx < app.tracks.len() {
        Some(track_idx)
    } else {
        None
    }
}

fn play_track(
    player_tx: &mpsc::UnboundedSender<player::PlayerCommand>,
    app: &App,
    cmd_tx: &mpsc::UnboundedSender<app::Command>,
) {
    if let Some(path) = local_liked_path(app) {
        let gen = player::next_generation();
        let _ = player_tx.send(player::PlayerCommand::PlayFile(
            path.to_string_lossy().into_owned(),
            app.volume,
            gen,
        ));
        if let Some(t) = app.current_track() {
            let _ = cmd_tx.send(app::Command::Status(format!(
                "Playing (offline): {}",
                t.title
            )));
        }
        return;
    }
    if let Some((id_or_url, source)) = app.get_play_url() {
        let gen = player::next_generation();
        match source {
            api::Source::ITunes => {
                let _ = player_tx.send(player::PlayerCommand::PlayFile(id_or_url, app.volume, gen));
            }
            api::Source::SoundCloud => {
                let tx = player_tx.clone();
                let vol = app.volume;
                let cmd_tx2 = cmd_tx.clone();
                let track_id = app.get_current_track_id().unwrap_or_default();
                tokio::spawn(async move {
                    let _ = cmd_tx2.send(app::Command::DownloadProgress(0));
                    let tmp_path = format!("/tmp/sc_{}.mp3", track_id);
                    let result = tokio::task::spawn_blocking({
                        let tmp_path = tmp_path.clone();
                        move || {
                            std::process::Command::new("yt-dlp")
                                .args([
                                    "--no-playlist",
                                    "-x",
                                    "--audio-format",
                                    "mp3",
                                    "--audio-quality",
                                    "128K",
                                    "-f",
                                    "bestaudio",
                                ])
                                .args(["--force-overwrites"])
                                .args(["-o", &tmp_path])
                                .arg(format!("https://api.soundcloud.com/tracks/{}", track_id))
                                .output()
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok(output)) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(100));
                            if output.status.success() {
                                let _ =
                                    tx.send(player::PlayerCommand::PlayFile(tmp_path, vol, gen));
                            } else {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                let msg = if stderr.contains("DRM protected") {
                                    "DRM protected — SoundCloud Go required".to_string()
                                } else {
                                    format!("yt-dlp failed: {}", output.status)
                                };
                                let _ = cmd_tx2.send(app::Command::PlayError(msg));
                            }
                        }
                        Ok(Err(e)) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(101));
                            let _ = cmd_tx2
                                .send(app::Command::PlayError(format!("yt-dlp error: {}", e)));
                        }
                        Err(e) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(101));
                            let _ = cmd_tx2
                                .send(app::Command::PlayError(format!("spawn error: {}", e)));
                        }
                    }
                });
            }
            api::Source::YandexMusic => {
                let token = app.token.clone();
                let tx = player_tx.clone();
                let vol = app.volume;
                let cmd_tx2 = cmd_tx.clone();
                tokio::spawn(async move {
                    match api::get_yandex_download_url(&id_or_url, &token).await {
                        Ok(url) => {
                            let _ = tx.send(player::PlayerCommand::PlayFile(url, vol, gen));
                        }
                        Err(e) => {
                            let _ = cmd_tx2.send(app::Command::PlayError(e.to_string()));
                        }
                    }
                });
            }
            api::Source::YouTubeMusic => {
                let tx = player_tx.clone();
                let vol = app.volume;
                let cmd_tx2 = cmd_tx.clone();
                let track_id = app.get_current_track_id().unwrap_or_default();
                tokio::spawn(async move {
                    let _ = cmd_tx2.send(app::Command::DownloadProgress(0));
                    let tmp_path = format!("/tmp/ytm_{}.mp3", track_id);
                    let result = tokio::task::spawn_blocking({
                        let tmp_path = tmp_path.clone();
                        move || {
                            std::process::Command::new("yt-dlp")
                                .args([
                                    "--no-playlist",
                                    "-x",
                                    "--audio-format",
                                    "mp3",
                                    "--audio-quality",
                                    "128K",
                                ])
                                .args(["--extractor-args", "youtube:player_client=mweb"])
                                .args(["--force-overwrites"])
                                .args(["-o", &tmp_path])
                                .arg(format!("https://music.youtube.com/watch?v={}", track_id))
                                .output()
                        }
                    })
                    .await;

                    match result {
                        Ok(Ok(output)) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(100));
                            if output.status.success() {
                                let _ =
                                    tx.send(player::PlayerCommand::PlayFile(tmp_path, vol, gen));
                            } else {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                let msg = if stderr.contains("DRM protected") {
                                    "DRM protected — not available".to_string()
                                } else {
                                    let short: String = stderr
                                        .lines()
                                        .rev()
                                        .find(|l| l.starts_with("ERROR:"))
                                        .unwrap_or("unknown error")
                                        .to_string();
                                    format!("yt-dlp: {}", short)
                                };
                                let _ = cmd_tx2.send(app::Command::PlayError(msg));
                            }
                        }
                        Ok(Err(e)) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(101));
                            let _ = cmd_tx2
                                .send(app::Command::PlayError(format!("yt-dlp error: {}", e)));
                        }
                        Err(e) => {
                            let _ = cmd_tx2.send(app::Command::DownloadProgress(101));
                            let _ = cmd_tx2
                                .send(app::Command::PlayError(format!("spawn error: {}", e)));
                        }
                    }
                });
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    struct TermGuard;
    impl Drop for TermGuard {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
            let mut stdout = io::stdout();
            let _ = execute!(
                stdout,
                LeaveAlternateScreen,
                crossterm::event::DisableMouseCapture
            );
        }
    }
    let _guard = TermGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<app::Command>();

    let cfg = config::load();
    if !cfg.soundcloud_client_id.is_empty() {
        api::set_sc_client_id(&cfg.soundcloud_client_id);
    }
    let mut app = App::new(
        cmd_tx.clone(),
        Some(cfg.yandex_token.clone()),
        Some(cfg.soundcloud_client_id.clone()),
        cfg.liked_shuffle,
    );
    start_download_liked(&app, &cmd_tx, true);
    start_liked_backfill(&app, &cmd_tx);
    start_update_check(&cfg, &cmd_tx);
    let (player_tx, player_handle) = player::start(cmd_tx.clone());

    {
        let bt_tx = cmd_tx.clone();
        std::thread::spawn(move || {
            let mut last: Option<String> = None;
            loop {
                let device = check_bluetooth_device();
                if device != last {
                    last = device.clone();
                    let _ = bt_tx.send(app::Command::BluetoothStatus(device));
                }
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
    }

    let (mpris_tx, mut mpris_rx) = mpsc::unbounded_channel::<mpris::MprisCommand>();
    let mpris = match mpris::MprisServer::new(mpris_tx).await {
        Ok(server) => Some(server),
        Err(e) => {
            dlog!("MPRIS init failed: {}", e);
            None
        }
    };

    let discord_id = if cfg.discord_client_id.is_empty() {
        config::DEFAULT_DISCORD_CLIENT_ID
    } else {
        &cfg.discord_client_id
    };
    let discord = discord_rpc::DiscordRpc::new(discord_id);

    let mut last_progress = std::time::Instant::now();
    loop {
        app.maybe_fetch_lyrics();
        let mut redraw = false;

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    redraw = true;
                    if key.kind == KeyEventKind::Press {
                        if key.code == KeyCode::Char('c')
                            && key.modifiers.contains(KeyModifiers::CONTROL)
                        {
                            break;
                        }
                        match app.input_mode {
                            InputMode::Search
                            | InputMode::TokenInput
                            | InputMode::ClientIdInput
                            | InputMode::NameInput => match key.code {
                                KeyCode::Esc => app.cancel_input(),
                                KeyCode::Enter => {
                                    let was_token_input = app.input_mode == InputMode::TokenInput;
                                    let was_sc_input = app.input_mode == InputMode::ClientIdInput;
                                    app.submit();
                                    if was_token_input && !app.token.is_empty() {
                                        save_all_config(&app, &cfg);
                                    }
                                    if was_sc_input && !app.sc_client_id.is_empty() {
                                        api::set_sc_client_id(&app.sc_client_id);
                                        save_all_config(&app, &cfg);
                                        app.status_message = "Switched to SoundCloud".to_string();
                                    }
                                }
                                KeyCode::Backspace => app.input_backspace(),
                                KeyCode::Char(c) if !c.is_control() => app.input_char(c),
                                _ => {}
                            },
                            InputMode::SourceSelect
                            | InputMode::ViewSelect
                            | InputMode::PlaylistSelect => match key.code {
                                KeyCode::Esc => app.cancel_input(),
                                KeyCode::Enter => app.submit(),
                                KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                    app.select_up()
                                }
                                KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                    app.select_down()
                                }
                                _ => {}
                            },
                            InputMode::Help => match key.code {
                                KeyCode::Esc
                                | KeyCode::Enter
                                | KeyCode::Char('h')
                                | KeyCode::Char('H') => app.toggle_help(),
                                _ => {}
                            },
                            InputMode::Normal => {
                                let shifted = key.modifiers.contains(KeyModifiers::SHIFT);
                                if shifted {
                                    if let KeyCode::Char(c) = key.code {
                                        match lower_char(c) {
                                            'h' | 'р' => {
                                                app.toggle_help();
                                            }
                                            'p' | 'з' => {
                                                app.enter_playlists();
                                            }
                                            'a' | 'ф' => {
                                                app.start_playlist_add();
                                            }
                                            'n' | 'т' => {
                                                app.start_name_input();
                                            }
                                            'd' | 'в' => {
                                                if app.view_mode == app::ViewMode::Playlists {
                                                    if app.active_playlist_id.is_none() {
                                                        app.delete_selected_playlist();
                                                    } else {
                                                        app.remove_current_from_playlist();
                                                    }
                                                } else {
                                                    app.start_playlist_add();
                                                }
                                            }
                                            _ => {}
                                        }
                                    }
                                } else {
                                    match key.code {
                                        KeyCode::Char('q') | KeyCode::Char('й') => break,
                                        KeyCode::Char('u') | KeyCode::Char('г') => {
                                            // open pending update, else check now
                                            if let Some(url) = app.take_update_url() {
                                                update::open_in_browser(&url);
                                                app.status_message =
                                                    "Opened release page in browser".to_string();
                                            } else {
                                                let tx = cmd_tx.clone();
                                                app.status_message =
                                                    "Checking for updates...".to_string();
                                                tokio::spawn(async move {
                                                    match update::check().await {
                                                        Some((tag, url)) => {
                                                            let _ = tx.send(
                                                                app::Command::UpdateAvailable {
                                                                    tag,
                                                                    url,
                                                                },
                                                            );
                                                        }
                                                        None => {
                                                            let _ = tx.send(app::Command::Status(
                                                                format!(
                                                                    "Up to date ({})",
                                                                    update::current_version()
                                                                ),
                                                            ));
                                                        }
                                                    }
                                                });
                                            }
                                        }
                                        KeyCode::Char('/')
                                        | KeyCode::Char('ы')
                                        | KeyCode::Char('щ')
                                        | KeyCode::Char('s') => app.focus_search(),
                                        KeyCode::Char('t') | KeyCode::Char('е') => {
                                            app.start_source_select()
                                        }
                                        KeyCode::Char('l') | KeyCode::Char('д') => {
                                            app.toggle_like()
                                        }
                                        KeyCode::Tab => app.start_view_select(),
                                        KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('л') => {
                                            app.prev()
                                        }
                                        KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('о') => {
                                            app.next()
                                        }
                                        KeyCode::Char(' ') => match app.toggle_play() {
                                            PlayAction::Pause => {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Pause);
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(false).await;
                                                }
                                                discord_state(&discord, "Paused", None, false);
                                            }
                                            PlayAction::Resume => {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Resume);
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(true).await;
                                                }
                                                discord_resume(&discord, &app);
                                            }
                                            PlayAction::NewTrack => {
                                                if let Some(ref m) = mpris {
                                                    m.update_track(app.current_track().unwrap())
                                                        .await;
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                                play_track(&player_tx, &app, &cmd_tx);
                                            }
                                        },
                                        KeyCode::Pause => {
                                            if app.is_playing {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Pause);
                                                app.pause();
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(false).await;
                                                }
                                                discord_state(&discord, "Paused", None, false);
                                            } else {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Resume);
                                                app.resume();
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(true).await;
                                                }
                                                discord_resume(&discord, &app);
                                            }
                                        }
                                        KeyCode::Char('d')
                                            if app.view_mode == app::ViewMode::Liked
                                                && app.download_liked_progress.is_none() =>
                                        {
                                            start_download_liked(&app, &cmd_tx, false);
                                        }
                                        KeyCode::Char('n') | KeyCode::Char('т') => {
                                            if app.next_track() {
                                                if let Some(ref m) = mpris {
                                                    m.update_track(app.current_track().unwrap())
                                                        .await;
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                                play_track(&player_tx, &app, &cmd_tx);
                                            }
                                        }
                                        KeyCode::Char('p') | KeyCode::Char('з') => {
                                            if app.prev_track() {
                                                if let Some(ref m) = mpris {
                                                    m.update_track(app.current_track().unwrap())
                                                        .await;
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                                play_track(&player_tx, &app, &cmd_tx);
                                            }
                                        }
                                        KeyCode::Char('g') | KeyCode::Char('п') => {
                                            app.toggle_lyrics();
                                        }
                                        KeyCode::Char('x') | KeyCode::Char('ч') => {
                                            if app.view_mode == app::ViewMode::Liked {
                                                app.toggle_liked_shuffle();
                                                save_all_config(&app, &cfg);
                                            } else {
                                                app.toggle_shuffle();
                                            }
                                        }
                                        KeyCode::Char('r') | KeyCode::Char('к') => {
                                            app.cycle_repeat();
                                        }
                                        KeyCode::Char('w') | KeyCode::Char('ц') => {
                                            let fresh = config::load();
                                            if fresh.lastfm_api_key.is_empty() {
                                                app.status_message = "Last.fm key not set — add \"lastfm_api_key\" to ~/.config/music-player-tui/config.json (free key: https://www.last.fm/api/account/create)".to_string();
                                            } else {
                                                start_radio(&app, &cmd_tx, fresh.lastfm_api_key);
                                            }
                                        }
                                        KeyCode::Char('[') | KeyCode::Char('х') => {
                                            app.speed_down();
                                            let _ = player_tx.send(
                                                player::PlayerCommand::SetSpeed(app.playback_speed),
                                            );
                                        }
                                        KeyCode::Char(']') | KeyCode::Char('ъ') => {
                                            app.speed_up();
                                            let _ = player_tx.send(
                                                player::PlayerCommand::SetSpeed(app.playback_speed),
                                            );
                                        }
                                        KeyCode::Left => {
                                            if let Some(pos) = app.position_ms() {
                                                let target = pos.saturating_sub(5000);
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Seek(
                                                        target as f64 / 1000.0,
                                                    ));
                                                app.set_position(target);
                                                if app.is_playing {
                                                    discord_state(
                                                        &discord,
                                                        "Playing",
                                                        app.position_ms(),
                                                        true,
                                                    );
                                                }
                                            }
                                        }
                                        KeyCode::Right => {
                                            if let Some(pos) = app.position_ms() {
                                                let mut target = pos + 5000;
                                                if let Some(dur) = app.track_duration_ms {
                                                    target = target
                                                        .min(dur.saturating_sub(500).max(pos));
                                                }
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Seek(
                                                        target as f64 / 1000.0,
                                                    ));
                                                app.set_position(target);
                                                if app.is_playing {
                                                    discord_state(
                                                        &discord,
                                                        "Playing",
                                                        app.position_ms(),
                                                        true,
                                                    );
                                                }
                                            }
                                        }
                                        KeyCode::Char('+') | KeyCode::Char('=') => {
                                            app.volume_up();
                                            let _ = player_tx
                                                .send(player::PlayerCommand::SetVolume(app.volume));
                                            if let Some(ref m) = mpris {
                                                m.set_volume(app.volume).await;
                                            }
                                        }
                                        KeyCode::Char('-') => {
                                            app.volume_down();
                                            let _ = player_tx
                                                .send(player::PlayerCommand::SetVolume(app.volume));
                                            if let Some(ref m) = mpris {
                                                m.set_volume(app.volume).await;
                                            }
                                        }
                                        KeyCode::Enter => {
                                            if app.view_mode == app::ViewMode::Playlists
                                                && app.active_playlist_id.is_none()
                                            {
                                                if let Some(pl) = app.selected_playlist() {
                                                    app.open_playlist(pl.id);
                                                }
                                            } else {
                                                app.is_playing = true;
                                                if let Some(ref m) = mpris {
                                                    if let Some(t) = app.current_track() {
                                                        m.update_track(t).await;
                                                    }
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                                play_track(&player_tx, &app, &cmd_tx);
                                            }
                                        }
                                        KeyCode::Esc => {
                                            if app.view_mode == app::ViewMode::Playlists {
                                                if app.active_playlist_id.is_some() {
                                                    app.active_playlist_id = None;
                                                    app.load_playlists();
                                                    app.tracks.clear();
                                                    app.current_index = 0;
                                                    app.playing_track_id = None;
                                                    app.playing_track = None;
                                                } else {
                                                    app.view_mode = app::ViewMode::Search;
                                                    app.status_message =
                                                        "Switched to search".to_string();
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                }
                Event::Mouse(mouse) => {
                    redraw = true;
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    let x = mouse.column;
                    let y = mouse.row;

                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(1),
                            Constraint::Length(1),
                            Constraint::Min(5),
                            Constraint::Length(3),
                            Constraint::Length(1),
                        ])
                        .split(area);

                    let header_area = chunks[0];
                    let track_area = chunks[2];
                    let player_area = chunks[3];
                    let footer_area = chunks[4];

                    let lyrics_view = app.view_mode == app::ViewMode::Lyrics;

                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            if !lyrics_view
                                && track_area.y <= y
                                && y < track_area.y + track_area.height
                            {
                                app.prev();
                            } else if player_area.y <= y && y < player_area.y + player_area.height {
                                app.volume_up();
                                let _ =
                                    player_tx.send(player::PlayerCommand::SetVolume(app.volume));
                                if let Some(ref m) = mpris {
                                    m.set_volume(app.volume).await;
                                }
                            }
                        }
                        MouseEventKind::ScrollDown => {
                            if !lyrics_view
                                && track_area.y <= y
                                && y < track_area.y + track_area.height
                            {
                                app.next();
                            } else if player_area.y <= y && y < player_area.y + player_area.height {
                                app.volume_down();
                                let _ =
                                    player_tx.send(player::PlayerCommand::SetVolume(app.volume));
                                if let Some(ref m) = mpris {
                                    m.set_volume(app.volume).await;
                                }
                            }
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            if !lyrics_view
                                && track_area.y <= y
                                && y < track_area.y + track_area.height
                            {
                                if let Some(idx) = track_idx_at(&app, &track_area, y) {
                                    app.current_index = idx;
                                    app.toggle_like();
                                }
                            }
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            if header_area.y <= y && y < header_area.y + header_area.height {
                                app.toggle_help();
                            } else if !lyrics_view
                                && track_area.y <= y
                                && y < track_area.y + track_area.height
                            {
                                if let Some(idx) = track_idx_at(&app, &track_area, y) {
                                    app.current_index = idx;
                                    if app.view_mode == app::ViewMode::Playlists
                                        && app.active_playlist_id.is_none()
                                    {
                                        if let Some(pl) = app.selected_playlist() {
                                            app.open_playlist(pl.id);
                                        }
                                    } else {
                                        app.is_playing = true;
                                        app.loading_play = true;
                                        app.note_playing();
                                        if let Some(ref m) = mpris {
                                            if let Some(t) = app.current_track() {
                                                m.update_track(t).await;
                                            }
                                            m.set_playing(true).await;
                                        }
                                        discord_set(&discord, &app);
                                        play_track(&player_tx, &app, &cmd_tx);
                                    }
                                }
                            } else if player_area.y <= y && y < player_area.y + player_area.height {
                                let rel_x = x.saturating_sub(player_area.x) as usize;
                                let c = ui::ctrl_start(player_area.width, app.volume);
                                if rel_x >= c {
                                    let rel = rel_x - c;
                                    if rel < ui::CTRL_PLAY_START {
                                        if app.prev_track() {
                                            if let Some(ref m) = mpris {
                                                if let Some(t) = app.current_track() {
                                                    m.update_track(t).await;
                                                }
                                                m.set_playing(true).await;
                                            }
                                            discord_set(&discord, &app);
                                            play_track(&player_tx, &app, &cmd_tx);
                                        }
                                    } else if rel < ui::CTRL_NEXT {
                                        match app.toggle_play() {
                                            PlayAction::Pause => {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Pause);
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(false).await;
                                                }
                                                discord_state(&discord, "Paused", None, false);
                                            }
                                            PlayAction::Resume => {
                                                let _ =
                                                    player_tx.send(player::PlayerCommand::Resume);
                                                if let Some(ref m) = mpris {
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                            }
                                            PlayAction::NewTrack => {
                                                if let Some(ref m) = mpris {
                                                    if let Some(t) = app.current_track() {
                                                        m.update_track(t).await;
                                                    }
                                                    m.set_playing(true).await;
                                                }
                                                discord_set(&discord, &app);
                                                play_track(&player_tx, &app, &cmd_tx);
                                            }
                                        }
                                    } else if rel < ui::CTRL_VOL_START {
                                        if app.next_track() {
                                            if let Some(ref m) = mpris {
                                                if let Some(t) = app.current_track() {
                                                    m.update_track(t).await;
                                                }
                                                m.set_playing(true).await;
                                            }
                                            discord_set(&discord, &app);
                                            play_track(&player_tx, &app, &cmd_tx);
                                        }
                                    } else if rel < ui::CTRL_VOL_START + ui::CTRL_VOL_LEN {
                                        let frac = (rel - ui::CTRL_VOL_START) as f32
                                            / ui::CTRL_VOL_LEN as f32;
                                        app.set_volume(frac);
                                        let _ = player_tx
                                            .send(player::PlayerCommand::SetVolume(app.volume));
                                        if let Some(ref m) = mpris {
                                            m.set_volume(app.volume).await;
                                        }
                                    }
                                } else {
                                    match app.toggle_play() {
                                        PlayAction::Pause => {
                                            let _ = player_tx.send(player::PlayerCommand::Pause);
                                            if let Some(ref m) = mpris {
                                                m.set_playing(false).await;
                                            }
                                            discord_state(&discord, "Paused", None, false);
                                        }
                                        PlayAction::Resume => {
                                            let _ = player_tx.send(player::PlayerCommand::Resume);
                                            if let Some(ref m) = mpris {
                                                m.set_playing(true).await;
                                            }
                                            discord_set(&discord, &app);
                                        }
                                        PlayAction::NewTrack => {
                                            if let Some(ref m) = mpris {
                                                if let Some(t) = app.current_track() {
                                                    m.update_track(t).await;
                                                }
                                                m.set_playing(true).await;
                                            }
                                            discord_set(&discord, &app);
                                            play_track(&player_tx, &app, &cmd_tx);
                                        }
                                    }
                                }
                            } else if footer_area.y <= y && y < footer_area.y + footer_area.height {
                                if let Some(dur_ms) = app.track_duration_ms {
                                    if app.started_at.is_some() || app.paused_at_ms.is_some() {
                                        let rel_x = x.saturating_sub(footer_area.x + 2) as usize;
                                        if rel_x < 20 {
                                            let frac = rel_x as f64 / 20.0;
                                            let target_ms = (frac * dur_ms as f64) as u64;
                                            let _ = player_tx.send(player::PlayerCommand::Seek(
                                                target_ms as f64 / 1000.0,
                                            ));
                                            app.set_position(target_ms);
                                            if app.is_playing {
                                                discord_state(
                                                    &discord,
                                                    "Playing",
                                                    app.position_ms(),
                                                    true,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                Event::Resize(..) => {
                    redraw = true;
                    terminal.autoresize()?;
                }
                _ => {}
            }
        }

        // Handle MPRIS commands from media keys
        while let Ok(cmd) = mpris_rx.try_recv() {
            redraw = true;
            match cmd {
                mpris::MprisCommand::Pause => {
                    let _ = player_tx.send(player::PlayerCommand::Pause);
                    app.pause();
                    if let Some(ref m) = mpris {
                        m.set_playing(false).await;
                    }
                    discord_state(&discord, "Paused", None, false);
                }
                mpris::MprisCommand::Resume => {
                    let _ = player_tx.send(player::PlayerCommand::Resume);
                    app.resume();
                    if let Some(ref m) = mpris {
                        m.set_playing(true).await;
                    }
                    discord_resume(&discord, &app);
                }
                mpris::MprisCommand::PlayPause => match app.toggle_play() {
                    PlayAction::Pause => {
                        let _ = player_tx.send(player::PlayerCommand::Pause);
                        if let Some(ref m) = mpris {
                            m.set_playing(false).await;
                        }
                        discord_state(&discord, "Paused", None, false);
                    }
                    PlayAction::Resume => {
                        let _ = player_tx.send(player::PlayerCommand::Resume);
                        if let Some(ref m) = mpris {
                            m.set_playing(true).await;
                        }
                        discord_set(&discord, &app);
                    }
                    PlayAction::NewTrack => {
                        if let Some(ref m) = mpris {
                            if let Some(t) = app.current_track() {
                                m.update_track(t).await;
                            }
                            m.set_playing(true).await;
                        }
                        discord_set(&discord, &app);
                        play_track(&player_tx, &app, &cmd_tx);
                    }
                },
                mpris::MprisCommand::Next => {
                    if app.next_track() {
                        if let Some(ref m) = mpris {
                            if let Some(t) = app.current_track() {
                                m.update_track(t).await;
                            }
                            m.set_playing(true).await;
                        }
                        discord_set(&discord, &app);
                        play_track(&player_tx, &app, &cmd_tx);
                    }
                }
                mpris::MprisCommand::Previous => {
                    if app.prev_track() {
                        if let Some(ref m) = mpris {
                            if let Some(t) = app.current_track() {
                                m.update_track(t).await;
                            }
                            m.set_playing(true).await;
                        }
                        discord_set(&discord, &app);
                        play_track(&player_tx, &app, &cmd_tx);
                    }
                }
                mpris::MprisCommand::Stop => {
                    let _ = player_tx.send(player::PlayerCommand::Pause);
                    if let Some(ref m) = mpris {
                        m.set_playing(false).await;
                    }
                    discord_state(&discord, "Stopped", None, false);
                }
            }
        }

        while let Ok(cmd) = cmd_rx.try_recv() {
            redraw = true;
            let auto_dl = match &cmd {
                app::Command::DownloadLikedTrack(t) => Some(t.clone()),
                _ => None,
            };
            if let Some(track) = auto_dl {
                if track.duration_ms.is_none()
                    || !artwork_is_big(track.source, track.artwork_url.as_deref())
                {
                    fetch_meta_into(&cmd_tx, track.clone(), &app);
                }
                let token = app.token.clone();
                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let _ = tokio::fs::create_dir_all(downloads_dir()).await;
                    let filename =
                        sanitize_filename(&format!("{} - {}.mp3", track.artist, track.title));
                    let dest = downloads_dir().join(&filename);
                    let result = if dest.exists() {
                        Ok(())
                    } else {
                        match track.source {
                            api::Source::ITunes => match &track.preview_url {
                                Some(url) => download_file(url, &dest).await,
                                None => Err("no preview url".into()),
                            },
                            api::Source::SoundCloud => download_soundcloud(&track.id, &dest).await,
                            api::Source::YouTubeMusic => download_ytmusic(&track.id, &dest).await,
                            api::Source::YandexMusic => {
                                download_yandex(&track.id, &token, &dest).await
                            }
                        }
                    };
                    let _ = tx.send(app::Command::DownloadLikedTrackDone {
                        title: track.title.clone(),
                        ok: result.is_ok(),
                    });
                });
            }
            let lyrics_fetch = matches!(&cmd, app::Command::FetchLyrics);
            if lyrics_fetch {
                let (artist, title) = match app.current_track() {
                    Some(t) => (t.artist.clone(), t.title.clone()),
                    None => (String::new(), String::new()),
                };
                let track_id = app.current_track_id().unwrap_or_default();
                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let (lines, source) = match api::get_lyrics(&artist, &title).await {
                        Ok(Some(lines)) => (Some(lines), "LRCLIB".to_string()),
                        _ => (None, String::new()),
                    };
                    let (lines, source) = if lines.is_none() {
                        match api::get_netease_lyrics(&artist, &title).await {
                            Ok(Some(lines)) => (Some(lines), "NetEase".to_string()),
                            _ => (None, String::new()),
                        }
                    } else {
                        (lines, source)
                    };
                    match lines {
                        Some(lines) => {
                            let _ = tx.send(app::Command::LyricsLoaded {
                                track_id,
                                lines,
                                source,
                            });
                        }
                        None => {
                            let _ =
                                tx.send(app::Command::LyricsError("lyrics not found".to_string()));
                        }
                    }
                });
            }
            let is_play = matches!(&cmd, app::Command::PlayStarted { .. });
            let is_err = matches!(&cmd, app::Command::PlayError(_));
            let is_finished = matches!(&cmd, app::Command::PlaybackFinished);
            app.handle_command(cmd);
            if is_play {
                if let Some(ref m) = mpris {
                    m.set_playing(true).await;
                }
                discord_set(&discord, &app);
                if let Some(t) = app.current_track() {
                    let tx = cmd_tx.clone();
                    let artist = t.artist.clone();
                    let title = t.title.clone();
                    let tid = app.current_track_id().unwrap_or_default();
                    tokio::spawn(async move {
                        if let Ok(Some(meta)) = api::get_mb_metadata(&artist, &title).await {
                            let _ = tx.send(app::Command::TrackMetadata {
                                track_id: tid,
                                album: meta.album,
                                year: meta.year,
                            });
                        }
                    });
                }
            } else if is_err {
                if let Some(ref m) = mpris {
                    m.set_playing(false).await;
                }
                discord_state(&discord, "Stopped", None, false);
            } else if is_finished {
                if let Some(ref m) = mpris {
                    m.set_playing(false).await;
                }
                discord_state(&discord, "Stopped", None, false);
                let do_advance = match app.repeat_mode {
                    app::RepeatMode::One => true,
                    app::RepeatMode::All => {
                        !(app.radio_active && app.current_index + 1 >= app.tracks.len())
                    }
                    app::RepeatMode::Off => app.current_index + 1 < app.tracks.len(),
                };
                if app.radio_active && app.current_index + 1 >= app.tracks.len() {
                    app.radio_active = false;
                    app.status_message = "Radio ended".to_string();
                }
                if app.repeat_mode == app::RepeatMode::One {
                    app.replay();
                } else if do_advance {
                    app.next_track();
                }
                if do_advance || app.repeat_mode == app::RepeatMode::One {
                    if let Some(ref m) = mpris {
                        if let Some(t) = app.current_track() {
                            m.update_track(t).await;
                        }
                        m.set_playing(true).await;
                    }
                    discord_set(&discord, &app);
                    play_track(&player_tx, &app, &cmd_tx);
                }
            }
        }

        if redraw || last_progress.elapsed() >= std::time::Duration::from_millis(500) {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            last_progress = std::time::Instant::now();
        }
    }

    let _ = player_tx.send(player::PlayerCommand::Stop);
    drop(player_tx);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), player_handle).await;

    Ok(())
}
