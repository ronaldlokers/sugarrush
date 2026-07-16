//! Rendering. v1: a single dashboard screen.

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
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(7), // current reading
            Constraint::Min(8),    // graph
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);
    draw_current(f, chunks[1], app);
    draw_graph(f, chunks[2], app);
    draw_footer(f, chunks[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let title = Line::from(vec![
        Span::styled(
            " sugarrush ",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("· {} ", app.units.label())),
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
            vec![
                Line::from(Span::styled(
                    format!("  {}  {}", value, e.arrow()),
                    Style::default()
                        .fg(color_for(e.sgv, app))
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(format!("  Δ {} {}", delta, app.units.label())),
            ]
        }
        None => vec![Line::from("  no data yet…")],
    };
    f.render_widget(Paragraph::new(text), inner);
}

fn draw_graph(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default().borders(Borders::ALL).title(" recent ");

    if app.entries.is_empty() {
        f.render_widget(
            Paragraph::new("  waiting for readings…")
                .block(block)
                .alignment(Alignment::Left),
            area,
        );
        return;
    }

    // Oldest -> newest, x = index, y = value in current units.
    let points: Vec<(f64, f64)> = app
        .entries
        .iter()
        .rev()
        .enumerate()
        .map(|(i, e)| (i as f64, app.units.from_mgdl(e.sgv)))
        .collect();

    let (min_y, max_y) = points.iter().fold((f64::MAX, f64::MIN), |(lo, hi), (_, y)| {
        (lo.min(*y), hi.max(*y))
    });
    let pad = ((max_y - min_y) * 0.1).max(app.units.from_mgdl(10.0));
    let bounds_y = [min_y - pad, max_y + pad];
    let bounds_x = [0.0, (points.len().saturating_sub(1)) as f64];

    let datasets = vec![Dataset::default()
        .marker(symbols::Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(Color::Cyan))
        .data(&points)];

    let chart = Chart::new(datasets)
        .block(block)
        .x_axis(Axis::default().bounds(bounds_x))
        .y_axis(
            Axis::default()
                .bounds(bounds_y)
                .labels(vec![
                    Span::raw(format!("{:.1}", bounds_y[0])),
                    Span::raw(format!("{:.1}", bounds_y[1])),
                ]),
        );
    f.render_widget(chart, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let text = match &app.last_error {
        Some(err) => Span::styled(format!(" error: {err} "), Style::default().fg(Color::Red)),
        None => Span::raw(" q quit · r refresh · u units "),
    };
    f.render_widget(Paragraph::new(Line::from(text)), area);
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
