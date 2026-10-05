use crate::api::Source;
use crate::app::{App, InputMode, RepeatMode, ViewMode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::borrow::Cow;

pub const CTRL_PLAY_START: usize = 4;
pub const CTRL_NEXT: usize = 10;
pub const CTRL_VOL_START: usize = 15;
pub const CTRL_VOL_LEN: usize = 12;

pub fn ctrl_start(width: u16, vol: f32) -> usize {
    let pct = (vol * 100.0) as u32;
    let ctrl_len = 30 + pct.to_string().len();
    (width as usize).saturating_sub(ctrl_len)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, app, chunks[0]);
    draw_input(f, app, chunks[1]);
    match app.view_mode {
        ViewMode::Lyrics => draw_lyrics(f, app, chunks[2]),
        ViewMode::Playlists if app.active_playlist_id.is_none() => {
            draw_playlist_list(f, app, chunks[2])
        }
        ViewMode::Stats => draw_stats(f, app, chunks[2]),
        _ => draw_tracklist(f, app, chunks[2]),
    }
    draw_player(f, app, chunks[3]);
    draw_footer(f, app, chunks[4]);

    match app.input_mode {
        InputMode::SourceSelect => draw_source_select(f, app),
        InputMode::ViewSelect => draw_view_select(f, app),
        InputMode::PlaylistSelect => draw_playlist_select(f, app),
        InputMode::Help => draw_help(f, app),
        _ => {}
    }
}

fn draw_source_select(f: &mut Frame, app: &App) {
    let sources = [
        ("Yandex Music", Source::YandexMusic),
        ("iTunes", Source::ITunes),
        ("SoundCloud", Source::SoundCloud),
        ("YouTube Music", Source::YouTubeMusic),
    ];
    let items: Vec<Line> = sources
        .iter()
        .enumerate()
        .map(|(i, (label, _src))| {
            let selected = app.select_index == i;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(
                format!(" {} {}", if selected { "▶" } else { " " }, label),
                style,
            ))
            .patch_style(Style::default())
        })
        .collect();
    draw_select_panel(f, " Source ", items, app);
}

fn draw_view_select(f: &mut Frame, app: &App) {
    let views = [
        ("Search", ViewMode::Search),
        ("Liked", ViewMode::Liked),
        ("Lyrics", ViewMode::Lyrics),
        ("Playlists", ViewMode::Playlists),
        ("Stats", ViewMode::Stats),
    ];
    let items: Vec<Line> = views
        .iter()
        .enumerate()
        .map(|(i, (label, view))| {
            let selected = app.select_index == i;
            let marker = if *view == app.view_mode { " ✓" } else { "" };
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(
                format!(" {} {}{}", if selected { "▶" } else { " " }, label, marker),
                style,
            ))
        })
        .collect();
    draw_select_panel(f, " View ", items, app);
}

fn draw_playlist_select(f: &mut Frame, app: &App) {
    if app.playlists.is_empty() {
        return;
    }
    let items: Vec<Line> = app
        .playlists
        .iter()
        .enumerate()
        .map(|(i, pl)| {
            let selected = app.select_index == i;
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::White)
            } else {
                Style::default().fg(Color::White)
            };
            Line::from(Span::styled(
                format!(
                    " {} {} ({})",
                    if selected { "▶" } else { " " },
                    pl.name,
                    pl.count
                ),
                style,
            ))
        })
        .collect();
    draw_select_panel(f, " Add to playlist ", items, app);
}

fn draw_help(f: &mut Frame, _app: &App) {
    let rows: Vec<Line> = [
        ("Shift+H", "help"),
        ("Shift+P", "playlists view"),
        ("Shift+A", "add track to playlist"),
        ("Shift+N", "new playlist"),
        ("Shift+D", "delete playlist / remove from playlist"),
        ("   /", "search"),
        ("Tab", "views (search/liked/lyrics/playlists)"),
        (" t", "source"),
        (" L", "like"),
        (" r", "repeat"),
        (" x", "shuffle"),
        (" g", "lyrics"),
        (" w", "radio"),
        ("Enter", "play / open"),
        ("Space", "play / pause"),
        ("Esc", "back / close"),
        ("m", "stats: switch ranking"),
        (" u", "check for updates"),
    ]
    .iter()
    .map(|(k, desc)| {
        Line::from(vec![
            Span::styled(
                format!("  {:<9}", k),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(*desc, Style::default().fg(Color::White)),
        ])
    })
    .collect();

    let area = f.area();
    let w = (area.width / 2).max(34);
    let h = rows.len() as u16 + 2;
    let x = area.x + (area.width / 2).saturating_sub(w / 2);
    let y = area.y + (area.height / 2).saturating_sub(h / 2);
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(" Keybindings ")
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .title_alignment(Alignment::Center);

    let p = Paragraph::new(rows).block(block);
    f.render_widget(p, popup);
}

fn draw_select_panel(f: &mut Frame, title: &str, items: Vec<Line>, _app: &App) {
    let area = f.area();
    let w = (area.width / 3).max(24);
    let h = items.len() as u16 + 2;
    let x = area.x + (area.width / 2).saturating_sub(w / 2);
    let y = area.y + (area.height / 2).saturating_sub(h / 2);
    let popup = Rect::new(x, y, w, h);

    f.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .title_alignment(Alignment::Center);

    let p = Paragraph::new(items).block(block);
    f.render_widget(p, popup);
}

fn source_label(source: &Source) -> &'static str {
    match source {
        Source::YandexMusic => "YM",
        Source::ITunes => "iT",
        Source::SoundCloud => "SC",
        Source::YouTubeMusic => "YT",
    }
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let src = source_label(&app.current_source);

    let view_label = match &app.view_mode {
        ViewMode::Search => "search",
        ViewMode::Liked => "liked",
        ViewMode::Lyrics => "lyrics",
        ViewMode::Playlists => "playlists",
        ViewMode::Stats => "stats",
    };

    let hint = "Shift+H help";

    let mut indicators = String::new();
    if app.shuffle || app.liked_shuffle {
        indicators.push_str(" SHUF");
    }
    indicators.push_str(match app.repeat_mode {
        RepeatMode::Off => "",
        RepeatMode::All => " REP",
        RepeatMode::One => " REP1",
    });
    if (app.playback_speed - 1.0).abs() > 0.01 {
        indicators.push_str(&format!(" {:.2}x", app.playback_speed));
    }
    if app.radio_active {
        indicators.push_str(" RADIO");
    }

    let mut spans = vec![
        Span::styled(
            " larp-music  ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("[{}] ", src), Style::default().fg(Color::DarkGray)),
        Span::styled(view_label, Style::default().fg(Color::DarkGray)),
        Span::styled("   ", Style::default()),
        Span::styled(hint, Style::default().fg(Color::DarkGray)),
        Span::styled(indicators, Style::default().fg(Color::Cyan)),
    ];
    if let Some(bt) = app.bluetooth_device.as_ref() {
        let prefix_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let avail = area.width as usize;
        let max_label = avail.saturating_sub(prefix_len).max(4) as usize;
        let mut label = "  BT: ".to_string();
        if bt.chars().count() + label.chars().count() > max_label {
            let budget = max_label.saturating_sub(label.chars().count()).max(1);
            let truncated: String = bt.chars().take(budget.saturating_sub(1)).collect();
            label.push_str(&truncated);
            label.push('…');
        } else {
            label.push_str(bt);
        }
        spans.push(Span::styled(
            label,
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        ));
    }

    let line = Line::from(spans);

    let header = Paragraph::new(line);
    f.render_widget(header, area);
}

fn draw_input(f: &mut Frame, app: &App, area: Rect) {
    match app.input_mode {
        InputMode::TokenInput => {
            let masked: String = app.token.chars().map(|_| '*').collect();
            let text = format!("  token: {}█", masked);
            let widget = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            f.render_widget(widget, area);
        }
        InputMode::Search => {
            let text = format!("  /{}█", app.search_query);
            let widget = Paragraph::new(text).style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            );
            f.render_widget(widget, area);
        }
        InputMode::Normal => {
            let widget =
                Paragraph::new("  press / to search").style(Style::default().fg(Color::DarkGray));
            f.render_widget(widget, area);
        }
        InputMode::ClientIdInput => {
            let text = format!("  sc client_id: {}█", app.sc_client_id);
            let widget = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            f.render_widget(widget, area);
        }
        InputMode::NameInput => {
            let text = format!("  new playlist: {}█", app.name_input);
            let widget = Paragraph::new(text).style(Style::default().fg(Color::Yellow));
            f.render_widget(widget, area);
        }
        InputMode::SourceSelect
        | InputMode::ViewSelect
        | InputMode::PlaylistSelect
        | InputMode::Help => {}
    }
}

fn draw_playlist_list(f: &mut Frame, app: &App, area: Rect) {
    if app.playlists.is_empty() {
        let p = Paragraph::new("  no playlists — Shift+N to create")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }

    let title_line = Line::from(vec![
        Span::styled(
            "  playlists ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({})", app.playlists.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let title_block = Block::default().title(title_line);
    f.render_widget(title_block, area);

    let inner = area.inner(Margin {
        horizontal: 0,
        vertical: 1,
    });
    let item_height: u16 = 3;
    let visible_items = (inner.height / item_height).max(1) as usize;
    let max_scroll = app.playlists.len().saturating_sub(visible_items);
    let scroll = app
        .current_index
        .saturating_sub(visible_items / 2)
        .min(max_scroll);

    for i in 0..visible_items {
        let idx = scroll + i;
        if idx >= app.playlists.len() {
            break;
        }
        let pl = &app.playlists[idx];
        let is_current = idx == app.current_index;

        let item_y = inner.y + (i as u16) * item_height;
        if item_y + item_height > inner.y + inner.height {
            break;
        }
        let item_area = Rect {
            x: inner.x,
            y: item_y,
            width: inner.width,
            height: item_height,
        };

        let line = Line::from(vec![
            Span::styled(
                if is_current { " ▸ " } else { "   " },
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                &pl.name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(if is_current {
                        Modifier::BOLD
                    } else {
                        Modifier::from_bits(0).unwrap()
                    }),
            ),
            Span::styled(
                format!("  {} tracks", pl.count),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let border_color = if is_current {
            Color::Rgb(180, 160, 80)
        } else {
            Color::Rgb(60, 60, 60)
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::block::BorderType::Rounded)
            .border_style(Style::default().fg(border_color));
        let paragraph = Paragraph::new(line).block(block);
        f.render_widget(paragraph, item_area);
    }
}

fn draw_tracklist(f: &mut Frame, app: &mut App, area: Rect) {
    if app.tracks.is_empty() {
        let msg = match &app.view_mode {
            ViewMode::Search => "  no tracks — press / to search",
            ViewMode::Liked => "  no liked songs — press L to like",
            ViewMode::Playlists => "  empty playlist — Shift+A to add tracks",
            ViewMode::Lyrics | ViewMode::Stats => "",
        };
        let empty = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, area);
        return;
    }

    let view: Cow<'_, str> = match &app.view_mode {
        ViewMode::Search => "tracks".into(),
        ViewMode::Liked => "liked".into(),
        ViewMode::Playlists => app
            .playlist_name()
            .map(Cow::Owned)
            .unwrap_or_else(|| "playlist".into()),
        ViewMode::Lyrics => "lyrics".into(),
        ViewMode::Stats => "stats".into(),
    };

    let playing_id = if app.is_playing {
        app.current_track().map(|t| t.id.clone())
    } else {
        None
    };

    let title_line = Line::from(vec![
        Span::styled(
            format!("  {} ", view),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("({})", app.tracks.len()),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let title_block = Block::default().title(title_line);
    f.render_widget(title_block, area);

    let inner = area.inner(Margin {
        horizontal: 0,
        vertical: 1,
    });

    let item_height: u16 = 3;
    let visible_items = (inner.height / item_height).max(1) as usize;

    let max_scroll = app.tracks.len().saturating_sub(visible_items);
    let scroll = app
        .current_index
        .saturating_sub(visible_items / 2)
        .min(max_scroll);

    for i in 0..visible_items {
        let track_idx = scroll + i;
        if track_idx >= app.tracks.len() {
            break;
        }

        let track = &app.tracks[track_idx];
        let is_current = track_idx == app.current_index;
        let is_playing = playing_id.as_deref() == Some(track.id.as_str()) && is_current;
        let is_liked = app.db.is_liked(&track.source, &track.id).unwrap_or(false);

        let item_y = inner.y + (i as u16) * item_height;
        if item_y + item_height > inner.y + inner.height {
            break;
        }

        let item_area = Rect {
            x: inner.x,
            y: item_y,
            width: inner.width,
            height: item_height,
        };

        let indicator = if is_playing {
            Span::styled(
                " ▶ ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )
        } else if is_current {
            Span::styled(" ▸ ", Style::default().fg(Color::Yellow))
        } else {
            Span::styled("   ", Style::default())
        };

        let heart = if is_liked {
            Span::styled("♥ ", Style::default().fg(Color::Red))
        } else {
            Span::styled("  ", Style::default())
        };

        let duration = track
            .duration_ms
            .map(|ms| {
                let secs = ms / 1000;
                format!("{}:{:02}", secs / 60, secs % 60)
            })
            .unwrap_or_else(|| "??:??".to_string());

        let name_style = if is_current {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        let plays = app.play_count_for(&track.source, &track.id);
        let play_badge = if plays > 0 {
            Span::styled(format!("  ▶{}", plays), Style::default().fg(Color::Magenta))
        } else {
            Span::raw("")
        };

        let line = Line::from(vec![
            indicator,
            heart,
            Span::styled(&track.artist, name_style.clone()),
            Span::styled(" — ", Style::default().fg(Color::DarkGray)),
            Span::styled(&track.title, name_style),
            play_badge,
            Span::styled(
                format!("  {}", duration),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        let border_color = if is_current {
            Color::Rgb(180, 160, 80)
        } else {
            Color::Rgb(60, 60, 60)
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(ratatui::widgets::block::BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(if is_current {
                Color::Rgb(25, 25, 35)
            } else {
                Color::Reset
            }));

        let paragraph = Paragraph::new(line).block(block);
        f.render_widget(paragraph, item_area);
    }
}

fn draw_stats(f: &mut Frame, app: &mut App, area: Rect) {
    use crate::db::{fmt_listened, TopBy};

    let totals = app.db.stats_totals().unwrap_or_default();

    let mut lines: Vec<Line> = Vec::new();
    let dim = Style::default().fg(Color::DarkGray);
    let head = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let val = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);

    if totals.plays == 0 {
        let p = Paragraph::new("  no listening history yet — play something")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }

    lines.push(Line::from(Span::styled("Totals", head)));
    lines.push(Line::from(vec![
        Span::styled("  listened ", dim),
        Span::styled(fmt_listened(totals.listened_ms), val),
        Span::styled("   plays ", dim),
        Span::styled(totals.plays.to_string(), val),
        Span::styled("   tracks ", dim),
        Span::styled(totals.tracks.to_string(), val),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  finished ", dim),
        Span::styled(totals.completed.to_string(), val),
        Span::styled("   active days ", dim),
        Span::styled(totals.days.to_string(), val),
    ]));
    if let Some(first) = &totals.first_day {
        lines.push(Line::from(Span::styled(format!("  since {}", first), dim)));
    }

    let by = app.stats_top_by;
    let title = match by {
        TopBy::Plays => "Most played",
        TopBy::Listened => "Most listened",
        TopBy::Completed => "Most finished",
    };
    let hint = match by {
        TopBy::Completed => "  (m: most played / listened / finished)",
        _ => "  (m: most played / listened / finished)",
    };
    lines.push(Line::from(Span::styled(
        format!("\n{}{}", title, hint),
        head,
    )));

    if let Ok(rows) = app.db.top_tracks(by, 10) {
        if rows.is_empty() {
            lines.push(Line::from(Span::styled("  nothing yet", dim)));
        }
        let label_of = |r: &crate::db::TrackStat| {
            if r.title.is_empty() {
                format!("{} {}", source_label(&r.source), r.track_id)
            } else if r.artist.is_empty() {
                r.title.clone()
            } else {
                format!("{} \u{2014} {}", r.artist, r.title)
            }
        };
        let longest = rows
            .iter()
            .map(|r| label_of(r).chars().count())
            .max()
            .unwrap_or(10);
        for (i, r) in rows.iter().enumerate() {
            let num = format!("{:>2}.", i + 1);
            let title_txt = format!(
                " {:<width$} ",
                label_of(r).chars().take(longest).collect::<String>(),
                width = longest
            );
            let meta = match by {
                TopBy::Listened => {
                    format!("{} · {} plays", fmt_listened(r.listened_ms), r.play_count)
                }
                TopBy::Completed => format!("{} · {} plays", r.completed_count, r.play_count),
                TopBy::Plays => format!("{} · {}", r.play_count, fmt_listened(r.listened_ms)),
            };
            lines.push(Line::from(vec![
                Span::styled(num, dim),
                Span::raw(title_txt),
                Span::styled(source_label(&r.source).to_string(), dim),
                Span::styled(format!("  {}", meta), Style::default().fg(Color::Green)),
                Span::styled(
                    format!("  \u{293c}{}", r.skip_count),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
        }
    }

    if let Ok(days) = app.db.daily_stats(7) {
        if !days.is_empty() {
            lines.push(Line::from(Span::styled("\nLast days", head)));
            let peak = days.iter().map(|d| d.listened_ms).max().unwrap_or(1).max(1);
            let width = (area.width.saturating_sub(34)).clamp(4, 40) as usize;
            for d in days {
                let bars = ((d.listened_ms as f64 / peak as f64) * width as f64) as usize;
                lines.push(Line::from(vec![
                    Span::styled(format!("  {} ", d.day), dim),
                    Span::styled(
                        "█".repeat(bars.max(usize::from(d.listened_ms > 0))),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        format!("  {} · {} plays", fmt_listened(d.listened_ms), d.plays),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]));
            }
        }
    }

    let p = Paragraph::new(lines).scroll((app.stats_scroll, 0));
    f.render_widget(p, area);
}

fn draw_lyrics(f: &mut Frame, app: &App, area: Rect) {
    if app.lyrics_loading {
        let p = Paragraph::new("  loading lyrics...").style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }
    if let Some(err) = &app.lyrics_error {
        let p = Paragraph::new(format!("  {}", err)).style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }
    if app.lyrics_lines.is_empty() {
        let p = Paragraph::new("  no lyrics — press g to refresh")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, area);
        return;
    }

    let (artist, title) = app
        .playing_track
        .as_ref()
        .or_else(|| app.current_track())
        .map(|t| (t.artist.clone(), t.title.clone()))
        .unwrap_or_else(|| ("".to_string(), "".to_string()));

    let pos = app.position_ms().unwrap_or(0);
    let has_sync = app.lyrics_lines.iter().any(|(t, _)| *t > 0);

    let mut active = 0usize;
    if has_sync {
        for (i, (t, _)) in app.lyrics_lines.iter().enumerate() {
            if *t <= pos {
                active = i;
            }
        }
    }

    let inner_h = (area.height as usize).saturating_sub(3);
    let window = inner_h.div_ceil(2).max(1);
    let scroll = if has_sync {
        active
            .saturating_sub(window / 2)
            .min(app.lyrics_lines.len().saturating_sub(window))
    } else {
        0
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("  ♪ ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{} — {}", artist, title),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  via {}", app.lyrics_source.as_deref().unwrap_or("?")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                format!(
                    "   —  {}%",
                    (app.position_ms().unwrap_or(0) as f32
                        / app.track_duration_ms.unwrap_or(1).max(1) as f32
                        * 100.0)
                        .min(100.0) as u32
                ),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(""),
    ];

    for i in scroll..app.lyrics_lines.len() {
        if lines.len() >= inner_h {
            break;
        }
        let (_, text) = &app.lyrics_lines[i];
        let is_active = has_sync && i == active;
        let style = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD)
        };
        let content = if is_active {
            let span = Span::styled(text.clone(), style);
            let pad = area
                .width
                .saturating_sub(Line::from(vec![span.clone()]).width() as u16 + 2);
            let mut spans = vec![Span::styled(" ► ", style), span];
            if pad > 0 {
                spans.push(Span::styled(" ".repeat(pad as usize), style));
            }
            Line::from(spans)
        } else {
            Line::from(vec![Span::styled(format!("   {}", text), style)])
        };
        lines.push(content);
        lines.push(Line::from(""));
    }

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn draw_player(f: &mut Frame, app: &App, area: Rect) {
    let vol = (app.volume * 100.0) as u32;

    let (track_info, src_label) = app
        .playing_track
        .as_ref()
        .or_else(|| app.current_track())
        .map(|t| {
            let mut info = format!("{} — {}", t.artist, t.title);
            if let Some(album) = &t.album {
                info.push_str(&format!("  [{}", album));
                if let Some(year) = t.year {
                    info.push_str(&format!(" ({})", year));
                }
                info.push(']');
            } else if let Some(year) = t.year {
                info.push_str(&format!("  [({})]", year));
            }
            (info, source_label(&t.source))
        })
        .unwrap_or_else(|| ("—".to_string(), ""));

    let vol_bar_len = 12;
    let filled = (vol as usize * vol_bar_len) / 100;
    let vol_bar: String = (0..vol_bar_len)
        .map(|i| if i < filled { '▰' } else { '▱' })
        .collect();

    let play_glyph = if app.is_playing { "||" } else { "▶ " };
    let ctrl = format!(" ◀   {}   >    {} {}% ", play_glyph, vol_bar, vol);
    let ctrl_w = ctrl.chars().count() as i32 + 2;

    let mut left = format!("{}  {}", track_info, src_label);
    if app.is_current_liked() {
        left.push_str(" ♥ ");
    }

    let avail = area.width as usize;
    let left_w = (avail as i32 - ctrl_w).max(0) as usize;
    if left.chars().count() > left_w {
        let truncated: String = left.chars().take(left_w.saturating_sub(3)).collect();
        left = format!("{}…", truncated);
    } else {
        left.push_str(&" ".repeat(left_w.saturating_sub(left.chars().count())));
    }

    let line = Line::from(vec![
        Span::styled(
            &left,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(ctrl, Style::default().fg(Color::DarkGray)),
    ]);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let player = Paragraph::new(line).block(block);
    f.render_widget(player, area);
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let play_state = if let Some((current, total)) = app.download_liked_progress {
        let bar_len = 20;
        let filled = if total > 0 {
            (current * bar_len) / total
        } else {
            0
        };
        let bar: String = (0..bar_len)
            .map(|i| if i < filled { '█' } else { '░' })
            .collect();
        Span::styled(
            format!("  ↓ {}/{} {}%", current, total, bar),
            Style::default().fg(Color::Cyan),
        )
    } else if let Some(pct) = app.download_progress {
        let bar_len = 30;
        let filled = (pct as usize * bar_len) / 100;
        let bar: String = (0..bar_len)
            .map(|i| if i < filled { '█' } else { '░' })
            .collect();
        if pct <= 100 {
            Span::styled(
                format!("  ▼ {} {}%", bar, pct),
                Style::default().fg(Color::Cyan),
            )
        } else {
            Span::styled("  decoding...", Style::default().fg(Color::Cyan))
        }
    } else if app.loading_play {
        Span::styled("  loading...", Style::default().fg(Color::DarkGray))
    } else if let (Some(pos), Some(dur_ms)) = (app.position_ms(), app.track_duration_ms) {
        let elapsed_ms = pos;
        if elapsed_ms >= dur_ms + 2000 {
            Span::styled("  ▶ finished", Style::default().fg(Color::DarkGray))
        } else {
            let elapsed = if elapsed_ms >= dur_ms {
                dur_ms
            } else {
                elapsed_ms
            };
            let total_secs = dur_ms / 1000;
            let elapsed_secs = elapsed / 1000;
            let progress = if total_secs > 0 {
                ((elapsed as f64 / dur_ms as f64) * 100.0).min(100.0) as u32
            } else {
                0
            };
            let bar_len = 20;
            let filled = (progress as usize * bar_len) / 100;
            let bar: String = (0..bar_len)
                .map(|i| if i < filled { '━' } else { '─' })
                .collect();
            Span::styled(
                format!(
                    "  {}:{:02} {} {}:{:02}",
                    elapsed_secs / 60,
                    elapsed_secs % 60,
                    bar,
                    total_secs / 60,
                    total_secs % 60
                ),
                Style::default().fg(Color::Green),
            )
        }
    } else if let (Some(paused), Some(dur_ms)) = (app.paused_at_ms, app.track_duration_ms) {
        let elapsed = if paused >= dur_ms { dur_ms } else { paused };
        let total_secs = dur_ms / 1000;
        let elapsed_secs = elapsed / 1000;
        let progress = if total_secs > 0 {
            ((elapsed as f64 / dur_ms as f64) * 100.0).min(100.0) as u32
        } else {
            0
        };
        let bar_len = 20;
        let filled = (progress as usize * bar_len) / 100;
        let bar: String = (0..bar_len)
            .map(|i| if i < filled { '━' } else { '─' })
            .collect();
        Span::styled(
            format!(
                "  ▸ {}:{:02} {} {}:{:02}",
                elapsed_secs / 60,
                elapsed_secs % 60,
                bar,
                total_secs / 60,
                total_secs % 60
            ),
            Style::default().fg(Color::DarkGray),
        )
    } else if app.started_at.is_some() {
        Span::styled("  ▶ playing", Style::default().fg(Color::Green))
    } else if app.is_playing {
        Span::styled("  ▶ playing", Style::default().fg(Color::Green))
    } else {
        Span::styled("  ▸ paused", Style::default().fg(Color::DarkGray))
    };

    let status = if app.status_message.is_empty()
        || app.status_message == "Playing"
        || app.status_message == "Paused"
    {
        Span::raw("")
    } else {
        Span::styled(
            format!("  {}", app.status_message),
            Style::default().fg(Color::DarkGray),
        )
    };

    let line = Line::from(vec![play_state, status]);
    let footer = Paragraph::new(line);
    f.render_widget(footer, area);
}
