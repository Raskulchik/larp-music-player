use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    YandexMusic,
    ITunes,
    SoundCloud,
    YouTubeMusic,
}

#[derive(Debug, Clone)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub source: Source,
    pub preview_url: Option<String>,
    pub artwork_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub album: Option<String>,
    pub year: Option<u16>,
}

// ========== iTunes ==========

#[derive(Debug, Deserialize)]
struct ItunesResponse {
    results: Vec<ItunesTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItunesTrack {
    track_id: i64,
    track_name: String,
    artist_name: String,
    #[serde(default)]
    preview_url: Option<String>,
    #[serde(default)]
    artwork_url_100: Option<String>,
    #[serde(default)]
    track_time_millis: Option<u64>,
}

pub async fn search_itunes(query: &str) -> anyhow::Result<Vec<Track>> {
    let url = format!(
        "https://itunes.apple.com/search?term={}&media=music&limit=25",
        urlencoding::encode(query)
    );

    let resp = reqwest::get(&url).await?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("iTunes HTTP {}: {}", status, body);
    }

    let text = resp.text().await?;
    let resp: ItunesResponse = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("iTunes parse error: {} | body: {}", e, &text[..text.len().min(500)]))?;

    let tracks = resp
        .results
        .into_iter()
        .filter(|t| t.preview_url.is_some())
        .map(|t| Track {
            id: t.track_id.to_string(),
            title: t.track_name,
            artist: t.artist_name,
            source: Source::ITunes,
            preview_url: t.preview_url,
            artwork_url: t.artwork_url_100,
            duration_ms: t.track_time_millis,
            album: None,
            year: None,
        })
        .collect();

    Ok(tracks)
}

// ========== Yandex Music ==========

const YM_API: &str = "https://api.music.yandex.net";
const YM_SIGN_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmSearchResponse {
    result: YmSearchResult,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmSearchResult {
    tracks: Option<YmSearchTracks>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmSearchTracks {
    results: Vec<YmTrack>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmTrack {
    id: serde_json::Value,
    title: Option<String>,
    #[serde(default)]
    artists: Vec<YmArtist>,
    duration_ms: Option<u64>,
    #[serde(default)]
    available: Option<bool>,
    #[serde(default)]
    cover_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmArtist {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmDownloadInfoResponse {
    result: Vec<YmDownloadInfo>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YmDownloadInfo {
    codec: String,
    download_info_url: String,
}

#[derive(Debug, Deserialize)]
struct YmDownloadUrl {
    host: String,
    path: String,
    ts: String,
    s: String,
}

pub async fn search_yandex(query: &str, token: &str) -> anyhow::Result<Vec<Track>> {
    let clean_token: String = token.chars().take_while(|&c| c != '&').collect();

    let url = format!(
        "{}/search/?text={}&type=track&page=0&nocorrect=false",
        YM_API,
        urlencoding::encode(query)
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("OAuth {}", clean_token))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Yandex HTTP {}: {}", status, body);
    }

    let text = resp.text().await?;
    let resp: YmSearchResponse = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("Yandex parse error: {} | body: {}", e, &text[..text.len().min(500)]))?;

    let tracks = resp
        .result
        .tracks
        .map(|t| t.results)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| t.available.unwrap_or(false))
        .map(|t| {
            let id = match &t.id {
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => s.clone(),
                _ => "0".to_string(),
            };
            let artist = t.artists.first()
                .and_then(|a| a.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());
            let duration = t.duration_ms;
            let artwork_url = t.cover_uri.map(|uri| {
                format!("https://{}", uri.replace("%%", "1000x1000"))
            });
            Track {
                id,
                title: t.title.unwrap_or_default(),
                artist,
                source: Source::YandexMusic,
                preview_url: None,
                artwork_url,
                duration_ms: duration,
                album: None,
                year: None,
            }
        })
        .collect();

    Ok(tracks)
}

pub async fn get_yandex_download_url(track_id: &str, token: &str) -> anyhow::Result<String> {
    let clean_token: String = token.chars().take_while(|&c| c != '&').collect();
    let url = format!("{}/tracks/{}/download-info", YM_API, track_id);

    let resp = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("OAuth {}", clean_token))
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Yandex download HTTP {}: {}", status, body);
    }

    let text = resp.text().await?;
    let resp: YmDownloadInfoResponse = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("Yandex download parse error: {} | body: {}", e, &text[..text.len().min(500)]))?;

    let info = resp.result.iter()
        .find(|i| i.codec == "mp3" || i.codec == "flac")
        .or(resp.result.first())
        .ok_or_else(|| anyhow::anyhow!("No download info found"))?;

    let xml_url = &info.download_info_url;
    let xml_text = reqwest::get(xml_url).await?.text().await?;

    let download_url: YmDownloadUrl = serde_xml_rs::from_str(&xml_text)?;

    let path_no_slash = download_url.path.strip_prefix('/').unwrap_or(&download_url.path);
    let sign_input = format!("{}{}{}", YM_SIGN_SALT, path_no_slash, download_url.s);
    let sign = format!("{:x}", md5::compute(sign_input.as_bytes()));

    let direct_url = format!(
        "https://{}/get-mp3/{}/{}/{}",
        download_url.host, sign, download_url.ts, path_no_slash
    );

    Ok(direct_url)
}

// ========== SoundCloud ==========

const SC_API: &str = "https://api-v2.soundcloud.com";

static SC_CLIENT_ID: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn curl_get(url: &str) -> anyhow::Result<String> {
    let output = std::process::Command::new("curl")
        .args(["-s", "--max-time", "15"])
        .args(["-H", "User-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/137.0.0.0 Safari/537.36"])
        .args(["-H", "Accept: application/json, text/plain, */*"])
        .args(["-H", "Accept-Language: en-US,en;q=0.9"])
        .args(["-H", "Origin: https://soundcloud.com"])
        .args(["-H", "Referer: https://soundcloud.com/"])
        .arg(url)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("curl failed with {}", output.status);
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn set_sc_client_id(id: &str) {
    if !id.is_empty() {
        *SC_CLIENT_ID.lock().unwrap() = Some(id.to_string());
    }
}

async fn get_sc_client_id() -> anyhow::Result<String> {
    if let Some(id) = SC_CLIENT_ID.lock().unwrap().as_ref() {
        return Ok(id.clone());
    }

    let html = tokio::task::spawn_blocking(|| -> anyhow::Result<String> {
        curl_get("https://soundcloud.com/")
    }).await??;

    let script_re = regex::Regex::new(r#"src="(https://a-v2\.sndcdn\.com/assets/[^"]+\.js)""#)?;
    let cid_re = regex::Regex::new(r#"client_id:"([a-zA-Z0-9]+)""#)?;

    let mut script_urls: Vec<String> = script_re.captures_iter(&html)
        .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
        .collect();
    script_urls.reverse();

    for js_url in &script_urls {
        let js_text = match tokio::task::spawn_blocking({
            let url = js_url.clone();
            move || curl_get(&url)
        }).await {
            Ok(Ok(text)) => text,
            _ => continue,
        };
        if let Some(m) = cid_re.captures(&js_text) {
            let id = m[1].to_string();
            *SC_CLIENT_ID.lock().unwrap() = Some(id.clone());
            return Ok(id);
        }
    }

    anyhow::bail!("Could not extract SoundCloud client_id from JS bundles")
}

#[derive(Debug, Deserialize)]
struct ScSearchResponse {
    collection: Vec<ScTrack>,
}

#[derive(Debug, Deserialize)]
struct ScTrack {
    id: i64,
    title: String,
    duration: u64,
    #[serde(default)]
    artwork_url: Option<String>,
    user: ScUser,
    #[serde(default)]
    media: Option<ScMedia>,
}

#[derive(Debug, Deserialize)]
struct ScUser {
    username: String,
}

#[derive(Debug, Deserialize)]
struct ScMedia {
    #[serde(default)]
    transcodings: Vec<ScTranscoding>,
}

#[derive(Debug, Deserialize)]
struct ScTranscoding {
    url: String,
    format: ScFormat,
}

#[derive(Debug, Deserialize)]
struct ScFormat {
    protocol: String,
    mime_type: String,
}

pub async fn search_soundcloud(query: &str) -> anyhow::Result<Vec<Track>> {
    let client_id = get_sc_client_id().await?;

    let url = format!(
        "{}/search/tracks?q={}&client_id={}&limit=20",
        SC_API,
        urlencoding::encode(query),
        urlencoding::encode(&client_id)
    );

    let body = tokio::task::spawn_blocking({
        let url = url.clone();
        move || curl_get(&url)
    }).await??;

    if body.trim() == "{}" || body.trim().is_empty() {
        SC_CLIENT_ID.lock().unwrap().take();
        anyhow::bail!("SoundCloud returned empty — client_id may be expired. Get a fresh one: SoundCloud → DevTools → Network → copy client_id from any api-v2 request URL");
    }

    let resp: ScSearchResponse = serde_json::from_str(&body)?;

    let tracks = resp.collection.into_iter().map(|t| {
        let artwork = t.artwork_url.map(|u| u.replace("-large", "-t500x500"));

        let mut preview_url = None;
        if let Some(ref media) = t.media {
            // cbc-encrypted-hls works — ffmpeg handles HLS natively
            // prefer audio/mp4 (returns m3u8 URL), not audio/mpegurl (returns {})
            if let Some(tc) = media.transcodings.iter().find(|tc| tc.format.protocol == "cbc-encrypted-hls" && tc.format.mime_type.starts_with("audio/mp4")) {
                preview_url = Some(tc.url.clone());
            }
            if preview_url.is_none() {
                if let Some(tc) = media.transcodings.iter().find(|tc| tc.format.protocol == "hls") {
                    preview_url = Some(tc.url.clone());
                }
            }
            // progressive is blocked by SoundCloud — skip it
        }

        Track {
            id: t.id.to_string(),
            title: t.title,
            artist: t.user.username,
            source: Source::SoundCloud,
            preview_url,
            artwork_url: artwork,
            duration_ms: Some(t.duration),
            album: None,
            year: None,
        }
    }).collect();

    Ok(tracks)
}

// ========== YouTube Music ==========

fn ytm_client() -> reqwest::Client {
    reqwest::Client::new()
}

pub async fn search_ytmusic(query: &str) -> anyhow::Result<Vec<Track>> {
    let body = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB_REMIX",
                "clientVersion": "1.20240311.01.00",
                "hl": "en"
            }
        },
        "query": query,
    });

    let url = "https://music.youtube.com/youtubei/v1/search?key=AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";

    let resp = ytm_client()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Origin", "https://music.youtube.com")
        .header("Referer", "https://music.youtube.com/")
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("YouTube Music HTTP {}: {}", status, resp.text().await.unwrap_or_default());
    }

    let text = resp.text().await?;
    let root: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| anyhow::anyhow!("YouTube Music parse error: {} | body: {}", e, &text[..text.len().min(500)]))?;

    let mut tracks = Vec::new();
    collect_ytm_tracks(&root, &mut tracks);

    Ok(tracks)
}

fn collect_ytm_tracks(value: &serde_json::Value, out: &mut Vec<Track>) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "musicResponsiveListItemRenderer" {
                    if let Some(track) = ytm_item_to_track(v) {
                        out.push(track);
                    }
                } else {
                    collect_ytm_tracks(v, out);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                collect_ytm_tracks(v, out);
            }
        }
        _ => {}
    }
}

fn ytm_item_to_track(item: &serde_json::Value) -> Option<Track> {
    let video_id = item.get("playlistItemData")
        .and_then(|p| p.get("videoId"))
        .and_then(|v| v.as_str())
        .or_else(|| item.get("navigationEndpoint")
            .and_then(|n| n.get("watchEndpoint"))
            .and_then(|w| w.get("videoId"))
            .and_then(|v| v.as_str()))
        .map(|s| s.to_string())?;

    let columns = item.get("flexColumns")?.as_array()?;
    let title = columns.get(0)?
        .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
        .and_then(|r| r.as_array())?
        .first()?
        .get("text")?
        .as_str()?
        .to_string();

    let subtitle_runs: Vec<String> = columns.get(1)
        .and_then(|col| col.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs"))
        .and_then(|r| r.as_array())
        .map(|runs| runs.iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .collect())
        .unwrap_or_default();

    let is_typed = subtitle_runs.first().map_or(false, |s| {
        matches!(s.as_str(), "Song" | "Video" | "Artist" | "Album" | "Single" | "Episode")
    });
    let artist_runs: Box<dyn Iterator<Item = &String>> = if is_typed {
        Box::new(subtitle_runs.iter().skip(1))
    } else {
        Box::new(subtitle_runs.iter())
    };

    let artist = artist_runs
        .map(|s| s.trim())
        .filter(|s| !s.is_empty() && *s != "•" && *s != "," && *s != "&"
            && !s.contains("views") && !s.contains("subscribers") && !s.contains("years ago")
            && !s.chars().all(|c| c.is_ascii_digit() || c == ':' || c == ' '))
        .collect::<Vec<_>>()
        .join(" ");
    let artist = if artist.is_empty() { "Unknown".to_string() } else { artist };

    let duration_ms = subtitle_runs.iter()
        .rev()
        .find_map(|s| {
            let s = s.trim();
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() == 2 {
                let min: u64 = parts[0].parse().ok()?;
                let sec: u64 = parts[1].parse().ok()?;
                Some((min * 60 + sec) * 1000)
            } else {
                None
            }
        });

    let artwork_url = item.get("thumbnail")
        .and_then(|t| t.get("musicThumbnailRenderer"))
        .and_then(|t| t.get("thumbnail"))
        .and_then(|t| t.get("thumbnails"))
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(|s| yt_big_thumbnail(s));

    Some(Track {
        id: video_id,
        title,
        artist,
        source: Source::YouTubeMusic,
        preview_url: None,
        artwork_url,
        duration_ms,
        album: None,
        year: None,
    })
}

// ========== YouTube Music metadata via ytmusicapi (Python) ==========

#[derive(Debug, Clone)]
pub struct YtMeta {
    pub duration_ms: Option<u64>,
    pub artwork_url: Option<String>,
}

fn ytmeta_script() -> Option<std::path::PathBuf> {
    let candidates = [
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ytmeta.py"),
        std::env::current_exe().ok()?.parent()?.join("ytmeta.py"),
        std::path::PathBuf::from("/usr/share/larp-music-player/ytmeta.py"),
        std::path::PathBuf::from(env!("HOME")).join(".config/music-player-tui/ytmeta.py"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

/// Fetch accurate duration + big artwork for a YouTube Music video via ytmusicapi.
pub async fn get_ytmeta(video_id: &str) -> Option<YtMeta> {
    let id = video_id.to_string();
    let script = ytmeta_script()?;
    tokio::task::spawn_blocking(move || {
        let mut cmd = std::process::Command::new("python3");
        let venv = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".venv/bin/python");
        if venv.exists() {
            cmd = std::process::Command::new(venv);
        }
        let output = cmd
            .arg(&script)
            .arg("song")
            .arg(&id)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout);
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        if v.get("ok").and_then(|o| o.as_bool()) != Some(true) {
            return None;
        }
        Some(YtMeta {
            duration_ms: v.get("duration_ms").and_then(|d| d.as_u64()).filter(|d| *d > 0),
            artwork_url: v.get("artwork_url").and_then(|a| a.as_str()).map(|s| s.to_string()),
        })
    })
    .await
    .ok()
    .flatten()
}

/// Rewrite a YT thumbnail URL size (`=w###-h###`) to w1080-h1080 for bigger artwork.
pub fn yt_big_thumbnail(url: &str) -> String {
    let re = regex::Regex::new(r"=w\d+-h\d+").unwrap();
    re.replace(url, "=w1080-h1080").to_string()
}

// ========== MusicBrainz ==========

#[derive(Debug, Clone)]
pub struct MbMeta {
    pub album: Option<String>,
    pub year: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct MbResponse {
    #[serde(default)]
    recordings: Vec<MbRecording>,
}

#[derive(Debug, Deserialize)]
struct MbRecording {
    #[serde(default)]
    releases: Vec<MbRelease>,
}

#[derive(Debug, Deserialize)]
struct MbRelease {
    #[serde(default)]
    title: String,
    #[serde(default)]
    date: String,
}

pub async fn get_mb_metadata(artist: &str, title: &str) -> Result<Option<MbMeta>, String> {
    // MusicBrainz rate limit: 1 request/sec — enforce with a plain sleep
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let query = format!(
        "recording:\"{}\" AND artist:\"{}\"",
        title.replace('"', ""),
        artist.replace('"', "")
    );
    let url = format!(
        "https://musicbrainz.org/ws/2/recording/?query={}&fmt=json&limit=3",
        urlencoding::encode(&query)
    );

    // MusicBrainz requires a valid User-Agent, otherwise 403
    let resp = reqwest::Client::builder()
        .user_agent("larp-music-player/0.1 ( https://github.com/anomalyco/opencode )")
        .build()
        .map_err(|e| e.to_string())?
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("musicbrainz status {}", resp.status()));
    }

    let body: MbResponse = resp.json().await.map_err(|e| e.to_string())?;
    let rec = match body.recordings.first() {
        Some(r) => r,
        None => return Ok(None),
    };
    let release = rec.releases.first();

    let album = release.and_then(|r| {
        if r.title.is_empty() { None } else { Some(r.title.clone()) }
    });
    let year = release.and_then(|r| {
        r.date.chars().take(4).collect::<String>().parse::<u16>().ok()
    });

    if album.is_none() && year.is_none() {
        return Ok(None);
    }
    Ok(Some(MbMeta { album, year }))
}

// ========== Last.fm ==========

#[derive(Debug, Deserialize)]
struct LastfmSimilarResponse {
    #[serde(default)]
    similartracks: LastfmSimilarTracks,
}

#[derive(Debug, Deserialize, Default)]
struct LastfmSimilarTracks {
    #[serde(default)]
    track: Vec<LastfmTrack>,
}

#[derive(Debug, Deserialize)]
struct LastfmTrack {
    #[serde(default)]
    name: String,
    #[serde(default)]
    artist: LastfmArtist,
}

#[derive(Debug, Deserialize, Default)]
struct LastfmArtist {
    #[serde(default)]
    name: String,
}

pub async fn lastfm_similar(api_key: &str, artist: &str, title: &str) -> Result<Vec<(String, String)>, String> {
    let url = format!(
        "https://ws.audioscrobbler.com/2.0/?method=track.getsimilar&artist={}&track={}&api_key={}&format=json&limit=25",
        urlencoding::encode(artist),
        urlencoding::encode(title),
        urlencoding::encode(api_key)
    );

    let resp = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("lastfm status {}", resp.status()));
    }

    let body: LastfmSimilarResponse = resp.json().await.map_err(|e| e.to_string())?;
    let similar: Vec<(String, String)> = body.similartracks.track
        .into_iter()
        .filter(|t| !t.name.is_empty() && !t.artist.name.is_empty())
        .map(|t| (t.artist.name, t.name))
        .collect();

    Ok(similar)
}

// ========== Lyrics (LRCLIB) ==========

#[derive(Debug, Deserialize)]
struct LrclibResp {
    synced_lyrics: Option<String>,
    plain_lyrics: Option<String>,
}

pub async fn get_lyrics(artist: &str, title: &str) -> Result<Option<Vec<(u64, String)>>, String> {
    let url = format!(
        "https://lrclib.net/api/get?artist_name={}&track_name={}",
        urlencoding::encode(artist),
        urlencoding::encode(title)
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "larp-music-player/0.1 (https://github.com/opencode)")
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("lrclib status {}", resp.status()));
    }

    let body: LrclibResp = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(synced) = body.synced_lyrics {
        if !synced.trim().is_empty() {
            return Ok(Some(parse_lrc(&synced)));
        }
    }
    if let Some(plain) = body.plain_lyrics {
        if !plain.trim().is_empty() {
            return Ok(Some(plain.lines().map(|l| (0u64, l.to_string())).collect()));
        }
    }
    Ok(None)
}

// ========== Lyrics (NetEase Cloud Music) ==========

#[derive(Debug, Deserialize)]
struct CloudSearchResp {
    result: CloudSearchResult,
}

#[derive(Debug, Deserialize)]
struct CloudSearchResult {
    #[serde(default)]
    songs: Vec<CloudSong>,
}

#[derive(Debug, Deserialize)]
struct CloudSong {
    id: i64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    ar: Vec<CloudArtist>,
    #[serde(default)]
    dt: i64,
}

#[derive(Debug, Deserialize)]
struct CloudArtist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct NeteaseLyricResp {
    lrc: Option<NeteaseLrc>,
    tlyric: Option<NeteaseLrc>,
}

#[derive(Debug, Deserialize)]
struct NeteaseLrc {
    #[serde(default)]
    lyric: String,
}

fn norm(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

pub async fn get_netease_lyrics(artist: &str, title: &str) -> Result<Option<Vec<(u64, String)>>, String> {
    let query = if title.is_empty() { artist } else { title };

    let client = reqwest::Client::new();
    let search = client
        .post("https://music.163.com/api/cloudsearch/pc")
        .form(&[("s", query), ("type", "1"), ("limit", "10")])
        .header("Referer", "https://music.163.com/")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: CloudSearchResp = search.json().await.map_err(|e| e.to_string())?;

    let n_artist = norm(artist);
    let n_title = norm(title);
    let song = body.result.songs.iter().max_by_key(|s| {
        let mut score = 0i64;
        let s_name = norm(&s.name);
        if !n_title.is_empty() && s_name == n_title {
            score += 100;
        } else if !n_title.is_empty() && s_name.contains(&n_title) {
            score += 50;
        }
        if !n_artist.is_empty() && s.ar.iter().any(|a| norm(&a.name).contains(&n_artist)) {
            score += 40;
        }
        score * 1_000_000 + s.dt
    });
    let song_id = match song {
        Some(s) => s.id,
        None => return Ok(None),
    };

    let lyric = client
        .get(format!(
            "https://music.163.com/api/song/lyric?id={}&lv=1&kv=1&tv=-1",
            song_id
        ))
        .header("Referer", "https://music.163.com/")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: NeteaseLyricResp = lyric.json().await.map_err(|e| e.to_string())?;

    if let Some(lrc) = body.lrc {
        if !lrc.lyric.trim().is_empty() {
            return Ok(Some(parse_lrc(&lrc.lyric)));
        }
    }
    if let Some(tlrc) = body.tlyric {
        if !tlrc.lyric.trim().is_empty() {
            return Ok(Some(parse_lrc(&tlrc.lyric)));
        }
    }
    Ok(None)
}

pub fn parse_lrc(lrc: &str) -> Vec<(u64, String)> {
    let re = regex::Regex::new(r"\[(\d+):(\d+)(?:\.(\d+))?\]").unwrap();
    let mut out = Vec::new();
    for line in lrc.lines() {
        let mut times = Vec::new();
        let mut last_end = 0usize;
        for cap in re.captures_iter(line) {
            let m = cap.get(0).unwrap();
            let min: u64 = cap[1].parse().unwrap_or(0);
            let sec: u64 = cap[2].parse().unwrap_or(0);
            let frac: u64 = cap.get(3).map(|f| f.as_str().parse().unwrap_or(0)).unwrap_or(0);
            let flen = cap.get(3).map(|f| f.as_str().len()).unwrap_or(0);
            let ms = min * 60000 + sec * 1000
                + if flen == 1 { frac * 100 } else if flen == 2 { frac * 10 } else { frac };
            times.push(ms);
            last_end = m.end();
        }
        let text = line[last_end..].trim().to_string();
        if text.is_empty() {
            continue;
        }
        if times.is_empty() {
            out.push((0, text));
        } else {
            for t in times {
                out.push((t, text.clone()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::parse_lrc;

    #[test]
    fn lrc_single_timestamp() {
        let lines = parse_lrc("[00:19.16] When you were here before");
        assert_eq!(lines, vec![(19160, "When you were here before".to_string())]);
    }

    #[test]
    fn lrc_multiple_timestamps() {
        let lines = parse_lrc("[01:00.00][02:00.00] chorus");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], (60000, "chorus".to_string()));
        assert_eq!(lines[1], (120000, "chorus".to_string()));
    }

    #[test]
    fn lrc_plain_line() {
        let lines = parse_lrc("just some text");
        assert_eq!(lines, vec![(0, "just some text".to_string())]);
    }
}
