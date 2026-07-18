//! Rendering. v1: a single dashboard screen.

use chrono::{Local, TimeZone};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Clear, Dataset, GraphType, Paragraph, Tabs},
    Frame,
};

use crate::agp;
use crate::app::{App, Field, GraphView, Screen};
use crate::bigfont;
use crate::config::GraphStyle;
use crate::stats;
use crate::units::Units;

/// Format an already-in-display-units value: integer for mg/dL, one decimal for
/// mmol/L (mg/dL never has a fractional part).
fn fmt_disp(units: Units, v: f64) -> String {
    match units {
        Units::Mgdl => format!("{v:.0}"),
        Units::Mmol => format!("{v:.1}"),
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    if app.screen == Screen::Settings {
        draw_settings(f, app);
        return;
    }
    // A one-line alert banner appears above the header only while alerting.
    let banner = app.alert.is_alerting();
    let minimap = app.minimap_enabled;
    // On wide terminals, current + stats share one row (side-by-side),
    // reclaiming ~5 rows for the graph. Otherwise they stack.
    let wide = f.area().width >= 90;

    let mut constraints = Vec::new();
    if banner {
        constraints.push(Constraint::Length(1)); // banner
    }
    constraints.push(Constraint::Length(3)); // header
    if wide {
        constraints.push(Constraint::Length(8)); // current + stats
    } else {
        constraints.push(Constraint::Length(8)); // current
        constraints.push(Constraint::Length(5)); // stats
    }
    constraints.push(Constraint::Min(8)); // graph
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
    i += 1;
    if wide {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[i]);
        draw_current(f, cols[0], app);
        draw_stats(f, cols[1], app);
        i += 1;
    } else {
        draw_current(f, chunks[i], app);
        draw_stats(f, chunks[i + 1], app);
        i += 2;
    }
    draw_graph_pane(f, chunks[i], app);
    i += 1;
    if minimap {
        draw_minimap(f, chunks[i], app);
        i += 1;
    }
    draw_footer(f, chunks[i], app);

    if app.show_help {
        draw_help(f, f.area());
    }
}

/// A centered keybinding cheatsheet, drawn over the dashboard. Dismissed by any
/// key. Reachable with `?` — the discoverable home for every binding, so the
/// footer can shrink on narrow terminals without hiding functionality.
fn draw_help(f: &mut Frame, area: Rect) {
    let rows = [
        ("q / Esc", "quit"),
        ("?", "toggle this help"),
        ("r", "refresh now"),
        ("u", "toggle mg/dL ↔ mmol/L"),
        ("Tab / ⇧Tab", "switch graph view (3h / 24h / AGP)"),
        ("h / l · ← / →", "pan back / forward"),
        ("+ / -", "zoom window (1h–24h)"),
        ("g", "jump to a date"),
        ("f / Home", "return to live"),
        ("a", "snooze the audible alarm"),
        ("n", "switch site (multi-site)"),
        ("s", "open / close settings"),
    ];
    let w = 52u16.min(area.width.saturating_sub(2));
    let h = (rows.len() as u16 + 4).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect::new(x, y, w, h);

    let mut lines = vec![Line::from("")];
    for (k, d) in rows {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {k:<14}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(d),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" keybindings "),
        ),
        popup,
    );
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
    // Time-in-range as a stacked zone bar with the in-range % alongside.
    let tir_line = match stats::tir(&app.entries, app.alerts.low, app.alerts.high) {
        Some(t) => {
            let bar_w = (inner.width as usize).saturating_sub(22).clamp(8, 40);
            let rc = (t.low / 100.0 * bar_w as f64).round() as usize;
            let yc = (t.high / 100.0 * bar_w as f64).round() as usize;
            let gc = bar_w.saturating_sub(rc + yc);
            Line::from(vec![
                Span::raw("  TIR "),
                Span::styled("█".repeat(rc), Style::default().fg(app.theme.low)),
                Span::styled("█".repeat(gc), Style::default().fg(app.theme.in_range)),
                Span::styled("█".repeat(yc), Style::default().fg(app.theme.high)),
                Span::styled(
                    format!(" {:.0}% in range", t.in_range),
                    Style::default().fg(app.theme.in_range),
                ),
            ])
        }
        None => Line::from("  TIR  —"),
    };

    // Mean + a sparkline of recent readings + estimated A1c.
    let avg_line = match stats::mean_mgdl(&app.entries) {
        Some(mean) => {
            // Newest-first entries → oldest→newest for the sparkline.
            let mut spark: Vec<f64> = app.entries.iter().take(16).map(|e| e.sgv).collect();
            spark.reverse();
            Line::from(vec![
                Span::raw(format!("  avg  {} {}  ", u.format(mean), u.label())),
                Span::styled(sparkline_str(&spark), Style::default().fg(app.theme.graph)),
                Span::raw(format!("  ·  GMI {:.1}%", stats::gmi(mean))),
            ])
        }
        None => Line::from("  avg  —"),
    };

    // Device / uploader status. IOB/COB are the clinically actionable numbers,
    // so give them foreground weight; the device/battery/uploader stay dim.
    let now = chrono::Utc::now().timestamp_millis();
    let strong = Style::default().add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let mut spans: Vec<Span> = Vec::new();
    if let Some(iob) = app.device.iob {
        spans.push(Span::styled(format!("  IOB {iob:.1}U"), strong));
    }
    if let Some(cob) = app.device.cob {
        spans.push(Span::styled(format!("   COB {cob:.0}g"), strong));
    }
    let mut rest = Vec::new();
    if let Some(name) = &app.device.device {
        rest.push(name.clone());
    }
    if let Some(b) = app.device.battery {
        rest.push(format!("battery {b}%"));
    }
    if let Some(start) = app.sensor_start_ms {
        rest.push(format!("sensor {}", fmt_age(now - start)));
    }
    if let Some(last) = app.device.last_ms {
        rest.push(format!("uploader {} ago", fmt_age(now - last)));
    }
    if !rest.is_empty() {
        let prefix = if spans.is_empty() { "  " } else { "   ·   " };
        spans.push(Span::styled(
            format!("{prefix}{}", rest.join("   ·   ")),
            dim,
        ));
    }
    let dev_line = if spans.is_empty() {
        Line::from(Span::styled("  device  —", dim))
    } else {
        Line::from(spans)
    };

    f.render_widget(Paragraph::new(vec![tir_line, avg_line, dev_line]), inner);
}

/// An 8-level block sparkline over the values (min→max normalized).
fn sparkline_str(values: &[f64]) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() {
        return String::new();
    }
    let (min, max) = values
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)));
    let range = (max - min).max(1.0);
    values
        .iter()
        .map(|&v| {
            let level = ((v - min) / range * (BARS.len() - 1) as f64).round() as usize;
            BARS[level.min(BARS.len() - 1)]
        })
        .collect()
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

    // Build display rows: a dim section header whenever the group changes,
    // then each field. Headers aren't selectable — navigation stays over
    // Field::ALL, so `settings_sel` still indexes fields directly.
    enum Row {
        Header(&'static str),
        Field(usize, Field),
    }
    let mut display: Vec<Row> = Vec::new();
    let mut last_group = "";
    for (i, &field) in Field::ALL.iter().enumerate() {
        let g = field.group();
        if g != last_group {
            display.push(Row::Header(g));
            last_group = g;
        }
        display.push(Row::Field(i, field));
    }

    // Scroll so the selected field (and ideally its header) stays visible.
    let height = inner.height.max(1) as usize;
    let sel_display = display
        .iter()
        .position(|r| matches!(r, Row::Field(i, _) if *i == app.settings_sel))
        .unwrap_or(0);
    let offset = if sel_display < height {
        0
    } else {
        (sel_display + 1 - height).min(display.len().saturating_sub(height))
    };

    let lines: Vec<Line> = display
        .iter()
        .skip(offset)
        .take(height)
        .map(|row| match row {
            Row::Header(name) => Line::from(Span::styled(
                format!(" {name}"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Row::Field(i, field) => {
                let selected = *i == app.settings_sel;
                let marker = if selected { " ▸ " } else { "   " };
                let style = if selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                let text = format!("{marker}{:<26}{}", field.label(), app.field_value(*field));
                // Pad the selected row to full width so its highlight fills the
                // line instead of ending ragged mid-text.
                let text = if selected {
                    format!("{text:<width$}", width = inner.width as usize)
                } else {
                    text
                };
                Line::from(Span::styled(text, style))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(lines), inner);

    let footer = match &app.status {
        Some(msg) => Span::styled(format!(" {msg} "), Style::default().fg(Color::Green)),
        None => Span::raw(" ↑/↓ select · ←/→ change · w save · s/esc back · q quit "),
    };
    f.render_widget(Paragraph::new(Line::from(footer)), chunks[2]);
}

fn draw_banner(f: &mut Frame, area: Rect, app: &App) {
    use crate::alert::Alert;
    // Route the banner background through the theme so the colourblind preset
    // (and any custom palette) recolours the most safety-critical widget.
    let color = match app.alert {
        Alert::UrgentLow | Alert::UrgentHigh => app.theme.urgent,
        Alert::Low => app.theme.low,
        Alert::High => app.theme.high,
        Alert::Stale => Color::Magenta,
        Alert::InRange => app.theme.in_range,
    };
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
        // Show the actual cause (auth vs unreachable) rather than a blanket
        // "offline", so a bad token doesn't send the user to debug the network.
        let msg = app
            .last_error
            .clone()
            .unwrap_or_else(|| "can't reach Nightscout".to_string());
        spans.push(Span::styled(
            format!(" ⚠ {msg}{age} "),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    // Standing reminder in the daily-use surface (it's disclaimed in the README
    // and `about`, but a user who only ever runs the TUI should see it too).
    spans.push(Span::styled(
        " · not a medical device",
        Style::default().fg(Color::DarkGray),
    ));
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

    // Reserve a bottom row for the range bar when there's height to spare.
    let (content, bar_area) = if inner.height >= 6 {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(4), Constraint::Length(1)])
            .split(inner);
        (rows[0], Some(rows[1]))
    } else {
        (inner, None)
    };

    let value = app.units.format(e.sgv);
    let color = color_for(e.sgv, app);
    let info = current_info(app, e);
    let big_w = bigfont::width(&value);

    // Big number when there's room; compact single line otherwise.
    if content.height as usize >= bigfont::ROWS && content.width >= big_w + 24 {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(big_w + 3), Constraint::Min(0)])
            .split(content);
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
        f.render_widget(Paragraph::new(lines), content);
    }

    if let Some(ba) = bar_area {
        f.render_widget(Paragraph::new(range_bar(app, e.sgv, ba.width)), ba);
    }
}

/// A one-row zoned range bar: `low ━━━●━━━ high` (display units), coloured by
/// zone with a marker at the current value.
fn range_bar<'a>(app: &App, sgv: f64, width: u16) -> Line<'a> {
    let u = app.units;
    let lo = u.from_mgdl(app.alerts.urgent_low);
    let hi = u.from_mgdl(app.alerts.urgent_high);
    let low = u.from_mgdl(app.alerts.low);
    let high = u.from_mgdl(app.alerts.high);
    let ulow = u.from_mgdl(app.alerts.urgent_low);
    let uhigh = u.from_mgdl(app.alerts.urgent_high);
    let lo_s = fmt_disp(u, lo);
    let hi_s = fmt_disp(u, hi);
    // ` <lo> ` + bar + ` <hi>`
    let used = lo_s.len() + hi_s.len() + 3;
    let cells = (width as usize).saturating_sub(used);
    if cells < 6 {
        return Line::from("");
    }
    let span = (hi - lo).max(0.1);
    let cur = u.from_mgdl(sgv);
    let marker = (((cur - lo) / span) * (cells as f64 - 1.0)).round();
    let marker = marker.clamp(0.0, cells as f64 - 1.0) as usize;

    let mut spans = vec![Span::styled(
        format!(" {lo_s} "),
        Style::default().fg(Color::DarkGray),
    )];
    for i in 0..cells {
        if i == marker {
            spans.push(Span::styled(
                "●",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        let v = lo + (i as f64 / (cells as f64 - 1.0)) * span;
        // Four zones: the bar spans urgent-low → urgent-high, so tint the
        // extremes urgent, then low / in-range / high between the thresholds.
        let c = if v <= ulow || v >= uhigh {
            app.theme.urgent
        } else if v < low {
            app.theme.low
        } else if v > high {
            app.theme.high
        } else {
            app.theme.in_range
        };
        spans.push(Span::styled("━", Style::default().fg(c)));
    }
    spans.push(Span::styled(
        format!(" {hi_s}"),
        Style::default().fg(Color::DarkGray),
    ));
    Line::from(spans)
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
        // Includes the value as plain text so the reading is readable when the
        // big-number layout draws it as block glyphs (screen readers, tmux
        // copy, braille). The compact layout skips this line — it shows the
        // value itself already.
        Line::from(Span::styled(
            format!(
                " {} {}  {}",
                app.units.format(e.sgv),
                app.units.label(),
                e.arrow()
            ),
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

/// The graph pane: a tab bar to pick the view, then the chosen chart below it.
fn draw_graph_pane(f: &mut Frame, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(3)])
        .split(area);
    draw_graph_tabs(f, rows[0], app);
    match app.graph_view {
        GraphView::Agp => draw_agp(f, rows[1], app),
        _ => draw_graph(f, rows[1], app),
    }
}

/// The 3h / 24h / AGP selector above the graph.
fn draw_graph_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = GraphView::ALL
        .iter()
        .map(|v| Line::from(v.label()))
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.graph_view.index())
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(app.theme.graph)
                .add_modifier(Modifier::BOLD),
        )
        .divider(symbols::DOT);
    f.render_widget(tabs, area);
}

/// Ambulatory Glucose Profile: readings from the last N days folded onto one
/// 24-hour clock, drawn as a percentile fan (median + 25/75 + 5/95 bands).
fn draw_agp(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(format!(
        " AGP · last {}d · target {}–{} {} · median + IQR + 5/95 ",
        app.agp_days,
        fmt_disp(app.units, app.units.from_mgdl(app.alerts.low)),
        fmt_disp(app.units, app.units.from_mgdl(app.alerts.high)),
        app.units.label(),
    ));

    let bands = agp::profile(&app.agp_entries);
    if bands.is_empty() {
        f.render_widget(
            Paragraph::new("  gathering days of history…").block(block),
            area,
        );
        return;
    }

    let conv = |mgdl: f64| app.units.from_mgdl(mgdl);
    // Median line (the only line drawn; the fan is a background tint).
    let p50: Vec<(f64, f64)> = bands
        .iter()
        .map(|b| (b.minute as f64, conv(b.p50)))
        .collect();

    let low_y = conv(app.alerts.low);
    let high_y = conv(app.alerts.high);
    let (min_y, max_y) = bands.iter().fold((f64::MAX, f64::MIN), |(lo, hi), b| {
        (lo.min(conv(b.p05)), hi.max(conv(b.p95)))
    });
    let (min_y, max_y) = (min_y.min(low_y), max_y.max(high_y));
    let pad = ((max_y - min_y) * 0.1).max(conv(10.0));
    let bounds_y = [min_y - pad, max_y + pad];
    let bounds_x = [0.0, 1440.0];

    let low_rail = [(0.0, low_y), (1440.0, low_y)];
    let high_rail = [(0.0, high_y), (1440.0, high_y)];

    // Only the median is a line; the 5–95 and 25–75 bands are a background fan.
    let median = Style::default()
        .fg(app.theme.graph)
        .add_modifier(Modifier::BOLD);
    let datasets = vec![
        braille_line(&low_rail, Style::default().fg(Color::DarkGray)),
        braille_line(&high_rail, Style::default().fg(Color::DarkGray)),
        braille_line(&p50, median),
    ];

    let lo_lab = fmt_disp(app.units, bounds_y[0]);
    let hi_lab = fmt_disp(app.units, bounds_y[1]);
    let gutter = chart_gutter(&[&lo_lab, &hi_lab], "00:00");

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds(bounds_x).labels(vec![
            Span::raw("00:00"),
            Span::raw("06:00"),
            Span::raw("12:00"),
            Span::raw("18:00"),
            Span::raw("24:00"),
        ]))
        .y_axis(
            Axis::default()
                .bounds(bounds_y)
                .labels(vec![Span::raw(lo_lab), Span::raw(hi_lab)]),
        );
    f.render_widget(chart, area);
    tint_agp_fan(f, area, bounds_y, gutter, &bands, &conv, app.theme.graph);
}

/// A braille line dataset over `data`, styled. Free fn so the borrow of `data`
/// carries into the returned `Dataset` (a closure can't express that).
fn braille_line(data: &[(f64, f64)], style: Style) -> Dataset<'_> {
    Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(style)
        .data(data)
}

fn draw_graph(f: &mut Frame, area: Rect, app: &App) {
    // Title carries a small legend for the treatment markers, which are
    // otherwise unlabelled dots on the graph.
    let mut title = vec![Span::raw(format!(
        " {} → {} ",
        fmt_time(app.view_start),
        fmt_time(app.view_end)
    ))];
    if app.treatments.iter().any(|t| t.carbs.is_some()) {
        title.push(Span::styled(
            "· ● carbs ",
            Style::default().fg(Color::Yellow),
        ));
    }
    if app.treatments.iter().any(|t| t.insulin.is_some()) {
        title.push(Span::styled("· ● bolus ", Style::default().fg(Color::Blue)));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(title));

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

    // Forecast cone, anchored to the latest actual reading so it emanates from
    // the current dot. The whole band is shifted so its first centre sits on the
    // reading — uploader curves already start near the current value (shift ≈ 0),
    // so this mainly straightens the AR2 fallback's initial jump. Band width
    // (the uncertainty) is preserved.
    let (mut pred_center, mut pred_low, mut pred_high) = (Vec::new(), Vec::new(), Vec::new());
    if let (Some(e), Some(first)) = (app.latest(), app.predictions.first()) {
        let anchor_y = app.units.from_mgdl(e.sgv);
        let first_mid = app.units.from_mgdl((first.low + first.high) / 2.0);
        // Nudge the forecast so its *start* sits on the current reading, but let
        // the correction decay to zero across the horizon — so the fan emanates
        // from the dot yet still reaches the model's true endpoint (the AR2
        // fallback amplifies the recent trend on its first step; a constant
        // shift would drag the whole projection off). Uploader curves start near
        // the current value, so the correction is tiny for them anyway.
        let shift = anchor_y - first_mid;
        let n = app.predictions.len();
        let a = (e.date as f64, anchor_y);
        pred_center.push(a);
        pred_low.push(a);
        pred_high.push(a);
        for (j, p) in app.predictions.iter().enumerate() {
            let decay = if n > 1 {
                (n - 1 - j) as f64 / (n - 1) as f64
            } else {
                0.0
            };
            let s = shift * decay;
            let t = p.at_ms as f64;
            let lo = app.units.from_mgdl(p.low) + s;
            let hi = app.units.from_mgdl(p.high) + s;
            pred_low.push((t, lo));
            pred_high.push((t, hi));
            pred_center.push((t, (lo + hi) / 2.0));
        }
    }

    // Threshold rails (in display units) — always kept in view for reference.
    let low_y = app.units.from_mgdl(app.alerts.low);
    let high_y = app.units.from_mgdl(app.alerts.high);
    let (min_y, max_y) = points
        .iter()
        .chain(pred_low.iter())
        .chain(pred_high.iter())
        .fold((f64::MAX, f64::MIN), |(lo, hi), (_, y)| {
            (lo.min(*y), hi.max(*y))
        });
    let (min_y, max_y) = (min_y.min(low_y), max_y.max(high_y));
    let pad = ((max_y - min_y) * 0.1).max(app.units.from_mgdl(10.0));
    let bounds_y = [min_y - pad, max_y + pad];
    // Anchor x to the requested window; extend right to cover any forecast.
    let right = app
        .predictions
        .last()
        .map(|p| p.at_ms)
        .unwrap_or(app.view_end)
        .max(app.view_end);
    let bounds_x = [app.view_start as f64, right as f64];
    let mid_x = (app.view_start + right) / 2;

    // A dim vertical rule at the latest reading marks the boundary between
    // actual readings and the forecast — only when it's within the window.
    let now_line = app
        .latest()
        .map(|e| e.date as f64)
        .filter(|x| *x >= app.view_start as f64 && *x <= right as f64)
        .map(|x| [(x, bounds_y[0]), (x, bounds_y[1])]);

    // Treatment markers along the bottom: carbs and boluses on separate rows.
    let span_y = (bounds_y[1] - bounds_y[0]).max(1.0);
    let carb_pts: Vec<(f64, f64)> = app
        .treatments
        .iter()
        .filter(|t| t.carbs.is_some())
        .map(|t| (t.at_ms as f64, bounds_y[0] + span_y * 0.02))
        .collect();
    let bolus_pts: Vec<(f64, f64)> = app
        .treatments
        .iter()
        .filter(|t| t.insulin.is_some())
        .map(|t| (t.at_ms as f64, bounds_y[0] + span_y * 0.08))
        .collect();

    let (marker, gtype) = match app.graph_style {
        GraphStyle::Line => (symbols::Marker::Braille, GraphType::Line),
        GraphStyle::Dots => (symbols::Marker::Dot, GraphType::Scatter),
        GraphStyle::Blocks => (symbols::Marker::Block, GraphType::Scatter),
    };

    // Dim reference rails at the low/high thresholds (drawn under everything).
    let low_rail = [(app.view_start as f64, low_y), (right as f64, low_y)];
    let high_rail = [(app.view_start as f64, high_y), (right as f64, high_y)];

    // In scatter modes, colour readings by zone. A connected line can't change
    // colour mid-segment, so line mode keeps a single colour.
    let scatter = !matches!(app.graph_style, GraphStyle::Line);
    let (mut low_z, mut in_z, mut high_z) = (Vec::new(), Vec::new(), Vec::new());
    if scatter {
        for e in app.entries.iter().rev() {
            let p = (e.date as f64, app.units.from_mgdl(e.sgv));
            if e.sgv < app.alerts.low {
                low_z.push(p);
            } else if e.sgv > app.alerts.high {
                high_z.push(p);
            } else {
                in_z.push(p);
            }
        }
    }

    let mut datasets = vec![
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::DarkGray))
            .data(&low_rail),
        Dataset::default()
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::DarkGray))
            .data(&high_rail),
    ];
    if scatter {
        for (pts, color) in [
            (&in_z, app.theme.in_range),
            (&low_z, app.theme.low),
            (&high_z, app.theme.high),
        ] {
            if !pts.is_empty() {
                datasets.push(
                    Dataset::default()
                        .marker(marker)
                        .graph_type(gtype)
                        .style(Style::default().fg(color))
                        .data(pts),
                );
            }
        }
    } else {
        datasets.push(
            Dataset::default()
                .marker(marker)
                .graph_type(gtype)
                .style(Style::default().fg(app.theme.graph))
                .data(&points),
        );
    }
    if let Some(nl) = &now_line {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::DarkGray))
                .data(nl),
        );
    }
    if !carb_pts.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Dot)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(Color::Yellow))
                .data(&carb_pts),
        );
    }
    if !bolus_pts.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Dot)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(Color::Blue))
                .data(&bolus_pts),
        );
    }
    // Forecast cone: the low–high band is a filled tint (below); draw only the
    // bright centre line on top.
    if !pred_center.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(app.theme.prediction))
                .data(&pred_center),
        );
    }

    let lo_lab = fmt_disp(app.units, bounds_y[0]);
    let hi_lab = fmt_disp(app.units, bounds_y[1]);
    let first_x = fmt_time(app.view_start);
    let gutter = chart_gutter(&[&lo_lab, &hi_lab], &first_x);

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds(bounds_x).labels(vec![
            Span::raw(first_x.clone()),
            Span::raw(fmt_time(mid_x)),
            Span::raw(fmt_time(right)),
        ]))
        .y_axis(
            Axis::default()
                .bounds(bounds_y)
                .labels(vec![Span::raw(lo_lab), Span::raw(hi_lab)]),
        );
    f.render_widget(chart, area);
    tint_in_range_band(f, area, bounds_y, gutter, low_y, high_y, app.theme.in_range);
    // Fill the forecast cone's low–high band, leaving the centre line on top.
    if pred_low.len() > 1 {
        tint_band(
            f,
            area,
            (bounds_x, bounds_y),
            gutter,
            &pred_low,
            &pred_high,
            tint_bg(app.theme.prediction, 0.32),
        );
    }
}

/// Approximate RGB for a `Color`, so background tints can be derived from the
/// (possibly named / colourblind) palette rather than hardcoded.
fn rgb_of(c: Color) -> (u8, u8, u8) {
    match c {
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Red => (205, 60, 55),
        Color::LightRed => (240, 100, 95),
        Color::Green => (40, 170, 95),
        Color::LightGreen => (90, 220, 130),
        Color::Yellow => (200, 170, 40),
        Color::LightYellow => (235, 215, 90),
        Color::Blue => (60, 110, 210),
        Color::LightBlue => (110, 170, 235),
        Color::Magenta => (185, 90, 175),
        Color::LightMagenta => (225, 130, 215),
        Color::Cyan => (40, 175, 180),
        Color::LightCyan => (110, 220, 225),
        Color::White | Color::Gray => (200, 205, 210),
        _ => (150, 155, 160),
    }
}

/// A dark background tint derived from a foreground colour (scaled toward
/// black), so a shaded zone tracks the active palette.
fn tint_bg(c: Color, scale: f32) -> Color {
    let (r, g, b) = rgb_of(c);
    Color::Rgb(
        (r as f32 * scale) as u8,
        (g as f32 * scale) as u8,
        (b as f32 * scale) as u8,
    )
}

/// The widest left-gutter reservation a `Chart` makes for its y-axis, matching
/// ratatui: the max y-label width, but at least the first (left-aligned) x-label
/// overhanging left of the y-axis by all but its last character.
fn chart_gutter(y_labels: &[&str], first_x_label: &str) -> u16 {
    let ymax = y_labels
        .iter()
        .map(|s| s.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let x_overhang = (first_x_label.chars().count() as u16).saturating_sub(1);
    ymax.max(x_overhang)
}

/// Geometry of a `Chart`'s plotting rect inside `area`, replicating ratatui's
/// layout so background tints line up with the chart's own lines/points:
/// exclude the block border, the left gutter + a y-axis line column, and the
/// bottom two rows (the x-axis line and its labels).
struct Plot {
    x0: u16,
    x1: u16,
    top: u16,
    bot: u16,
    bounds_y: [f64; 2],
}

impl Plot {
    fn new(area: Rect, bounds_y: [f64; 2], gutter: u16) -> Option<Self> {
        let inner = area.inner(Margin::new(1, 1));
        let x0 = inner.x.saturating_add(gutter + 1); // gutter + y-axis line
        let x1 = inner.x + inner.width;
        let top = inner.y;
        let bot = inner.y + inner.height.saturating_sub(3); // x-axis line + labels
        (bot > top && x1 > x0).then_some(Self {
            x0,
            x1,
            top,
            bot,
            bounds_y,
        })
    }
    fn row_of(&self, v: f64) -> u16 {
        let ph = (self.bot - self.top) as f64;
        let yspan = (self.bounds_y[1] - self.bounds_y[0]).max(0.001);
        let r = ((self.bounds_y[1] - v) / yspan * ph).round() as i32;
        (self.top as i32 + r).clamp(self.top as i32, self.bot as i32) as u16
    }
}

/// Shade the in-range y-band by tinting the plot cells' background — a clean
/// solid band that `Chart` can't paint directly. Runs after the chart so
/// readings keep their foreground colour on the tint. The tint is derived from
/// the in-range palette colour so it survives theming / the colourblind preset.
fn tint_in_range_band(
    f: &mut Frame,
    area: Rect,
    bounds_y: [f64; 2],
    gutter: u16,
    low_y: f64,
    high_y: f64,
    in_range: Color,
) {
    let Some(plot) = Plot::new(area, bounds_y, gutter) else {
        return;
    };
    let (y0, y1) = (plot.row_of(high_y), plot.row_of(low_y));
    let band = tint_bg(in_range, 0.20);
    let buf = f.buffer_mut();
    for yy in y0..=y1 {
        for xx in plot.x0..plot.x1 {
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                cell.set_bg(band);
            }
        }
    }
}

/// Interpolate the `y` of a sorted `(x, y)` series at `x` (clamped to the ends).
fn interp_xy(pts: &[(f64, f64)], x: f64) -> f64 {
    match pts.iter().position(|p| p.0 >= x) {
        Some(0) => pts[0].1,
        Some(i) => {
            let (a, b) = (pts[i - 1], pts[i]);
            let span = (b.0 - a.0).max(1.0);
            a.1 + (b.1 - a.1) * ((x - a.0) / span)
        }
        None => pts.last().map(|p| p.1).unwrap_or(0.0),
    }
}

/// Fill the band between a `low` and `high` `(x_data, y_display)` series by
/// tinting cell backgrounds per column — used for the forecast cone. `x_data`
/// is in the chart's x-bounds space (epoch ms); columns outside the series'
/// x-range are left untouched, so only the forecast region is shaded.
fn tint_band(
    f: &mut Frame,
    area: Rect,
    bounds: ([f64; 2], [f64; 2]),
    gutter: u16,
    low: &[(f64, f64)],
    high: &[(f64, f64)],
    bg: Color,
) {
    let (bounds_x, bounds_y) = bounds;
    let Some(plot) = Plot::new(area, bounds_y, gutter) else {
        return;
    };
    if low.len() < 2 || high.len() < 2 {
        return;
    }
    let (xmin, xmax) = (low[0].0, low[low.len() - 1].0);
    let xspan = (bounds_x[1] - bounds_x[0]).max(1.0);
    let pw = (plot.x1 - plot.x0).max(1) as f64;
    let buf = f.buffer_mut();
    for xx in plot.x0..plot.x1 {
        let x = bounds_x[0] + (xx - plot.x0) as f64 / pw * xspan;
        if x < xmin || x > xmax {
            continue;
        }
        let a = plot.row_of(interp_xy(high, x));
        let b = plot.row_of(interp_xy(low, x));
        for yy in a.min(b)..=a.max(b) {
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                cell.set_bg(bg);
            }
        }
    }
}

/// Fill the AGP percentile fan by tinting cell backgrounds per column: a light
/// outer 5–95 band and a darker inner 25–75 band, interpolated across the day.
/// Leaves the median line (drawn by the chart) crisp on top.
fn tint_agp_fan(
    f: &mut Frame,
    area: Rect,
    bounds_y: [f64; 2],
    gutter: u16,
    bands: &[agp::Band],
    conv: &dyn Fn(f64) -> f64,
    base: Color,
) {
    let Some(plot) = Plot::new(area, bounds_y, gutter) else {
        return;
    };
    if bands.len() < 2 {
        return;
    }
    let outer = tint_bg(base, 0.16);
    let inner = tint_bg(base, 0.34);
    // Linear-interpolate a percentile curve at `minute` from the sparse buckets.
    let at = |minute: f64, pick: &dyn Fn(&agp::Band) -> f64| -> f64 {
        match bands.iter().position(|b| b.minute as f64 >= minute) {
            Some(0) => conv(pick(&bands[0])),
            Some(i) => {
                let (a, b) = (&bands[i - 1], &bands[i]);
                let span = (b.minute - a.minute).max(1) as f64;
                let t = (minute - a.minute as f64) / span;
                conv(pick(a)) + (conv(pick(b)) - conv(pick(a))) * t
            }
            None => conv(pick(&bands[bands.len() - 1])),
        }
    };
    let pw = (plot.x1 - plot.x0).max(1) as f64;
    let buf = f.buffer_mut();
    for xx in plot.x0..plot.x1 {
        let minute = (xx - plot.x0) as f64 / pw * 1440.0;
        let (o_lo, o_hi) = (
            plot.row_of(at(minute, &|b| b.p95)),
            plot.row_of(at(minute, &|b| b.p05)),
        );
        let (i_lo, i_hi) = (
            plot.row_of(at(minute, &|b| b.p75)),
            plot.row_of(at(minute, &|b| b.p25)),
        );
        for yy in o_lo..=o_hi {
            let c = if yy >= i_lo && yy <= i_hi {
                inner
            } else {
                outer
            };
            if let Some(cell) = buf.cell_mut((xx, yy)) {
                cell.set_bg(c);
            }
        }
    }
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
            // On a narrow footer the full hint line silently clips, hiding
            // settings/site/snooze; fall back to a terse set that always keeps
            // `? help` so nothing becomes undiscoverable.
            let alarm = app.alarm_active(chrono::Utc::now().timestamp_millis());
            let s = if area.width < 72 {
                let mut s = String::from(" q quit · tab view · s settings");
                if alarm {
                    s.push_str(" · a snooze");
                }
                s.push_str(" · ? help ");
                s
            } else {
                let mut s = if app.is_agp() {
                    String::from(" q quit · r refresh · u units · tab view · s settings")
                } else {
                    String::from(
                        " q quit · r refresh · u units · tab view · h/l pan · +/- zoom · g date · f live · s settings",
                    )
                };
                if app.sites.len() > 1 {
                    s.push_str(" · n site");
                }
                if app.minimap_enabled {
                    s.push_str(" · drag overview");
                }
                if alarm {
                    s.push_str(" · a snooze");
                }
                s.push_str(" · ? help ");
                s
            };
            Span::raw(s)
        }
    };
    // A visible acknowledgement that the safety alarm is silenced, with a
    // countdown, shown ahead of the normal footer content.
    let mut spans = Vec::new();
    if let Some(mins) = app.snooze_remaining_min(chrono::Utc::now().timestamp_millis()) {
        spans.push(Span::styled(
            format!(" ⏸ alarm snoozed · {mins}m left "),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans.push(text);
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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
