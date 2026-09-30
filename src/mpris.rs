use crate::dlog;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, RwLock};
use mpris_server::zbus::Result as ZResult;
use mpris_server::{
    Server, Metadata, Time, Volume, PlaybackStatus, LoopStatus, PlaybackRate,
    PlayerInterface, RootInterface, Property, Uri, TrackId,
};
use crate::api::Track;

#[derive(Debug, Clone)]
pub enum MprisCommand {
    PlayPause,
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
}

#[derive(Clone)]
struct MprisState {
    playback_status: PlaybackStatus,
    metadata: Metadata,
    volume: Volume,
    position: Time,
    started_at: Option<Instant>,
}

impl Default for MprisState {
    fn default() -> Self {
        Self {
            playback_status: PlaybackStatus::Stopped,
            metadata: Metadata::new(),
            volume: 1.0,
            position: Time::ZERO,
            started_at: None,
        }
    }
}

pub struct MprisServer {
    server: Option<Server<MprisHandler>>,
    state: Arc<RwLock<MprisState>>,
}

struct MprisHandler {
    state: Arc<RwLock<MprisState>>,
    cmd_tx: mpsc::UnboundedSender<MprisCommand>,
}

impl RootInterface for MprisHandler {
    async fn raise(&self) -> mpris_server::zbus::fdo::Result<()> { Ok(()) }
    async fn quit(&self) -> mpris_server::zbus::fdo::Result<()> { Ok(()) }
    async fn can_quit(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn fullscreen(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn set_fullscreen(&self, _: bool) -> ZResult<()> { Ok(()) }
    async fn can_set_fullscreen(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn can_raise(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn has_track_list(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn identity(&self) -> mpris_server::zbus::fdo::Result<String> { Ok("Music Player TUI".into()) }
    async fn desktop_entry(&self) -> mpris_server::zbus::fdo::Result<String> { Ok("larp-music-player".into()) }
    async fn supported_uri_schemes(&self) -> mpris_server::zbus::fdo::Result<Vec<String>> {
        Ok(vec!["http".into(), "https".into(), "file".into()])
    }
    async fn supported_mime_types(&self) -> mpris_server::zbus::fdo::Result<Vec<String>> {
        Ok(vec!["audio/mpeg".into(), "audio/x-flac".into(), "audio/ogg".into()])
    }
}

impl PlayerInterface for MprisHandler {
    async fn next(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Next);
        Ok(())
    }
    async fn previous(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Previous);
        Ok(())
    }
    async fn pause(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Pause);
        Ok(())
    }
    async fn play_pause(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::PlayPause);
        Ok(())
    }
    async fn stop(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Stop);
        Ok(())
    }
    async fn play(&self) -> mpris_server::zbus::fdo::Result<()> {
        let _ = self.cmd_tx.send(MprisCommand::Resume);
        Ok(())
    }
    async fn seek(&self, _offset: Time) -> mpris_server::zbus::fdo::Result<()> { Ok(()) }
    async fn set_position(&self, _track_id: TrackId, _position: Time) -> mpris_server::zbus::fdo::Result<()> { Ok(()) }
    async fn open_uri(&self, _uri: Uri) -> mpris_server::zbus::fdo::Result<()> { Ok(()) }

    async fn playback_status(&self) -> mpris_server::zbus::fdo::Result<PlaybackStatus> {
        Ok(self.state.read().await.playback_status)
    }
    async fn loop_status(&self) -> mpris_server::zbus::fdo::Result<LoopStatus> { Ok(LoopStatus::None) }
    async fn set_loop_status(&self, _: LoopStatus) -> ZResult<()> { Ok(()) }
    async fn rate(&self) -> mpris_server::zbus::fdo::Result<PlaybackRate> { Ok(1.0) }
    async fn set_rate(&self, _: PlaybackRate) -> ZResult<()> { Ok(()) }
    async fn shuffle(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(false) }
    async fn set_shuffle(&self, _: bool) -> ZResult<()> { Ok(()) }

    async fn metadata(&self) -> mpris_server::zbus::fdo::Result<Metadata> {
        Ok(self.state.read().await.metadata.clone())
    }

    async fn volume(&self) -> mpris_server::zbus::fdo::Result<Volume> {
        Ok(self.state.read().await.volume)
    }
    async fn set_volume(&self, volume: Volume) -> ZResult<()> {
        self.state.write().await.volume = volume;
        Ok(())
    }
    async fn position(&self) -> mpris_server::zbus::fdo::Result<Time> {
        let state = self.state.read().await;
        if state.playback_status == PlaybackStatus::Playing {
            if let Some(started) = state.started_at {
                return Ok(Time::from_millis(started.elapsed().as_millis() as i64));
            }
        }
        Ok(state.position)
    }
    async fn minimum_rate(&self) -> mpris_server::zbus::fdo::Result<PlaybackRate> { Ok(0.0) }
    async fn maximum_rate(&self) -> mpris_server::zbus::fdo::Result<PlaybackRate> { Ok(2.0) }
    async fn can_go_next(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
    async fn can_go_previous(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
    async fn can_play(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
    async fn can_pause(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
    async fn can_seek(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
    async fn can_control(&self) -> mpris_server::zbus::fdo::Result<bool> { Ok(true) }
}

impl MprisServer {
    pub async fn new(cmd_tx: mpsc::UnboundedSender<MprisCommand>) -> anyhow::Result<Self> {
        let state = Arc::new(RwLock::new(MprisState::default()));
        let handler = MprisHandler {
            state: state.clone(),
            cmd_tx,
        };
        match Server::new("music_player_tui", handler).await {
            Ok(server) => Ok(Self { server: Some(server), state }),
            Err(e) => {
                dlog!("MPRIS init failed: {}", e);
                Ok(Self { server: None, state })
            }
        }
    }

    pub async fn update_track(&self, track: &Track) {
        if let Some(ref server) = self.server {
            let mut mb = Metadata::builder()
                .title(&track.title)
                .artist([track.artist.as_str()]);

            if let Some(dur) = track.duration_ms {
                mb = mb.length(Time::from_millis(dur as i64));
            }
            if let Some(ref url) = track.artwork_url {
                mb = mb.art_url(url);
            }

            let metadata = mb.build();

            {
                let mut state = self.state.write().await;
                state.metadata = metadata.clone();
                state.position = Time::ZERO;
                state.started_at = Some(Instant::now());
            }

            let _ = server.properties_changed([
                Property::Metadata(metadata),
            ]).await;
        }
    }

    pub async fn set_playing(&self, playing: bool) {
        if let Some(ref server) = self.server {
            let status = if playing { PlaybackStatus::Playing } else { PlaybackStatus::Paused };
            {
                let mut state = self.state.write().await;
                state.playback_status = status;
                if playing {
                    state.started_at = Some(Instant::now());
                } else if let Some(started) = state.started_at {
                    state.position = Time::from_millis(started.elapsed().as_millis() as i64);
                    state.started_at = None;
                }
            }
            let _ = server.properties_changed([
                Property::PlaybackStatus(status),
            ]).await;
        }
    }

    pub async fn set_volume(&self, volume: f32) {
        if let Some(ref server) = self.server {
            {
                let mut state = self.state.write().await;
                state.volume = volume as f64;
            }
            let _ = server.properties_changed([
                Property::Volume(volume as f64),
            ]).await;
        }
    }
}
