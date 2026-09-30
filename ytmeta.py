#!/usr/bin/env python3
"""Fetch accurate YouTube Music metadata (duration, big artwork) via ytmusicapi.

Usage:
    ytmeta.py song <videoId>
        -> {"ok": true, "duration_ms": 219000, "artwork_url": "https://...=w1080-h1080"}
        -> {"ok": false, "error": "..."} on failure
"""

import json
import re
import sys

BIG = "=w1080-h1080"


def big_thumb(url):
    """Rewrite a YT thumbnail URL's size params to w1080-h1080."""
    if not url:
        return url
    return re.sub(r"=w\d+-h\d+", BIG, url)


def song(video_id):
    from ytmusicapi import YTMusic

    y = YTMusic()
    s = y.get_song(video_id)
    vd = s.get("videoDetails") or {}
    dur = vd.get("lengthSeconds")
    thumbs = (vd.get("thumbnail") or {}).get("thumbnails") or []
    art = big_thumb(thumbs[-1]["url"]) if thumbs else None
    return {
        "ok": True,
        "duration_ms": int(dur) * 1000 if dur else None,
        "artwork_url": art,
    }


def main():
    if len(sys.argv) < 3 or sys.argv[1] != "song":
        print(json.dumps({"ok": False, "error": "usage: ytmeta.py song <videoId>"}))
        sys.exit(1)
    video_id = sys.argv[2]
    try:
        print(json.dumps(song(video_id)))
    except Exception as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        sys.exit(1)


if __name__ == "__main__":
    main()