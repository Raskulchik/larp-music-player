#!/usr/bin/env python3
"""Fetch accurate metadata (duration, big artwork) for any supported source.

Usage:
    meta.py ytmusic <videoId>
    meta.py itunes <trackId>
    meta.py soundcloud <trackId> <clientId>
    meta.py yandex <trackId> <token>

Output JSON: {"ok": true, "duration_ms": 219000, "artwork_url": "..."}
Or:         {"ok": false, "error": "..."} on failure
"""

import json
import re
import sys
import urllib.request

UA = "larp-music-player/0.1 (linux)"


def http_json(url, token=None, client_id=None):
    if client_id:
        url += ("&" if "?" in url else "?") + "client_id=" + client_id
    req = urllib.request.Request(url, headers={"User-Agent": UA, "Accept": "application/json"})
    if token:
        req.add_header("Authorization", "OAuth " + token)
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.load(r)


def big_thumb(url):
    """Rewrite a YT thumbnail URL size (`=w###-h###`) to w1080-h1080."""
    if not url:
        return url
    return re.sub(r"=w\d+-h\d+", "=w1080-h1080", url)


def song(source, vid, token=None, client_id=None):
    if source == "ytmusic":
        from ytmusicapi import YTMusic

        s = YTMusic().get_song(vid)
        vd = s.get("videoDetails") or {}
        dur = vd.get("lengthSeconds")
        thumbs = (vd.get("thumbnail") or {}).get("thumbnails") or []
        return {
            "ok": True,
            "duration_ms": int(dur) * 1000 if dur else None,
            "artwork_url": big_thumb(thumbs[-1]["url"]) if thumbs else None,
        }

    if source == "itunes":
        d = http_json("https://itunes.apple.com/lookup?id=" + vid)
        r = (d.get("results") or [{}])[0]
        art = r.get("artworkUrl100")
        if art:
            art = art.replace("100x100bb", "600x600bb")
        return {"ok": True, "duration_ms": r.get("trackTimeMillis"), "artwork_url": art}

    if source == "soundcloud":
        d = http_json("https://api-v2.soundcloud.com/tracks/" + vid, client_id=client_id)
        art = d.get("artwork_url")
        if art:
            art = art.replace("-large", "-t500x500")
        return {"ok": True, "duration_ms": d.get("duration"), "artwork_url": art}

    if source == "yandex":
        d = http_json("https://api.music.yandex.net/tracks/" + vid, token=token)
        r = (d.get("result") or [{}])[0]
        cover = r.get("coverUri")
        art = ("https://" + cover.replace("%%", "1000x1000")) if cover else None
        return {"ok": True, "duration_ms": r.get("durationMs"), "artwork_url": art}

    return {"ok": False, "error": "unknown source: " + source}


def main():
    args = sys.argv[1:]
    if len(args) < 2:
        print(json.dumps({"ok": False, "error": "usage: meta.py <source> <id> [client_id|token]"}))
        sys.exit(1)
    source, vid = args[0], args[1]
    token = client_id = None
    if source == "soundcloud" and len(args) > 2:
        client_id = args[2]
    if source == "yandex" and len(args) > 2:
        token = args[2]
    try:
        print(json.dumps(song(source, vid, token, client_id)))
    except Exception as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()