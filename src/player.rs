use crate::app::Command as AppCommand;
use crate::dlog;
use rodio::{buffer::SamplesBuffer, OutputStream, Sink, Source};
use std::path::PathBuf;
use std::process::{Command as FfmpegCommand, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;

static GENERATION: AtomicU64 = AtomicU64::new(0);

pub enum PlayerCommand {
    /// `path_or_url`, volume, generation, keep a decoded PCM copy in the cache.
    ///
    /// Files that already live on disk are decoded on the fly: caching them
    /// would only duplicate the audio.
    PlayFile(String, f32, u64, bool),
    Pause,
    Resume,
    Stop,
    SetVolume(f32),
    SetSpeed(f32),
    Seek(f64),
}

/// Where decoded audio and downloads are kept (`~/.local/share/music-player-tui`).
fn cache_dir() -> PathBuf {
    crate::paths::data_dir()
}

fn cache_path(key: &str) -> PathBuf {
    let hash = format!("{:x}", md5::compute(key.as_bytes()));
    cache_dir().join(format!("{}.pcm", hash))
}

/// Upper bound for the decoded-PCM cache. The oldest entries are dropped
/// first, so the cache cannot fill up the disk.
const PCM_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// The limit in use, overridable with `LARP_PCM_CACHE_MAX_MB`.
fn pcm_cache_max_bytes() -> u64 {
    if let Ok(v) = std::env::var("LARP_PCM_CACHE_MAX_MB") {
        if let Ok(mb) = v.parse::<u64>() {
            return mb * 1024 * 1024;
        }
    }
    PCM_CACHE_MAX_BYTES
}

/// Remove the oldest cached PCM files until the cache fits into `max_bytes`.
fn prune_pcm_cache_in(dir: &std::path::Path, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pcm") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let len = meta.len();
        total += len;
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        files.push((mtime, len, path));
    }
    if total <= max_bytes {
        return;
    }
    files.sort_by_key(|(mtime, _, _)| *mtime);
    let mut excess = total - max_bytes;
    for (_, len, path) in files {
        if excess == 0 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            excess = excess.saturating_sub(len);
        }
    }
}

/// Keep the PCM cache under its limit.
pub fn prune_pcm_cache() {
    prune_pcm_cache_in(&cache_dir(), pcm_cache_max_bytes());
}

pub fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

fn is_stale(gen: u64) -> bool {
    gen != GENERATION.load(Ordering::SeqCst)
}

pub fn start(
    cmd_tx: mpsc::UnboundedSender<AppCommand>,
) -> (
    mpsc::UnboundedSender<PlayerCommand>,
    tokio::task::JoinHandle<()>,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<PlayerCommand>();

    let handle = tokio::task::spawn_blocking(move || {
        let (_stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();
        sink.set_volume(0.7);
        let sink = Arc::new(sink);

        let _ = std::fs::create_dir_all(cache_dir());
        // Drop leftovers that exceed the cache limit (e.g. from older versions).
        prune_pcm_cache();

        let mut playing = false;
        let mut current_samples: Option<Vec<i16>> = None;
        let mut current_amplify: f32 = 1.0;

        loop {
            match rx.try_recv() {
                Ok(cmd) => match cmd {
                    PlayerCommand::PlayFile(url_or_path, volume, generation, cache_pcm) => {
                        if is_stale(generation) {
                            dlog!("[player] stale PlayFile (gen {}), discarding", generation);
                            continue;
                        }

                        sink.stop();

                        let pcm_key = &url_or_path;
                        let cached = cache_path(pcm_key);
                        // Files that are already on disk are decoded on the
                        // fly instead of keeping a second PCM copy.
                        let local = !cache_pcm;

                        let pcm_data = if !local
                            && cached.exists()
                            && std::fs::metadata(&cached)
                                .map(|m| m.len() > 0)
                                .unwrap_or(false)
                        {
                            std::fs::read(&cached).ok()
                        } else if url_or_path.contains(".m3u8") {
                            if is_stale(generation) {
                                continue;
                            }
                            dlog!(
                                "[player] HLS stream: {}",
                                &url_or_path[..url_or_path.len().min(120)]
                            );
                            match decode_with_ffmpeg_url(&url_or_path) {
                                Ok(samples) if !samples.is_empty() => {
                                    dlog!("[player] decoded {} samples from HLS", samples.len());
                                    let pcm = samples_to_bytes(&samples);
                                    if !local {
                                        let _ = std::fs::write(&cached, &pcm);
                                        prune_pcm_cache();
                                    }
                                    Some(pcm)
                                }
                                _ => {
                                    let _ = cmd_tx.send(AppCommand::PlayError(
                                        "HLS decode failed".to_string(),
                                    ));
                                    continue;
                                }
                            }
                        } else {
                            let raw = if url_or_path.starts_with("http") {
                                if is_stale(generation) {
                                    continue;
                                }
                                dlog!(
                                    "[player] downloading: {}",
                                    &url_or_path[..url_or_path.len().min(120)]
                                );
                                let _ = cmd_tx.send(AppCommand::DownloadProgress(0));
                                match download_with_progress(&url_or_path, generation, &cmd_tx) {
                                    Ok(bytes) => {
                                        let _ = cmd_tx.send(AppCommand::DownloadProgress(100));
                                        Some(bytes)
                                    }
                                    Err(e) => {
                                        let _ = cmd_tx.send(AppCommand::PlayError(format!(
                                            "curl error: {}",
                                            e
                                        )));
                                        continue;
                                    }
                                }
                            } else {
                                std::fs::read(&url_or_path).ok()
                            };

                            if is_stale(generation) {
                                continue;
                            }

                            let _ = cmd_tx.send(AppCommand::DownloadProgress(101));
                            match raw.and_then(|bytes| decode_with_ffmpeg(&bytes).ok()) {
                                Some(samples) if !samples.is_empty() => {
                                    dlog!(
                                        "[player] decoded {} samples ({}ms)",
                                        samples.len(),
                                        samples.len() / 88
                                    );
                                    let pcm = samples_to_bytes(&samples);
                                    if !local {
                                        let _ = std::fs::write(&cached, &pcm);
                                        prune_pcm_cache();
                                    }
                                    Some(pcm)
                                }
                                _ => {
                                    dlog!("[player] decode returned empty/failed");
                                    let _ = cmd_tx
                                        .send(AppCommand::PlayError("Decode failed".to_string()));
                                    continue;
                                }
                            }
                        };

                        if is_stale(generation) {
                            continue;
                        }

                        match pcm_data {
                            Some(pcm) => {
                                dlog!("[player] playing {} bytes of PCM", pcm.len());
                                let samples = bytes_to_samples(&pcm);
                                current_samples = Some(samples.clone());
                                current_amplify = volume;
                                let source = SamplesBuffer::new(2, 44100, samples).amplify(volume);
                                sink.append(source);
                                sink.play();
                                playing = true;
                                let actual_duration_ms =
                                    Some((pcm.len() as u64 * 1000) / (4 * 44100));
                                let _ = cmd_tx.send(AppCommand::PlayStarted { actual_duration_ms });
                            }
                            None => {
                                let _ =
                                    cmd_tx.send(AppCommand::PlayError("No audio data".to_string()));
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
                                let source =
                                    SamplesBuffer::new(2, 44100, samples[start..].to_vec())
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

fn download_with_progress(
    url: &str,
    generation: u64,
    cmd_tx: &mpsc::UnboundedSender<AppCommand>,
) -> Result<Vec<u8>, String> {
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
                    let pct = if bytes_read > 0 {
                        ((bytes_read as f64 / 1_000_000.0 * 100.0).min(99.0)) as u32
                    } else {
                        0
                    };
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
            "-i", "pipe:0", "-f", "s16le", "-ar", "44100", "-ac", "2", "pipe:1",
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
        .args([
            "-i", url, "-f", "s16le", "-ar", "44100", "-ac", "2", "pipe:1",
        ])
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

#[cfg(test)]
mod tests {
    use super::prune_pcm_cache_in;
    use std::path::PathBuf;

    fn tmp_dir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "larp_pcm_{}_{}_{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn prune_keeps_cache_within_the_limit() {
        let dir = tmp_dir("prune");
        // Three files of 1 KiB each, written oldest first.
        for i in 0..3u64 {
            let path = dir.join(format!("{}.pcm", i));
            std::fs::write(&path, vec![0u8; 1024]).unwrap();
            // Make the order of the files unambiguous.
            let time = std::time::SystemTime::now() - std::time::Duration::from_secs(100 - i);
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_times(std::fs::FileTimes::new().set_modified(time))
                .unwrap();
        }
        std::fs::write(dir.join("keep.txt"), b"x").unwrap();

        prune_pcm_cache_in(&dir, 2048);
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.contains(&"keep.txt".to_string()), "other files stay");
        let pcm_left = left.iter().filter(|n| n.ends_with(".pcm")).count();
        assert!(
            pcm_left <= 2,
            "cache must shrink to the limit, left {}",
            pcm_left
        );
        // The newest file must survive, the oldest must go.
        assert!(left.contains(&"2.pcm".to_string()));
        assert!(!left.contains(&"0.pcm".to_string()));
    }

    #[test]
    fn prune_keeps_everything_below_the_limit() {
        let dir = tmp_dir("prune_small");
        std::fs::write(dir.join("a.pcm"), vec![0u8; 100]).unwrap();
        prune_pcm_cache_in(&dir, 10_000);
        assert!(dir.join("a.pcm").exists(), "small cache is untouched");
    }
}
