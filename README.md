Baka

<img width="498" height="390" alt="anime-tsundere" src="https://github.com/user-attachments/assets/522907f3-1cf0-4a72-a01c-490e553f142c" />

## Windows version (beta)

https://github.com/Raskulchik/larp-music-player-windows

## CLI

Run the binary with arguments for CLI mode; run it without arguments to open the TUI.

```bash
larp-music-player --help
larp-music-player status
larp-music-player play "Kino - Gruppa Krovi"
larp-music-player volume 40
larp-music-player search "Molchat Doma" -s ytmusic -n 5
larp-music-player stats --top 10
```

Playback commands (`status`, `play`, `pause`, `next`, `volume`, ...) talk to a
running TUI over a Unix socket. Library commands (`search`, `liked`,
`playlists`, `playlist`, `stats`, `local`, `info`, `config`, `sources`) work
without a running player.

Use `larp-music-player --help` for the full command list and examples.
