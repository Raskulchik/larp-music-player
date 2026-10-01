use crate::api::{self, Source, Track};
use crate::db::{Database, Playlist};
use tokio::sync::mpsc;

pub enum Command {
    SearchResults(Vec<Track>),
    SearchError(String),
    PlayStarted {
        actual_duration_ms: Option<u64>,
    },
    PlaybackFinished,
    PlayError(String),
    DownloadProgress(u32),
    DownloadLikedProgress {
        current: usize,
        total: usize,
    },
    DownloadLikedDone {
        downloaded: usize,
        failed: usize,
    },
    DownloadLikedTrack(Track),
    DownloadLikedTrackDone {
        title: String,
        ok: bool,
    },
    Status(String),
    FetchLyrics,
    LyricsLoaded {
        track_id: String,
        lines: Vec<(u64, String)>,
        source: String,
    },
    LyricsError(String),
    TrackMetadata {
        track_id: String,
        album: Option<String>,
        year: Option<u16>,
    },
    MetaUpdated {
        source: Source,
        track_id: String,
        duration_ms: Option<u64>,
        artwork_url: Option<String>,
    },
    UpdateAvailable {
        tag: String,
        url: String,
    },
    RadioFound(Vec<Track>),
    BluetoothStatus(Option<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    TokenInput,
    ClientIdInput,
    SourceSelect,
    ViewSelect,
    PlaylistSelect,
    NameInput,
    Help,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ViewMode {
    Search,
    Liked,
    Lyrics,
    Playlists,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RepeatMode {
    Off,
    All,
    One,
}

const SPEEDS: [f32; 5] = [0.5, 0.75, 1.0, 1.25, 1.5];

fn shuffle_index(len: usize, exclude: usize) -> usize {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let mut state = (t.as_nanos() as u64) ^ (len as u64).wrapping_mul(0x9E3779B97F4A7C15);
    state ^= state >> 33;
    state = state.wrapping_mul(0xFF51AFD7ED558CCD);
    state ^= state >> 33;
    let mut idx = (state % len as u64) as usize;
    if len > 1 && idx == exclude {
        idx = (idx + 1) % len;
    }
    idx
}

pub struct App {
    pub tracks: Vec<Track>,
    pub current_index: usize,
    pub is_playing: bool,
    pub volume: f32,
    pub current_source: Source,
    pub search_query: String,
    pub token: String,
    pub sc_client_id: String,
    pub input_mode: InputMode,
    pub view_mode: ViewMode,
    pub status_message: String,
    pub loading: bool,
    pub download_progress: Option<u32>,
    pub loading_play: bool,
    pub started_at: Option<std::time::Instant>,
    pub paused_at_ms: Option<u64>,
    pub track_duration_ms: Option<u64>,
    pub playing_track_id: Option<String>,
    pub playing_track: Option<Track>,
    pub download_liked_progress: Option<(usize, usize)>,
    pub db: Database,
    pub shuffle: bool,
    pub repeat_mode: RepeatMode,
    pub playback_speed: f32,
    pub lyrics_lines: Vec<(u64, String)>,
    pub lyrics_loading: bool,
    pub lyrics_error: Option<String>,
    pub lyrics_track_id: Option<String>,
    pub lyrics_source: Option<String>,
    pub radio_active: bool,
    pub liked_shuffle: bool,
    pub select_index: usize,
    pub bluetooth_device: Option<String>,
    pub playlists: Vec<Playlist>,
    pub active_playlist_id: Option<i64>,
    pub name_input: String,
    pub update_available: Option<String>,
    cmd_tx: mpsc::UnboundedSender<Command>,
}

impl App {
    pub fn new(
        cmd_tx: mpsc::UnboundedSender<Command>,
        saved_token: Option<String>,
        saved_sc_client_id: Option<String>,
        saved_liked_shuffle: bool,
    ) -> Self {
        let has_token = saved_token.as_ref().map(|t| !t.is_empty()).unwrap_or(false);
        let token = saved_token.unwrap_or_default();
        let sc_client_id = saved_sc_client_id.unwrap_or_default();
        let db = Database::open().expect("Failed to open database");

        Self {
            tracks: Vec::new(),
            current_index: 0,
            is_playing: false,
            volume: 0.7,
            current_source: if has_token {
                Source::YandexMusic
            } else {
                Source::ITunes
            },
            search_query: String::new(),
            token,
            sc_client_id,
            input_mode: InputMode::Normal,
            view_mode: ViewMode::Search,
            status_message: if has_token {
                "Token loaded! Press / to search".to_string()
            } else {
                "No Yandex token — using iTunes. / search, t source, L like".to_string()
            },
            loading: false,
            download_progress: None,
            loading_play: false,
            started_at: None,
            paused_at_ms: None,
            track_duration_ms: None,
            playing_track_id: None,
            playing_track: None,
            download_liked_progress: None,
            db,
            shuffle: false,
            repeat_mode: RepeatMode::Off,
            playback_speed: 1.0,
            lyrics_lines: Vec::new(),
            lyrics_loading: false,
            lyrics_error: None,
            lyrics_track_id: None,
            lyrics_source: None,
            radio_active: false,
            liked_shuffle: saved_liked_shuffle,
            select_index: 0,
            bluetooth_device: None,
            playlists: Vec::new(),
            active_playlist_id: None,
            name_input: String::new(),
            update_available: None,
            cmd_tx,
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.tracks.get(self.current_index)
    }

    pub fn current_track_id(&self) -> Option<String> {
        self.current_track()
            .map(|t| format!("{}:{}", t.source as u8, t.id))
    }

    pub fn note_playing(&mut self) {
        self.playing_track_id = self.current_track_id();
        self.playing_track = self.current_track().cloned();
    }

    pub fn prev(&mut self) {
        if self.view_mode == ViewMode::Playlists && self.active_playlist_id.is_none() {
            if self.current_index > 0 {
                self.current_index -= 1;
            }
        } else if self.current_index > 0 {
            self.current_index -= 1;
        }
    }

    pub fn next(&mut self) {
        if self.view_mode == ViewMode::Playlists && self.active_playlist_id.is_none() {
            if !self.playlists.is_empty() && self.current_index < self.playlists.len() - 1 {
                self.current_index += 1;
            }
        } else if !self.tracks.is_empty() && self.current_index < self.tracks.len() - 1 {
            self.current_index += 1;
        }
    }

    pub fn toggle_help(&mut self) {
        self.input_mode = if self.input_mode == InputMode::Help {
            InputMode::Normal
        } else {
            InputMode::Help
        };
    }

    pub fn pause(&mut self) {
        self.is_playing = false;
        self.status_message = "Paused".to_string();
        self.paused_at_ms = self.position_ms();
        self.started_at = None;
    }

    pub fn resume(&mut self) {
        self.is_playing = true;
        self.status_message = "Playing".to_string();
        if let Some(paused) = self.paused_at_ms {
            let real = (paused as f64 / self.playback_speed.max(0.01) as f64) as u64;
            self.started_at =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(real));
            self.paused_at_ms = None;
        }
    }

    pub fn position_ms(&self) -> Option<u64> {
        if let Some(paused) = self.paused_at_ms {
            Some(paused)
        } else if let Some(started) = self.started_at {
            let elapsed = started.elapsed().as_millis() as u64;
            Some(((elapsed as f64) * self.playback_speed as f64) as u64)
        } else {
            None
        }
    }

    pub fn set_position(&mut self, ms: u64) {
        let real_ms = (ms as f64 / self.playback_speed.max(0.01) as f64) as u64;
        if self.paused_at_ms.is_some() {
            self.paused_at_ms = Some(ms);
        } else if self.started_at.is_some() {
            self.started_at =
                Some(std::time::Instant::now() - std::time::Duration::from_millis(real_ms));
        }
    }

    pub fn replay(&mut self) {
        if self.tracks.is_empty() {
            return;
        }
        self.is_playing = true;
        self.loading_play = true;
        self.note_playing();
        self.paused_at_ms = None;
    }

    pub fn toggle_play(&mut self) -> PlayAction {
        let track_id = self.current_track_id();

        if self.is_playing {
            self.pause();
            PlayAction::Pause
        } else {
            self.is_playing = true;
            self.status_message = "Playing".to_string();

            if self.playing_track_id == track_id {
                self.resume();
                PlayAction::Resume
            } else {
                self.note_playing();
                self.loading_play = true;
                self.paused_at_ms = None;
                PlayAction::NewTrack
            }
        }
    }

    pub fn next_track(&mut self) -> bool {
        if self.tracks.is_empty() {
            return false;
        }
        if self.view_mode == ViewMode::Liked {
            self.current_index = (self.current_index + 1) % self.tracks.len();
        } else if self.shuffle && self.tracks.len() > 1 {
            self.current_index = shuffle_index(self.tracks.len(), self.current_index);
        } else {
            self.current_index = (self.current_index + 1) % self.tracks.len();
        }
        self.is_playing = true;
        self.loading_play = true;
        self.note_playing();
        true
    }

    pub fn prev_track(&mut self) -> bool {
        if !self.tracks.is_empty() {
            self.current_index = if self.current_index == 0 {
                self.tracks.len() - 1
            } else {
                self.current_index - 1
            };
            self.is_playing = true;
            self.loading_play = true;
            self.note_playing();
            true
        } else {
            false
        }
    }

    pub fn toggle_shuffle(&mut self) {
        self.shuffle = !self.shuffle;
        self.status_message = if self.shuffle {
            "Shuffle: on".to_string()
        } else {
            "Shuffle: off".to_string()
        };
    }

    fn shuffle_tracks(tracks: &mut Vec<Track>) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        let mut state = seed | 1;
        for i in (1..tracks.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let j = (state as usize) % (i + 1);
            tracks.swap(i, j);
        }
    }

    pub fn toggle_liked_shuffle(&mut self) {
        self.liked_shuffle = !self.liked_shuffle;
        let anchor = self.current_track().map(|t| (t.source, t.id.clone()));
        if self.liked_shuffle {
            Self::shuffle_tracks(&mut self.tracks);
        } else if let Ok(tracks) = self.db.get_liked() {
            self.tracks = tracks;
        }
        if let Some((src, id)) = anchor {
            if let Some(idx) = self
                .tracks
                .iter()
                .position(|t| t.source == src && t.id == id)
            {
                self.current_index = idx;
            } else {
                self.current_index = 0;
            }
        } else {
            self.current_index = 0;
        }
        self.status_message = if self.liked_shuffle {
            "Liked: random order".to_string()
        } else {
            "Liked: sequential".to_string()
        };
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat_mode = match self.repeat_mode {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        };
        let label = match self.repeat_mode {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        };
        self.status_message = format!("Repeat: {}", label);
    }

    pub fn speed_up(&mut self) {
        if let Some(s) = SPEEDS.iter().find(|s| **s > self.playback_speed + 0.01) {
            self.playback_speed = *s;
        }
    }

    pub fn speed_down(&mut self) {
        if let Some(s) = SPEEDS
            .iter()
            .rev()
            .find(|s| **s < self.playback_speed - 0.01)
        {
            self.playback_speed = *s;
        }
    }

    pub fn toggle_lyrics(&mut self) {
        self.view_mode = if self.view_mode == ViewMode::Lyrics {
            ViewMode::Search
        } else {
            ViewMode::Lyrics
        };
    }

    pub fn maybe_fetch_lyrics(&mut self) {
        if self.view_mode != ViewMode::Lyrics {
            return;
        }
        let id = match self.current_track_id() {
            Some(id) => id,
            None => return,
        };
        if self.lyrics_track_id.as_deref() == Some(id.as_str()) {
            return;
        }
        self.lyrics_track_id = Some(id);
        self.lyrics_loading = true;
        self.lyrics_error = None;
        self.lyrics_source = None;
        let _ = self.cmd_tx.send(Command::FetchLyrics);
    }

    pub fn volume_up(&mut self) {
        self.volume = (self.volume + 0.1).min(1.0);
    }

    pub fn volume_down(&mut self) {
        self.volume = (self.volume - 0.1).max(0.0);
    }

    pub fn set_volume(&mut self, pct: f32) {
        self.volume = pct.clamp(0.0, 1.0);
    }

    pub fn toggle_like(&mut self) {
        let track = self.current_track().cloned();
        if let Some(track) = track {
            match self.db.is_liked(&track.source, &track.id) {
                Ok(true) => {
                    let _ = self.db.unlike_track(&track.source, &track.id);
                    self.status_message = format!("Unliked: {}", track.title);
                }
                Ok(false) => {
                    let _ = self.db.like_track(&track);
                    self.status_message = format!("Liked: {}", track.title);
                    let _ = self.cmd_tx.send(Command::DownloadLikedTrack(track.clone()));
                }
                Err(e) => {
                    self.status_message = format!("Like error: {}", e);
                }
            }
        }
    }

    pub fn is_current_liked(&self) -> bool {
        self.current_track()
            .and_then(|t| self.db.is_liked(&t.source, &t.id).ok())
            .unwrap_or(false)
    }

    pub fn start_source_select(&mut self) {
        self.select_index = match self.current_source {
            Source::YandexMusic => 0,
            Source::ITunes => 1,
            Source::SoundCloud => 2,
            Source::YouTubeMusic => 3,
        };
        self.input_mode = InputMode::SourceSelect;
    }

    pub fn start_view_select(&mut self) {
        self.select_index = match self.view_mode {
            ViewMode::Search => 0,
            ViewMode::Liked => 1,
            ViewMode::Lyrics => 2,
            ViewMode::Playlists => 3,
        };
        self.input_mode = InputMode::ViewSelect;
    }

    pub fn select_up(&mut self) {
        match self.input_mode {
            InputMode::SourceSelect => {
                self.select_index = (self.select_index + 3) % 4;
            }
            InputMode::ViewSelect => {
                self.select_index = (self.select_index + 3) % 4;
            }
            InputMode::PlaylistSelect => {
                if self.playlists.len() > 0 {
                    self.select_index =
                        (self.select_index + self.playlists.len() - 1) % self.playlists.len();
                }
            }
            _ => {}
        }
    }

    pub fn select_down(&mut self) {
        match self.input_mode {
            InputMode::SourceSelect => {
                self.select_index = (self.select_index + 1) % 4;
            }
            InputMode::ViewSelect => {
                self.select_index = (self.select_index + 1) % 4;
            }
            InputMode::PlaylistSelect => {
                if !self.playlists.is_empty() {
                    self.select_index = (self.select_index + 1) % self.playlists.len();
                }
            }
            _ => {}
        }
    }

    pub fn confirm_source_select(&mut self) {
        let source = match self.select_index {
            0 => Source::YandexMusic,
            1 => Source::ITunes,
            2 => Source::SoundCloud,
            _ => Source::YouTubeMusic,
        };
        if source == self.current_source {
            self.input_mode = InputMode::Normal;
            return;
        }
        self.current_source = source;
        self.tracks.clear();
        self.current_index = 0;
        self.playing_track_id = None;
        self.playing_track = None;
        self.radio_active = false;

        match self.current_source {
            Source::YandexMusic => {
                if self.token.is_empty() {
                    self.input_mode = InputMode::Normal;
                    self.status_message = "Yandex needs a token — press / to enter it".to_string();
                } else {
                    self.input_mode = InputMode::Search;
                    self.status_message = "Switched to Yandex Music".to_string();
                }
            }
            Source::ITunes => {
                self.input_mode = InputMode::Search;
                self.status_message = "Switched to iTunes".to_string();
            }
            Source::SoundCloud => {
                api::set_sc_client_id(&self.sc_client_id);
                self.input_mode = InputMode::Search;
                self.status_message = "Switched to SoundCloud".to_string();
            }
            Source::YouTubeMusic => {
                self.input_mode = InputMode::Search;
                self.status_message = "Switched to YouTube Music".to_string();
            }
        }
    }

    pub fn confirm_view_select(&mut self) {
        let view = match self.select_index {
            0 => ViewMode::Search,
            1 => ViewMode::Liked,
            2 => ViewMode::Lyrics,
            _ => ViewMode::Playlists,
        };
        self.input_mode = InputMode::Normal;
        match view {
            ViewMode::Search => {
                self.view_mode = ViewMode::Search;
                self.status_message = "Switched to search".to_string();
            }
            ViewMode::Liked => {
                self.view_mode = ViewMode::Liked;
                self.load_liked();
            }
            ViewMode::Lyrics => {
                self.view_mode = ViewMode::Lyrics;
                self.status_message = "Lyrics — g to hide".to_string();
            }
            ViewMode::Playlists => {
                self.enter_playlists();
            }
        }
    }

    pub fn load_liked(&mut self) {
        match self.db.get_liked() {
            Ok(mut tracks) => {
                if self.liked_shuffle {
                    Self::shuffle_tracks(&mut tracks);
                }
                let count = tracks.len();
                self.tracks = tracks;
                self.current_index = 0;
                self.playing_track_id = None;
                self.playing_track = None;
                self.radio_active = false;
                self.status_message = format!("Liked songs: {}", count);
            }
            Err(e) => {
                self.status_message = format!("Error loading liked: {}", e);
            }
        }
    }

    pub fn enter_playlists(&mut self) {
        self.view_mode = ViewMode::Playlists;
        self.active_playlist_id = None;
        self.load_playlists();
        self.tracks.clear();
        self.current_index = 0;
        self.status_message = "Playlists — Shift+N new, Shift+A add, Shift+D delete".to_string();
    }

    pub fn load_playlists(&mut self) {
        match self.db.get_playlists() {
            Ok(list) => {
                self.playlists = list;
            }
            Err(e) => {
                self.status_message = format!("Error loading playlists: {}", e);
            }
        }
    }

    pub fn open_playlist(&mut self, id: i64) {
        match self.db.get_playlist_tracks(id) {
            Ok(tracks) => {
                self.active_playlist_id = Some(id);
                let count = tracks.len();
                self.tracks = tracks;
                self.current_index = 0;
                self.playing_track_id = None;
                self.playing_track = None;
                self.radio_active = false;
                if let Some(name) = self
                    .playlists
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| p.name.clone())
                {
                    self.status_message =
                        format!("{}: {} tracks — Shift+D remove from playlist", name, count);
                }
            }
            Err(e) => {
                self.status_message = format!("Error opening playlist: {}", e);
            }
        }
    }

    pub fn playlists_view_open(&self) -> bool {
        self.view_mode == ViewMode::Playlists && self.active_playlist_id.is_some()
    }

    pub fn playlist_name(&self) -> Option<String> {
        self.playlists
            .iter()
            .find(|p| Some(p.id) == self.active_playlist_id)
            .map(|p| p.name.clone())
    }

    pub fn selected_playlist(&self) -> Option<Playlist> {
        self.playlists.get(self.current_index).cloned()
    }

    pub fn create_playlist(&mut self) {
        let name = self.name_input.trim().to_string();
        self.name_input.clear();
        self.input_mode = InputMode::Normal;
        if name.is_empty() {
            self.status_message = "Playlist name cannot be empty".to_string();
            return;
        }
        match self.db.create_playlist(&name) {
            Ok(_) => {
                self.enter_playlists();
                self.status_message =
                    format!("Playlist \"{}\" created — Shift+A to add tracks", name);
            }
            Err(e) => {
                self.status_message = format!("Create failed (maybe duplicate): {}", e);
            }
        }
    }

    pub fn delete_selected_playlist(&mut self) {
        if self.playlists.is_empty() {
            self.status_message = "No playlists to delete".to_string();
            return;
        }
        let idx = self
            .current_index
            .min(self.playlists.len().saturating_sub(1));
        let pl = self.playlists[idx].clone();
        if let Err(e) = self.db.delete_playlist(pl.id) {
            self.status_message = format!("Delete failed: {}", e);
            return;
        }
        self.load_playlists();
        if !self.playlists.is_empty() && self.current_index >= self.playlists.len() {
            self.current_index = self.playlists.len() - 1;
        }
        self.status_message = format!("Playlist \"{}\" deleted", pl.name);
    }

    pub fn remove_current_from_playlist(&mut self) {
        if !self.playlists_view_open() {
            return;
        }
        let Some(pl_id) = self.active_playlist_id else {
            return;
        };
        let Some(track) = self.current_track().cloned() else {
            return;
        };
        if let Err(e) = self
            .db
            .remove_from_playlist(pl_id, &track.source, &track.id)
        {
            self.status_message = format!("Remove failed: {}", e);
            return;
        }
        let title = track.title.clone();
        self.tracks.remove(self.current_index);
        if !self.tracks.is_empty() && self.current_index >= self.tracks.len() {
            self.current_index = self.tracks.len() - 1;
        }
        self.status_message = format!("Removed from playlist: {}", title);
        self.load_playlists();
    }

    pub fn start_playlist_add(&mut self) {
        self.load_playlists();
        if self.playlists.is_empty() {
            self.status_message = "No playlists — create with Shift+N".to_string();
            return;
        }
        self.select_index = 0;
        self.input_mode = InputMode::PlaylistSelect;
    }

    pub fn start_name_input(&mut self) {
        self.name_input.clear();
        self.input_mode = InputMode::NameInput;
        self.status_message = "Enter playlist name, Enter to create".to_string();
    }

    pub fn confirm_playlist_add(&mut self) {
        self.input_mode = InputMode::Normal;
        let Some(pl) = self.playlists.get(self.select_index).cloned() else {
            self.status_message = "Select a playlist first".to_string();
            return;
        };
        let Some(track) = self.current_track().cloned() else {
            return;
        };
        match self.db.add_to_playlist(pl.id, &track) {
            Ok(_) => {
                self.load_playlists();
                self.status_message =
                    format!("Added \"{}\" to playlist \"{}\"", track.title, pl.name);
            }
            Err(e) => {
                self.status_message = format!("Add failed: {}", e);
            }
        }
    }

    pub fn input_char(&mut self, c: char) {
        match self.input_mode {
            InputMode::Search => self.search_query.push(c),
            InputMode::TokenInput => self.token.push(c),
            InputMode::ClientIdInput => self.sc_client_id.push(c),
            InputMode::NameInput => self.name_input.push(c),
            _ => {}
        }
    }

    pub fn input_backspace(&mut self) {
        match self.input_mode {
            InputMode::Search => {
                self.search_query.pop();
            }
            InputMode::TokenInput => {
                self.token.pop();
            }
            InputMode::ClientIdInput => {
                self.sc_client_id.pop();
            }
            InputMode::NameInput => {
                self.name_input.pop();
            }
            _ => {}
        }
    }

    pub fn submit(&mut self) {
        match self.input_mode {
            InputMode::TokenInput => {
                if self.token.is_empty() {
                    self.status_message = "Token cannot be empty".to_string();
                    return;
                }
                self.input_mode = InputMode::Search;
                self.status_message = "Token set! Press / to search".to_string();
            }
            InputMode::ClientIdInput => {
                if self.sc_client_id.is_empty() {
                    self.status_message = "client_id cannot be empty".to_string();
                    return;
                }
                self.input_mode = InputMode::Search;
            }
            InputMode::Search => {
                let q = self.search_query.clone();
                self.submit_search();
                if !q.is_empty() {
                    self.input_mode = InputMode::Normal;
                    self.search_query.clear();
                }
            }
            InputMode::SourceSelect => self.confirm_source_select(),
            InputMode::ViewSelect => self.confirm_view_select(),
            InputMode::PlaylistSelect => self.confirm_playlist_add(),
            InputMode::NameInput => self.create_playlist(),
            InputMode::Help => self.toggle_help(),
            InputMode::Normal => {}
        }
    }

    pub fn cancel_input(&mut self) {
        match self.input_mode {
            InputMode::TokenInput => {
                if self.token.is_empty() {
                    self.current_source = Source::ITunes;
                    self.input_mode = InputMode::Search;
                    self.status_message = "Switched to iTunes (no token)".to_string();
                } else {
                    self.input_mode = InputMode::Search;
                }
            }
            InputMode::ClientIdInput => {
                self.current_source = Source::YandexMusic;
                self.input_mode = InputMode::Search;
                self.status_message = "Switched to Yandex Music (no client_id)".to_string();
            }
            InputMode::Search => {
                self.input_mode = InputMode::Normal;
            }
            InputMode::SourceSelect | InputMode::ViewSelect | InputMode::PlaylistSelect => {
                self.input_mode = InputMode::Normal;
            }
            InputMode::NameInput => {
                self.name_input.clear();
                self.input_mode = InputMode::Normal;
            }
            InputMode::Help => {
                self.input_mode = InputMode::Normal;
            }
            InputMode::Normal => {}
        }
    }

    pub fn focus_search(&mut self) {
        self.view_mode = ViewMode::Search;
        match self.current_source {
            Source::YandexMusic => {
                if self.token.is_empty() {
                    self.input_mode = InputMode::TokenInput;
                    self.status_message = "Enter Yandex Music token first:".to_string();
                } else {
                    self.input_mode = InputMode::Search;
                }
            }
            Source::ITunes => {
                self.input_mode = InputMode::Search;
            }
            Source::SoundCloud => {
                self.input_mode = InputMode::Search;
            }
            Source::YouTubeMusic => {
                self.input_mode = InputMode::Search;
            }
        }
    }

    fn submit_search(&self) {
        if self.search_query.is_empty() {
            return;
        }
        let query = self.search_query.clone();
        let tx = self.cmd_tx.clone();
        let source = self.current_source;
        let token = self.token.clone();

        tokio::spawn(async move {
            let result = match source {
                Source::YandexMusic => api::search_yandex(&query, &token).await,
                Source::ITunes => api::search_itunes(&query).await,
                Source::SoundCloud => api::search_soundcloud(&query).await,
                Source::YouTubeMusic => api::search_ytmusic(&query).await,
            };

            match result {
                Ok(tracks) => {
                    let _ = tx.send(Command::SearchResults(tracks));
                }
                Err(e) => {
                    let _ = tx.send(Command::SearchError(e.to_string()));
                }
            }
        });
    }

    pub fn get_play_url(&self) -> Option<(String, Source)> {
        self.current_track().map(|t| match t.source {
            Source::ITunes => (t.preview_url.clone().unwrap_or_default(), t.source),
            Source::SoundCloud => (t.preview_url.clone().unwrap_or_default(), t.source),
            Source::YandexMusic => (t.id.clone(), t.source),
            Source::YouTubeMusic => (t.id.clone(), t.source),
        })
    }

    pub fn get_current_track_id(&self) -> Option<String> {
        self.current_track().map(|t| t.id.clone())
    }

    pub fn handle_command(&mut self, cmd: Command) {
        match cmd {
            Command::SearchResults(tracks) => {
                let count = tracks.len();
                self.tracks = tracks;
                self.current_index = 0;
                self.loading = false;
                self.input_mode = InputMode::Normal;
                self.view_mode = ViewMode::Search;
                self.radio_active = false;
                self.status_message = format!("Found {} tracks", count);
            }
            Command::SearchError(e) => {
                self.loading = false;
                self.status_message = format!("Error: {}", e);
                if self.current_source == Source::SoundCloud && self.sc_client_id.is_empty() {
                    self.input_mode = InputMode::ClientIdInput;
                    self.status_message =
                        "Auto-extract failed. Enter SoundCloud client_id:".to_string();
                }
            }
            Command::PlayStarted { actual_duration_ms } => {
                self.is_playing = true;
                self.loading_play = false;
                self.download_progress = None;
                self.started_at = Some(std::time::Instant::now());
                self.paused_at_ms = None;
                self.note_playing();
                if let Some(dur) = actual_duration_ms {
                    self.track_duration_ms = Some(dur);
                    if let Some(idx) = self.tracks.iter().position(|t| {
                        format!("{}:{}", t.source as u8, t.id)
                            == self.playing_track_id.as_deref().unwrap_or("")
                    }) {
                        self.tracks[idx].duration_ms = Some(dur);
                    }
                } else {
                    self.track_duration_ms = self.current_track().and_then(|t| t.duration_ms);
                }
            }
            Command::PlaybackFinished => {
                self.is_playing = false;
                self.started_at = None;
                self.paused_at_ms = None;
                self.track_duration_ms = None;
            }
            Command::PlayError(e) => {
                self.is_playing = false;
                self.loading_play = false;
                self.download_progress = None;
                self.status_message = format!("Play error: {}", e);
            }
            Command::DownloadProgress(pct) => {
                self.loading_play = false;
                if pct <= 100 {
                    self.download_progress = Some(pct);
                } else {
                    self.download_progress = None;
                }
            }
            Command::DownloadLikedProgress { current, total } => {
                self.download_liked_progress = Some((current, total));
                self.status_message = format!("Downloading liked: {}/{}", current, total);
            }
            Command::DownloadLikedDone { downloaded, failed } => {
                self.download_liked_progress = None;
                self.status_message =
                    format!("Download complete: {} ok, {} failed", downloaded, failed);
            }
            Command::DownloadLikedTrack(_) => {}
            Command::DownloadLikedTrackDone { title, ok } => {
                if ok {
                    self.status_message = format!("Downloaded: {}", title);
                } else {
                    self.status_message = format!("Download failed: {}", title);
                }
            }
            Command::Status(msg) => {
                self.status_message = msg;
            }
            Command::FetchLyrics => {}
            Command::LyricsLoaded {
                track_id,
                lines,
                source,
            } => {
                if self
                    .current_track_id()
                    .map(|id| id == track_id)
                    .unwrap_or(false)
                {
                    self.lyrics_lines = lines;
                    self.lyrics_loading = false;
                    self.lyrics_error = None;
                    self.lyrics_source = Some(source);
                }
            }
            Command::LyricsError(e) => {
                self.lyrics_loading = false;
                if e == "lyrics not found" {
                    self.lyrics_error = Some("lyrics not found".to_string());
                } else {
                    self.lyrics_error = Some(format!("lyrics error: {}", e));
                }
            }
            Command::TrackMetadata {
                track_id,
                album,
                year,
            } => {
                if let Some(idx) = self
                    .tracks
                    .iter()
                    .position(|t| format!("{}:{}", t.source as u8, t.id) == track_id)
                {
                    let t = &mut self.tracks[idx];
                    if album.is_some() {
                        t.album = album;
                    }
                    if year.is_some() {
                        t.year = year;
                    }
                    if idx == self.current_index {
                        let album = t.album.clone().unwrap_or_default();
                        let year = t.year.map(|y| y.to_string()).unwrap_or_default();
                        self.status_message = if album.is_empty() && year.is_empty() {
                            "MB: no metadata".to_string()
                        } else {
                            format!("MB: {} {}", album, year).trim().to_string()
                        };
                    }
                }
            }
            Command::RadioFound(tracks) => {
                let count = tracks.len();
                self.radio_active = true;
                self.tracks.extend(tracks);
                self.status_message = if count > 0 {
                    format!("Radio: {} similar tracks appended", count)
                } else {
                    "Radio: no similar tracks found".to_string()
                };
            }
            Command::MetaUpdated {
                source,
                track_id,
                duration_ms,
                artwork_url,
            } => {
                let _ = self.db.update_liked_meta(
                    &source,
                    &track_id,
                    duration_ms,
                    artwork_url.as_deref(),
                );
                if let Some(idx) = self
                    .tracks
                    .iter()
                    .position(|t| t.source == source && t.id == track_id)
                {
                    if let Some(d) = duration_ms {
                        self.tracks[idx].duration_ms = Some(d);
                    }
                    if let Some(u) = &artwork_url {
                        self.tracks[idx].artwork_url = Some(u.clone());
                    }
                }
            }
            Command::BluetoothStatus(device) => {
                self.bluetooth_device = device;
            }
            Command::UpdateAvailable { tag, url } => {
                self.update_available = Some(url.clone());
                self.status_message =
                    format!("Update {} available — press U to open release page", tag);
            }
        }
    }

    pub fn take_update_url(&mut self) -> Option<String> {
        self.update_available.take()
    }
}

pub enum PlayAction {
    Pause,
    Resume,
    NewTrack,
}
