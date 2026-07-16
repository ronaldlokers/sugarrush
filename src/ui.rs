//! Rendering. v1: a single dashboard screen.

use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph},
    Frame,
};

use crate::app::{App, Field, Screen};
use crate::bigfont;
use crate::config::GraphStyle;
use crate::stats;

pub fn draw(f: &mut Frame, app: &App) {
    if app.screen == Screen::Settings {
        draw_settings(f, app);
        return;
    }
    // A one-line alert banner appears above the header only while alerting.
    let banner = app.alert.is_alerting();
    let minimap = app.minimap_enabled;
    let mut constraints = Vec::new();
    if banner {
        constraints.push(Constraint::Length(1)); // banner
    }
    constraints.extend([
        Constraint::Length(3), // header
        Constraint::Length(7), // current reading
        Constraint::Length(5), // stats
        Constraint::Min(8),    // graph
    ]);
    if minimap {
        constraints.push(Constraint::Length(4)); // minimap
    }
    constraints.push(Constraint::Length(1)); // footer

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(f.area());

    let mut i = 0;
    if banner {
        draw_banner(f, chunks[i], app);
        i += 1;
    }
    draw_header(f, chunks[i], app);
    draw_current(f, chunks[i + 1], app);
    draw_stats(f, chunks[i + 2], app);
    draw_graph(f, chunks[i + 3], app);
    i += 4;
    if minimap {
        draw_minimap(f, chunks[i], app);
        i += 1;
    }
    draw_footer(f, chunks[i], app);
}

fn draw_minimap(f: &mut Frame, area: Rect, app: &App) {
    let hours = app.minimap_span_ms / 3_600_000;
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {hours}h overview "));
    let inner = block.inner(area);
    f.render_widget(block, area);
    // Record the inner rect so mouse events can map columns back to time.
    app.minimap_rect.set(Some(inner));

    let now = chrono::Utc::now().timestamp_millis();
    let start = now - app.minimap_span_ms;

    if app.minimap_entries.is_empty() {
        return;
    }

    let points: Vec<(f64, f64)> = app
        .minimap_entries
        .iter()
        .rev()
        .map(|e| (e.date as f64, app.units.from_mgdl(e.sgv)))
        .collect();
    let (min_y, max_y) = points
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), (_, y)| {
            (lo.min(*y), hi.max(*y))
        });
    let bounds_y = [min_y, max_y.max(min_y + 1.0)];

    // Bracket the currently-visible window with two vertical rules.
    let vs = (app.view_start.max(start)) as f64;
    let ve = (app.view_end.min(now)) as f64;
    let start_rule = [(vs, bounds_y[0]), (vs, bounds_y[1])];
    let end_rule = [(ve, bounds_y[0]), (ve, bounds_y[1])];

    let datasets = vec![
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::DarkGray))
            .data(&points),
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(app.theme.graph))
            .data(&start_rule),
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(app.theme.graph))
            .data(&end_rule),
    ];

    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds([start as f64, now as f64]))
        .y_axis(Axis::default().bounds(bounds_y));
    f.render_widget(chart, inner);
}

fn draw_stats(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" stats ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let u = app.units;
    // Time-in-range over the loaded window.
    let tir_line = match stats::tir(&app.entries, app.alerts.low, app.alerts.high) {
        Some(t) => Line::from(vec![
            Span::raw("  TIR  "),
            Span::styled(
                format!("low {:.0}%", t.low),
                Style::default().fg(Color::Red),
            ),
            Span::raw("  "),
            Span::styled(
                format!("in-range {:.0}%", t.in_range),
                Style::default().fg(Color::Green),
            ),
            Span::raw("  "),
            Span::styled(
                format!("high {:.0}%", t.high),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        None => Line::from("  TIR  —"),
    };

    // Mean + estimated A1c.
    let avg_line = match stats::mean_mgdl(&app.entries) {
        Some(mean) => Line::from(format!(
            "  avg  {} {}   ·   GMI {:.1}%",
            u.format(mean),
            u.label(),
            stats::gmi(mean),
        )),
        None => Line::from("  avg  —"),
    };

    // Device / uploader status.
    let now = chrono::Utc::now().timestamp_millis();
    let mut parts = Vec::new();
    if let Some(name) = &app.device.device {
        parts.push(name.clone());
    }
    if let Some(b) = app.device.battery {
        parts.push(format!("battery {b}%"));
    }
    if let Some(start) = app.sensor_start_ms {
        parts.push(format!("sensor {}", fmt_age(now - start)));
    }
    if let Some(last) = app.device.last_ms {
        parts.push(format!("uploader {} ago", fmt_age(now - last)));
    }
    let dev_line = if parts.is_empty() {
        Line::from(Span::styled(
            "  device  —",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(
            format!("  {}", parts.join("   ·   ")),
            Style::default().fg(Color::DarkGray),
        ))
    };

    f.render_widget(Paragraph::new(vec![tir_line, avg_line, dev_line]), inner);
}

/// Format a positive duration in ms as a compact age like `6d 4h` or `12m`.
fn fmt_age(ms: i64) -> String {
    let mins = ms.max(0) / 60_000;
    let days = mins / 1440;
    let hours = (mins % 1440) / 60;
    let m = mins % 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {m}m")
    } else {
        format!("{m}m")
    }
}

fn draw_settings(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(5),    // fields
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    let header = Paragraph::new(Line::from(Span::styled(
        " settings ",
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, chunks[0]);

    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    // Scroll so the selected row stays visible when the list is taller than
    // the pane.
    let height = inner.height.max(1) as usize;
    let total = Field::ALL.len();
    let offset = if app.settings_sel < height {
        0
    } else {
        (app.settings_sel + 1 - height).min(total.saturating_sub(height))
    };
    let rows: Vec<Line> = Field::ALL
        .iter()
        .enumerate()
        .skip(offset)
        .take(height)
        .map(|(i, &field)| {
            let selected = i == app.settings_sel;
            let marker = if selected { "▸ " } else { "  " };
            let style = if selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(
                format!("{marker}{:<22}{}", field.label(), app.field_value(field)),
                style,
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(rows), inner);

    let footer = match &app.status {
        Some(msg) => Span::styled(format!(" {msg} "), Style::default().fg(Color::Green)),
        None => Span::raw(" ↑/↓ select · ←/→ change · w save · s/esc back · q quit "),
    };
    f.render_widget(Paragraph::new(Line::from(footer)), chunks[2]);
}

fn draw_banner(f: &mut Frame, area: Rect, app: &App) {
    let color = app.alert.color();
    let line = Line::from(Span::styled(
        format!(" ⚠ {} ", app.alert.label()),
        Style::default()
            .fg(Color::Black)
            .bg(color)
            .add_modifier(Modifier::BOLD),
    ));
    f.render_widget(
        Paragraph::new(line)
            .style(Style::default().bg(color))
            .alignment(Alignment::Center),
        area,
    );
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mode = if app.view.is_live() {
        Span::styled(" ● live ", Style::default().fg(Color::Green))
    } else {
        Span::styled(" ◷ history ", Style::default().fg(Color::Yellow))
    };
    let mut spans = vec![
        Span::styled(
            " sugarrush ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "· {} · {} ",
            app.units.label(),
            app.view.span.label()
        )),
        mode,
    ];
    if app.sites.len() > 1 {
        spans.push(Span::styled(
            format!(" [{}] ", app.active_site().name),
            Style::default().fg(Color::Blue),
        ));
    }
    if !app.online {
        let age = app
            .last_ok_ms
            .map(|t| {
                format!(
                    " (last {} ago)",
                    fmt_age(chrono::Utc::now().timestamp_millis() - t)
                )
            })
            .unwrap_or_default();
        spans.push(Span::styled(
            format!(" ⚠ offline — can't reach Nightscout{age} "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    let title = Line::from(spans);
    let p = Paragraph::new(title).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_current(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" current ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(e) = app.latest() else {
        f.render_widget(Paragraph::new("  no data in this window…"), inner);
        return;
    };
    let value = app.units.format(e.sgv);
    let color = color_for(e.sgv, app);
    let info = current_info(app, e);
    let big_w = bigfont::width(&value);

    // Big number when there's room; compact single line otherwise.
    if inner.height as usize >= bigfont::ROWS && inner.width >= big_w + 24 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(big_w + 3), Constraint::Min(0)])
            .split(inner);
        let big: Vec<Line> = bigfont::render(&value)
            .into_iter()
            .map(|l| {
                Line::from(Span::styled(
                    format!(" {l}"),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
            })
            .collect();
        f.render_widget(Paragraph::new(big), cols[0]);
        f.render_widget(Paragraph::new(info), cols[1]);
    } else {
        let mut lines = vec![Line::from(Span::styled(
            format!("  {}  {}", value, e.arrow()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))];
        lines.extend(info.into_iter().skip(1)); // drop the unit/arrow line (already shown)
        f.render_widget(Paragraph::new(lines), inner);
    }
}

/// The secondary info lines beside/below the current value: unit + arrow,
/// delta, forecast ETA, and the timestamp.
fn current_info<'a>(app: &App, e: &crate::nightscout::Entry) -> Vec<Line<'a>> {
    let delta = app
        .delta_mgdl()
        .map(|d| {
            let sign = if d >= 0.0 { "+" } else { "-" };
            format!("{}{}", sign, app.units.format(d.abs()))
        })
        .unwrap_or_else(|| "--".into());
    let stamp = fmt_time(e.date);
    let when = if app.view.is_live() {
        format!("as of {stamp}")
    } else {
        format!("window end · {stamp}")
    };

    // Textual range label — legible without relying on color.
    let range = crate::alert::from_value(e.sgv, &app.alerts).label();
    let mut lines = vec![
        Line::from(Span::styled(
            format!(" {}  {}", app.units.label(), e.arrow()),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {range}"),
            Style::default().fg(color_for(e.sgv, app)),
        )),
        Line::from(format!(" Δ {} {}", delta, app.units.label())),
    ];
    if let Some((rising, mins)) = app.prediction_eta(chrono::Utc::now().timestamp_millis()) {
        let (arrow, word, c) = if rising {
            ("↗", "high", app.theme.high)
        } else {
            ("↘", "low", app.theme.low)
        };
        lines.push(Line::from(Span::styled(
            format!(" {arrow} {word} in ~{mins} min"),
            Style::default().fg(c),
        )));
    }
    lines.push(Line::from(Span::styled(
        format!(" {when}"),
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn draw_graph(f: &mut Frame, area: Rect, app: &App) {
    let title = format!(
        " {} → {} ",
        fmt_time(app.view_start),
        fmt_time(app.view_end)
    );
    let block = Block::default().borders(Borders::ALL).title(title);

    if app.entries.is_empty() {
        f.render_widget(
            Paragraph::new("  no readings in this window…")
                .block(block)
                .alignment(Alignment::Left),
            area,
        );
        return;
    }

    // x = real timestamp (ms), y = value in current units.
    let points: Vec<(f64, f64)> = app
        .entries
        .iter()
        .rev()
        .map(|e| (e.date as f64, app.units.from_mgdl(e.sgv)))
        .collect();

    // Forecast series, anchored to the latest actual reading for continuity.
    let pred: Vec<(f64, f64)> = if app.predictions.is_empty() {
        Vec::new()
    } else {
        let anchor = app
            .latest()
            .map(|e| (e.date as f64, app.units.from_mgdl(e.sgv)));
        anchor
            .into_iter()
            .chain(
                app.predictions
                    .iter()
                    .map(|(t, mgdl)| (*t as f64, app.units.from_mgdl(*mgdl))),
            )
            .collect()
    };

    let (min_y, max_y) = points
        .iter()
        .chain(pred.iter())
        .fold((f64::MAX, f64::MIN), |(lo, hi), (_, y)| {
            (lo.min(*y), hi.max(*y))
        });
    let pad = ((max_y - min_y) * 0.1).max(app.units.from_mgdl(10.0));
    let bounds_y = [min_y - pad, max_y + pad];
    // Anchor x to the requested window; extend right to cover any forecast.
    let right = pred
        .last()
        .map(|(x, _)| *x as i64)
        .unwrap_or(app.view_end)
        .max(app.view_end);
    let bounds_x = [app.view_start as f64, right as f64];
    let mid_x = (app.view_start + right) / 2;

    let (marker, gtype) = match app.graph_style {
        GraphStyle::Line => (symbols::Marker::Braille, GraphType::Line),
        GraphStyle::Dots => (symbols::Marker::Dot, GraphType::Scatter),
        GraphStyle::Blocks => (symbols::Marker::Block, GraphType::Scatter),
    };
    let mut datasets = vec![Dataset::default()
        .marker(marker)
        .graph_type(gtype)
        .style(Style::default().fg(app.theme.graph))
        .data(&points)];
    if !pred.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(app.theme.prediction))
                .data(&pred),
        );
    }

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds(bounds_x).labels(vec![
            Span::raw(fmt_time(app.view_start)),
            Span::raw(fmt_time(mid_x)),
            Span::raw(fmt_time(right)),
        ]))
        .y_axis(Axis::default().bounds(bounds_y).labels(vec![
            Span::raw(format!("{:.1}", bounds_y[0])),
            Span::raw(format!("{:.1}", bounds_y[1])),
        ]));
    f.render_widget(chart, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(buf) = &app.date_input {
        let line = Line::from(vec![
            Span::styled(
                " jump to date (YYYY-MM-DD): ",
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(buf.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            Span::raw("  · enter confirm · esc cancel"),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let text = match &app.last_error {
        Some(err) => Span::styled(format!(" error: {err} "), Style::default().fg(Color::Red)),
        None if app.perm_warning => Span::styled(
            " ⚠ config.toml is readable by others — run: chmod 600 ~/.config/sugarrush/config.toml ",
            Style::default().fg(Color::Yellow),
        ),
        None => {
            let mut s = String::from(
                " q quit · r refresh · u units · h/l pan · +/- zoom · g date · f live · s settings",
            );
            if app.sites.len() > 1 {
                s.push_str(" · n site");
            }
            if app.minimap_enabled {
                s.push_str(" · drag overview");
            }
            if app.alarm_active(chrono::Utc::now().timestamp_millis()) {
                s.push_str(" · a snooze");
            }
            s.push(' ');
            Span::raw(s)
        }
    };
    f.render_widget(Paragraph::new(Line::from(text)), area);
}

/// Format an epoch-ms timestamp as local `MM-DD HH:MM`.
fn fmt_time(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(dt) => dt.format("%m-%d %H:%M").to_string(),
        None => "--".into(),
    }
}

/// Colour a reading by configured thresholds and theme.
fn color_for(sgv: f64, app: &App) -> Color {
    let a = &app.alerts;
    let t = &app.theme;
    if sgv <= a.urgent_low || sgv >= a.urgent_high {
        t.urgent
    } else if sgv < a.low {
        t.low
    } else if sgv > a.high {
        t.high
    } else {
        t.in_range
    }
}
