use crate::dlog;
use std::path::PathBuf;
use std::process::{Command as FfmpegCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use rodio::{OutputStream, Sink, Source, buffer::SamplesBuffer};
use tokio::sync::mpsc;
use crate::app::Command as AppCommand;

static GENERATION: AtomicU64 = AtomicU64::new(0);

pub enum PlayerCommand {
    PlayFile(String, f32, u64),
    Pause,
    Resume,
    Stop,
    SetVolume(f32),
    SetSpeed(f32),
    Seek(f64),
}

fn cache_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".cache").join("music-player-tui")
}

fn cache_path(key: &str) -> PathBuf {
    let hash = format!("{:x}", md5::compute(key.as_bytes()));
    cache_dir().join(format!("{}.pcm", hash))
}

pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

fn is_stale(gen: u64) -> bool {
    gen != GENERATION.load(Ordering::SeqCst)
}

pub fn start(cmd_tx: mpsc::UnboundedSender<AppCommand>) -> (mpsc::UnboundedSender<PlayerCommand>, tokio::task::JoinHandle<()>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<PlayerCommand>();

    let handle = tokio::task::spawn_blocking(move || {
        let (_stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        sink.set_volume(0.7);
        let sink = Arc::new(sink);

        let _ = std::fs::create_dir_all(cache_dir());

        let mut playing = false;
        let mut current_samples: Option<Vec<i16>> = None;
        let mut current_amplify: f32 = 1.0;

        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    PlayerCommand::PlayFile(url_or_path, volume, generation) => {
                        if is_stale(generation) {
                            dlog!("[player] stale PlayFile (gen {}), discarding", generation);
                            continue;
                        }

                        sink.stop();

                        let pcm_key = &url_or_path;
                        let cached = cache_path(pcm_key);

                        let pcm_data = if cached.exists() && std::fs::metadata(&cached).map(|m| m.len() > 0).unwrap_or(false) {
                            std::fs::read(&cached).ok()
                        } else if url_or_path.contains(".m3u8") {
                            if is_stale(generation) { continue; }
                            dlog!("[player] HLS stream: {}", &url_or_path[..url_or_path.len().min(120)]);
                            match decode_with_ffmpeg_url(&url_or_path) {
                                Ok(samples) if !samples.is_empty() => {
                                    dlog!("[player] decoded {} samples from HLS", samples.len());
                                    let pcm = samples_to_bytes(&samples);
                                    let _ = std::fs::write(&cached, &pcm);
                                    Some(pcm)
                                }
                                _ => {
                                    let _ = cmd_tx.send(AppCommand::PlayError("HLS decode failed".to_string()));
                                    continue;
                                }
                            }
                        } else {
                            let raw = if url_or_path.starts_with("http") {
                                if is_stale(generation) { continue; }
                                dlog!("[player] downloading: {}", &url_or_path[..url_or_path.len().min(120)]);
                                let _ = cmd_tx.send(AppCommand::DownloadProgress(0));
                                match download_with_progress(&url_or_path, generation, &cmd_tx) {
                                    Ok(bytes) => {
                                        let _ = cmd_tx.send(AppCommand::DownloadProgress(100));
                                        Some(bytes)
                                    }
                                    Err(e) => {
                                        let _ = cmd_tx.send(AppCommand::PlayError(format!("curl error: {}", e)));
                                        continue;
                                    }
                                }
                            } else {
                                std::fs::read(&url_or_path).ok()
                            };

                            if is_stale(generation) { continue; }

                            let _ = cmd_tx.send(AppCommand::DownloadProgress(101));
                            match raw.and_then(|bytes| decode_with_ffmpeg(&bytes).ok()) {
                                Some(samples) if !samples.is_empty() => {
                                    dlog!("[player] decoded {} samples ({}ms)", samples.len(), samples.len() / 88);
                                    let pcm = samples_to_bytes(&samples);
                                    let _ = std::fs::write(&cached, &pcm);
                                    Some(pcm)
                                }
                                _ => {
                                    dlog!("[player] decode returned empty/failed");
                                    let _ = cmd_tx.send(AppCommand::PlayError("Decode failed".to_string()));
                                    continue;
                                }
                            }
                        };

                        if is_stale(generation) { continue; }

                        match pcm_data {
                            Some(pcm) => {
                                dlog!("[player] playing {} bytes of PCM", pcm.len());
                                let samples = bytes_to_samples(&pcm);
                                current_samples = Some(samples.clone());
                                current_amplify = volume;
                                let source = SamplesBuffer::new(2, 44100, samples)
                                    .amplify(volume);
                                sink.append(source);
                                sink.play();
                                playing = true;
                                let actual_duration_ms = Some((pcm.len() as u64 * 1000) / (4 * 44100));
                                let _ = cmd_tx.send(AppCommand::PlayStarted { actual_duration_ms });
                            }
                            None => {
                                let _ = cmd_tx.send(AppCommand::PlayError("No audio data".to_string()));
                            }
                        }
                    }
                    PlayerCommand::Pause => {
                        sink.pause();
                        playing = false;
                    }
                    PlayerCommand::Resume => {
                        sink.play();
                        playing = true;
                    }
                    PlayerCommand::Stop => {
                        sink.stop();
                        break;
                    }
                    PlayerCommand::SetVolume(vol) => {
                        sink.set_volume(vol);
                    }
                    PlayerCommand::SetSpeed(speed) => {
                        sink.set_speed(speed);
                    }
                    PlayerCommand::Seek(secs) => {
                        if let Some(ref samples) = current_samples {
                            let start = ((secs * 44100.0) as usize) * 2;
                            if start < samples.len() {
                                dlog!("[player] seek to {:.1}s", secs);
                                let was_playing = playing;
                                sink.stop();
                                let vol = sink.volume();
                                let source = SamplesBuffer::new(2, 44100, samples[start..].to_vec())
                                    .amplify(current_amplify);
                                sink.set_volume(vol);
                                sink.append(source);
                                if was_playing {
                                    sink.play();
                                } else {
                                    sink.pause();
                                }
                                playing = was_playing;
                            }
                        }
                    }
                },
                Err(mpsc::error::TryRecvError::Empty) => {
                    if playing && sink.empty() {
                        playing = false;
                        current_samples = None;
                        let _ = cmd_tx.send(AppCommand::PlaybackFinished);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
    });

    (tx, handle)
}

fn download_with_progress(url: &str, generation: u64, cmd_tx: &mpsc::UnboundedSender<AppCommand>) -> Result<Vec<u8>, String> {
    use std::io::Read;

    let mut child = std::process::Command::new("curl")
        .args(["-sL", "--max-time", "60", "-w", "\n%{size_download}\n%{speed_download}"])
        .args(["-H", "User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36"])
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {}", e))?;

    let mut data = Vec::new();
    let mut bytes_read: u64 = 0;

    if let Some(ref mut stdout) = child.stdout {
        let mut buf = [0u8; 65536];
        loop {
            if is_stale(generation) {
                let _ = child.kill();
                return Err("cancelled".to_string());
            }

            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    data.extend_from_slice(&buf[..n]);
                    bytes_read += n as u64;
                    let pct = if bytes_read > 0 { ((bytes_read as f64 / 1_000_000.0 * 100.0).min(99.0)) as u32 } else { 0 };
                    let _ = cmd_tx.send(AppCommand::DownloadProgress(pct));
                }
                Err(_) => break,
            }
        }
    }

    let status = child.wait().map_err(|e| format!("wait: {}", e))?;
    if !status.success() {
        return Err(format!("curl exited {}", status));
    }
    Ok(data)
}

fn samples_to_bytes(samples: &[i16]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

fn bytes_to_samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn decode_with_ffmpeg(input: &[u8]) -> anyhow::Result<Vec<i16>> {
    let input = input.to_vec();

    let mut child = FfmpegCommand::new("ffmpeg")
        .args([
            "-i", "pipe:0",
            "-f", "s16le",
            "-ar", "44100",
            "-ac", "2",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(&input);
        });
    }

    let output = child.wait_with_output()?;
    let raw = output.stdout;
    let samples: Vec<i16> = raw
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    Ok(samples)
}

fn decode_with_ffmpeg_url(url: &str) -> anyhow::Result<Vec<i16>> {
    let output = std::process::Command::new("ffmpeg")
        .args(["-i", url, "-f", "s16le", "-ar", "44100", "-ac", "2", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()?;

    let raw = output.stdout;
    let samples: Vec<i16> = raw
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    Ok(samples)
}
