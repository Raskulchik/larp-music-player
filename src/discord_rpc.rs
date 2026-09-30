use crate::dlog;
use discord_rich_presence::{activity, DiscordIpc, DiscordIpcClient};
use std::sync::mpsc;
use std::thread;

#[derive(Clone)]
pub enum RpcCommand {
    SetActivity {
        title: String,
        artist: String,
        artwork_url: Option<String>,
        duration_ms: Option<u64>,
        state: String,
        /// Current playback position in ms (if the track is already in progress).
        /// The RPC start timestamp is back-dated so Discord's timer stays in sync.
        position_ms: Option<u64>,
        /// Whether the Discord progress bar / timer should be shown.
        show_timer: bool,
    },
    SetState {
        state: String,
        position_ms: Option<u64>,
        show_timer: bool,
    },
}

pub struct DiscordRpc {
    cmd_tx: mpsc::Sender<RpcCommand>,
}

impl DiscordRpc {
    pub fn new(client_id: &str) -> Option<Self> {
        let client_id = client_id.to_string();
        let (cmd_tx, cmd_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            run_rpc_loop(&client_id, cmd_rx);
        });

        // Don't hold handle — thread runs until Shutdown
        std::mem::forget(handle);

        Some(Self { cmd_tx })
    }

    pub fn set_activity(&self, title: &str, artist: &str, _source: &str, artwork_url: Option<&str>, duration_ms: Option<u64>, state: &str) {
        let _ = self.cmd_tx.send(RpcCommand::SetActivity {
            title: title.to_string(),
            artist: artist.to_string(),
            artwork_url: artwork_url.map(|s| s.to_string()),
            duration_ms,
            state: state.to_string(),
            position_ms: None,
            show_timer: true,
        });
    }

    pub fn set_state(&self, state: &str, position_ms: Option<u64>, show_timer: bool) {
        let _ = self.cmd_tx.send(RpcCommand::SetState {
            state: state.to_string(),
            position_ms,
            show_timer,
        });
    }
}

fn run_rpc_loop(client_id: &str, cmd_rx: mpsc::Receiver<RpcCommand>) {
    let mut client = DiscordIpcClient::new(client_id);

    let mut connected = match client.connect() {
        Ok(()) => {
            dlog!("[discord] connected");
            true
        }
        Err(e) => {
            dlog!("[discord] connect failed (will retry): {}", e);
            false
        }
    };

    let mut pending_activity: Option<RpcCommand> = None;

    loop {
        // If disconnected, try reconnecting every 5 seconds
        if !connected {
            thread::sleep(std::time::Duration::from_secs(5));
            match client.connect() {
                Ok(()) => {
                    dlog!("[discord] reconnected");
                    connected = true;
                    // Re-send last activity if we had one
                    if let Some(cmd) = pending_activity.take() {
                        let _ = apply_command(&mut client, &cmd);
                    }
                    continue;
                }
                Err(e) => {
                    dlog!("[discord] reconnect failed: {}", e);
                    continue;
                }
            }
        }

        match cmd_rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(cmd) => {
                let apply: Option<RpcCommand> = match &cmd {
                    RpcCommand::SetState { state, position_ms, show_timer } => match &pending_activity {
                        Some(RpcCommand::SetActivity { title, artist, artwork_url, duration_ms, .. }) => {
                            let upd = RpcCommand::SetActivity {
                                title: title.clone(),
                                artist: artist.clone(),
                                artwork_url: artwork_url.clone(),
                                duration_ms: *duration_ms,
                                state: state.clone(),
                                position_ms: *position_ms,
                                show_timer: *show_timer,
                            };
                            pending_activity = Some(upd.clone());
                            Some(upd)
                        }
                        _ => None,
                    },
                    _ => {
                        pending_activity = Some(cmd.clone());
                        Some(cmd)
                    }
                };
                if let Some(cmd) = apply {
                    if let Err(e) = apply_command(&mut client, &cmd) {
                        dlog!("[discord] command failed: {}", e);
                        connected = false;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn apply_command(client: &mut DiscordIpcClient, cmd: &RpcCommand) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        RpcCommand::SetActivity { title, artist, artwork_url, duration_ms, state, position_ms, show_timer } => {
            let mut assets = activity::Assets::new();
            if let Some(url) = artwork_url {
                assets = assets.large_image(url);
            }

            let mut activity = activity::Activity::new()
                .state(artist)
                .details(format!("{} • {}", state, title))
                .assets(assets)
                .activity_type(activity::ActivityType::Listening);

            if *show_timer {
                if let Some(dur) = duration_ms {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64;
                    let start = now - position_ms.unwrap_or(0).min(*dur) as i64;
                    let end = start + *dur as i64;
                    activity = activity.timestamps(activity::Timestamps::new().start(start).end(end));
                }
            }

            client.set_activity(activity)?;
        }
        RpcCommand::SetState { .. } => {}
    }
    Ok(())
}
