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

use crate::app::App;

pub fn draw(f: &mut Frame, app: &App) {
    // A one-line alert banner appears above the header only while alerting.
    let banner = app.alert.is_alerting();
    let mut constraints = Vec::new();
    if banner {
        constraints.push(Constraint::Length(1)); // banner
    }
    constraints.extend([
        Constraint::Length(3), // header
        Constraint::Length(7), // current reading
        Constraint::Min(8),    // graph
        Constraint::Length(1), // footer
    ]);
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
    draw_graph(f, chunks[i + 2], app);
    draw_footer(f, chunks[i + 3], app);
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
    let title = Line::from(vec![
        Span::styled(
            " sugarrush ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("· {} · {} ", app.units.label(), app.view.span.label())),
        mode,
    ]);
    let p = Paragraph::new(title).block(Block::default().borders(Borders::ALL));
    f.render_widget(p, area);
}

fn draw_current(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" current ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = match app.latest() {
        Some(e) => {
            let value = app.units.format(e.sgv);
            let delta = app
                .delta_mgdl()
                .map(|d| {
                    let sign = if d >= 0.0 { "+" } else { "-" };
                    format!("{}{}", sign, app.units.format(d.abs()))
                })
                .unwrap_or_else(|| "--".into());
            let stamp = fmt_time(e.date);
            let when = if app.view.is_live() {
                format!("  as of {stamp}")
            } else {
                format!("  window end · {stamp}")
            };
            vec![
                Line::from(Span::styled(
                    format!("  {}  {}", value, e.arrow()),
                    Style::default()
                        .fg(color_for(e.sgv, app))
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(format!("  Δ {} {}", delta, app.units.label())),
                Line::from(Span::styled(when, Style::default().fg(Color::DarkGray))),
            ]
        }
        None => vec![Line::from("  no data in this window…")],
    };
    f.render_widget(Paragraph::new(text), inner);
}

fn draw_graph(f: &mut Frame, area: Rect, app: &App) {
    let title = format!(" {} → {} ", fmt_time(app.view_start), fmt_time(app.view_end));
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

    let mut datasets = vec![Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&points)];
    if !pred.is_empty() {
        datasets.push(
            Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Magenta))
                .data(&pred),
        );
    }

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(
            Axis::default().bounds(bounds_x).labels(vec![
                Span::raw(fmt_time(app.view_start)),
                Span::raw(fmt_time(mid_x)),
                Span::raw(fmt_time(right)),
            ]),
        )
        .y_axis(
            Axis::default().bounds(bounds_y).labels(vec![
                Span::raw(format!("{:.1}", bounds_y[0])),
                Span::raw(format!("{:.1}", bounds_y[1])),
            ]),
        );
    f.render_widget(chart, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    if let Some(buf) = &app.date_input {
        let line = Line::from(vec![
            Span::styled(" jump to date (YYYY-MM-DD): ", Style::default().fg(Color::Cyan)),
            Span::styled(buf.clone(), Style::default().add_modifier(Modifier::BOLD)),
            Span::styled("_", Style::default().add_modifier(Modifier::SLOW_BLINK)),
            Span::raw("  · enter confirm · esc cancel"),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let text = match &app.last_error {
        Some(err) => Span::styled(format!(" error: {err} "), Style::default().fg(Color::Red)),
        None => Span::raw(" q quit · r refresh · u units · h/l pan · +/- zoom · g date · f live "),
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

/// Colour a reading by rough range (thresholds in mg/dL).
fn color_for(sgv: f64, _app: &App) -> Color {
    if sgv < 70.0 {
        Color::Red
    } else if sgv > 180.0 {
        Color::Yellow
    } else {
        Color::Green
    }
}
