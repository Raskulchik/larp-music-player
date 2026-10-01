use serde::Deserialize;

pub const REPO: &str = "Raskulchik/larp-music-player";

fn api_url() -> String {
    format!("https://api.github.com/repos/{}/releases/latest", REPO)
}

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Compare two `vX.Y.Z` tags (semver, pre-release aware).
/// Returns true if `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (Vec<u64>, bool) {
        let body = s.trim().trim_start_matches('v');
        let (core, pre) = match body.split_once('-') {
            Some((c, p)) => (c, p),
            None => (body, ""),
        };
        let core = core
            .split('+')
            .next()
            .unwrap_or(core)
            .split('.')
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect();
        (core, !pre.is_empty())
    };
    let (l, l_pre) = parse(latest);
    let (c, c_pre) = parse(current);
    for i in 0..std::cmp::max(l.len(), c.len()) {
        let lv = l.get(i).copied().unwrap_or(0);
        let cv = c.get(i).copied().unwrap_or(0);
        if lv != cv {
            return lv > cv;
        }
    }
    // Same core: a final release beats a pre-release.
    c_pre && !l_pre
}

async fn fetch_latest() -> Option<Release> {
    let client = reqwest::Client::builder()
        .user_agent(format!("larp-music-player/{}", current_version()))
        .build()
        .ok()?;
    let rel = client.get(api_url()).send().await.ok()?;
    if !rel.status().is_success() {
        return None;
    }
    let rel: Release = rel.json().await.ok()?;
    if rel.draft || rel.prerelease {
        return None;
    }
    Some(rel)
}

/// Check GitHub for a newer release. Returns the new tag + release URL if newer than current.
pub async fn check() -> Option<(String, String)> {
    let rel = fetch_latest().await?;
    if is_newer(&rel.tag_name, current_version()) {
        Some((rel.tag_name, rel.html_url))
    } else {
        None
    }
}

pub fn open_in_browser(url: &str) {
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.1.0"));
        assert!(!is_newer("v0.1.0", "v0.2.0"));
        assert!(is_newer("v0.1.1", "v0.1.0"));
        assert!(is_newer("v1.0.0", "v0.9.9"));
        // pre-release ordering: v0.2.0-rc1 > v0.1.0, but < v0.2.0
        assert!(is_newer("v0.2.0-rc1", "v0.1.0"));
        assert!(is_newer("v0.2.0", "v0.2.0-rc1"));
        assert!(!is_newer("v0.2.0-rc1", "v0.2.0"));
        // missing component treated as 0
        assert!(is_newer("v0.1.1", "v0.1"));
        assert!(!is_newer("v0.1", "v0.1.0"));
    }
}
