use chrono::{DateTime, Utc};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Cell, Paragraph, Row};
use ratatui::Frame;

use super::common;
use crate::tui::app::{App, Focus};
use crate::tui::events_state::StatsRange;
use crate::tui::model::StatsOverview;
use crate::tui::status;
use crate::tui::theme::Tone;

const WIDE_LAYOUT: u16 = 100;
const SUMMARY_HEIGHT: u16 = 6;
const MAX_BAR_WIDTH: u16 = 6;

pub const ASCII_BARS: symbols::bar::Set = symbols::bar::Set {
    full: "#",
    seven_eighths: "#",
    three_quarters: "#",
    five_eighths: "#",
    half: "#",
    three_eighths: "#",
    one_quarter: "#",
    one_eighth: "#",
    empty: " ",
};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    let range = app.event_screens.stats.range;
    let loading = app.data.stats_overview.loading || app.data.source_volume.loading;
    let block = common::pane(app, "Stats", loading, app.focus == Focus::Main);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [picker_area, content] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(inner);
    render_picker(frame, picker_area, app, range);
    let Some(overview) = &app.data.stats_overview.value else {
        return;
    };
    if frame.area().width >= WIDE_LAYOUT {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
                .areas(content);
        let [chart, summary] =
            Layout::vertical([Constraint::Min(6), Constraint::Length(SUMMARY_HEIGHT)]).areas(left);
        render_chart(frame, chart, app, overview);
        render_summary(frame, summary, app, overview);
        render_volume(frame, right, app);
    } else {
        let sources = app
            .data
            .source_volume
            .value
            .as_ref()
            .map_or(0, |page| page.items.len()) as u16;
        let [summary, chart, volume] = Layout::vertical([
            Constraint::Length(SUMMARY_HEIGHT),
            Constraint::Min(6),
            Constraint::Length((sources + 2).clamp(3, 8)),
        ])
        .areas(content);
        render_summary(frame, summary, app, overview);
        render_chart(frame, chart, app, overview);
        render_volume(frame, volume, app);
    }
}

fn render_picker(frame: &mut Frame, area: Rect, app: &App, current: StatsRange) {
    let theme = &app.theme;
    let mut spans = vec![Span::styled("range  ", theme.fg(Tone::Muted))];
    for (index, range) in StatsRange::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        let style = if *range == current {
            theme.title()
        } else {
            theme.fg(Tone::Muted)
        };
        spans.push(Span::styled(range.label(), style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn latency(milliseconds: Option<f64>) -> String {
    milliseconds.map_or_else(|| "-".to_string(), |value| format!("{value:.0} ms"))
}

fn render_summary(frame: &mut Frame, area: Rect, app: &App, overview: &StatsOverview) {
    let theme = &app.theme;
    let separator = theme.glyphs.separator;
    let block = common::pane(app, "Deliveries", false, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let latency_line = format!(
        "p50 {} {separator} p95 {} {separator} p99 {}",
        latency(overview.e2e_latency_ms.p50_ms),
        latency(overview.e2e_latency_ms.p95_ms),
        latency(overview.e2e_latency_ms.p99_ms),
    );
    let counts: Vec<i64> = overview
        .deliveries_by_status
        .iter()
        .map(|status| status.count)
        .collect();
    let widths = stacked_widths(&counts, usize::from(inner.width));
    let block_glyph = if theme.ascii { "#" } else { "█" };
    let bar: Vec<Span> = overview
        .deliveries_by_status
        .iter()
        .zip(widths)
        .map(|(status_count, width)| {
            let tone = status::delivery(&status_count.status, theme.glyphs, 0).tone;
            Span::styled(block_glyph.repeat(width), theme.fg(tone))
        })
        .collect();
    let mut legend = Vec::new();
    for status_count in &overview.deliveries_by_status {
        if !legend.is_empty() {
            legend.push(Span::styled(
                format!(" {separator} "),
                theme.fg(Tone::Muted),
            ));
        }
        let badge = status::delivery(&status_count.status, theme.glyphs, app.tick_count);
        legend.push(common::badge_span(app, badge));
        legend.push(Span::styled(
            format!(" {} {}", status_count.status, status_count.count),
            theme.fg(Tone::Text),
        ));
    }
    let lines = vec![
        Line::from(vec![
            Span::styled("Events ", theme.fg(Tone::Muted)),
            Span::styled(
                common::group_thousands(overview.total_events),
                theme.title(),
            ),
            Span::styled(
                format!("   Failed attempts {}", overview.failed_attempts),
                theme.fg(Tone::Muted),
            ),
        ]),
        Line::from(vec![
            Span::styled("Latency ", theme.fg(Tone::Muted)),
            Span::styled(latency_line, theme.fg(Tone::Text)),
        ]),
        Line::from(bar),
        Line::from(legend),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_chart(frame: &mut Frame, area: Rect, app: &App, overview: &StatsOverview) {
    let theme = &app.theme;
    let block = common::pane(
        app,
        &format!("Events per {}", overview.bucket_unit),
        false,
        false,
    );
    let inner_width = block.inner(area).width;
    let (bar_width, bar_gap) = bar_layout(overview.events_per_bucket.len(), inner_width);
    let bars: Vec<Bar> = overview
        .events_per_bucket
        .iter()
        .map(|bucket| {
            Bar::default()
                .value(u64::try_from(bucket.count).unwrap_or(0))
                .label(Line::from(bucket_label(
                    &bucket.bucket,
                    &overview.bucket_unit,
                )))
                .text_value(String::new())
                .style(theme.fg(Tone::Accent))
        })
        .collect();
    let chart = BarChart::default()
        .block(block)
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(bar_gap)
        .bar_set(if theme.ascii {
            ASCII_BARS
        } else {
            symbols::bar::NINE_LEVELS
        })
        .label_style(theme.fg(Tone::Muted));
    frame.render_widget(chart, area);
}

fn render_volume(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = common::pane(app, "By source", app.data.source_volume.loading, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let Some(page) = &app.data.source_volume.value else {
        return;
    };
    let mut volumes: Vec<_> = page.items.iter().collect();
    volumes.sort_by_key(|volume| std::cmp::Reverse(volume.count));
    let total: i64 = volumes.iter().map(|volume| volume.count).sum();
    let rows: Vec<Row> = volumes
        .iter()
        .map(|volume| {
            let share = if total > 0 {
                format!("{:.0}%", volume.count as f64 * 100.0 / total as f64)
            } else {
                "-".to_string()
            };
            Row::new(vec![
                Cell::from(Span::styled(volume.name.clone(), theme.fg(Tone::Text))),
                Cell::from(Span::styled(
                    common::group_thousands(volume.count),
                    theme.fg(Tone::Text),
                )),
                Cell::from(Span::styled(share, theme.fg(Tone::Muted))),
            ])
        })
        .collect();
    let widths = [
        Constraint::Fill(1),
        Constraint::Length(9),
        Constraint::Length(5),
    ];
    frame.render_widget(common::table(app, rows, &widths), inner);
}

/// Splits `width` cells in proportion to `counts`; rounding leftovers go to
/// the largest count, so the bar is always exactly `width` wide.
pub fn stacked_widths(counts: &[i64], width: usize) -> Vec<usize> {
    let total: i64 = counts.iter().sum();
    if total <= 0 {
        return vec![0; counts.len()];
    }
    let mut widths: Vec<usize> = counts
        .iter()
        .map(|count| (*count as usize * width) / total as usize)
        .collect();
    let used: usize = widths.iter().sum();
    if let Some(largest) =
        (0..counts.len()).max_by_key(|index| (counts[*index], usize::MAX - *index))
    {
        widths[largest] += width - used;
    }
    widths
}

/// (bar width, gap) so that `count` bars fit into `width` columns.
pub fn bar_layout(count: usize, width: u16) -> (u16, u16) {
    if count == 0 {
        return (1, 0);
    }
    let slot = (width / count as u16).max(1);
    match slot {
        1 => (1, 0),
        2 => (1, 1),
        _ => ((slot - 1).min(MAX_BAR_WIDTH), 1),
    }
}

pub fn bucket_label(bucket: &str, unit: &str) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(bucket) else {
        return String::new();
    };
    let pattern = match unit {
        "hour" => "%H",
        "day" => "%d",
        _ => "%m",
    };
    parsed.with_timezone(&Utc).format(pattern).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacked_widths_fill_the_bar_exactly() {
        assert_eq!(stacked_widths(&[1180, 12, 3, 9], 50), vec![50, 0, 0, 0]);
        assert_eq!(stacked_widths(&[1, 1], 5), vec![3, 2]);
        assert_eq!(stacked_widths(&[0, 0], 10), vec![0, 0]);
        assert_eq!(stacked_widths(&[600, 400], 10), vec![6, 4]);
    }

    #[test]
    fn bars_fit_the_width() {
        assert_eq!(bar_layout(6, 60), (6, 1));
        assert_eq!(bar_layout(24, 58), (1, 1));
        assert_eq!(bar_layout(30, 40), (1, 0));
        assert_eq!(bar_layout(0, 40), (1, 0));
    }

    #[test]
    fn bucket_labels_follow_the_unit() {
        assert_eq!(bucket_label("2026-09-23T06:00:00Z", "hour"), "06");
        assert_eq!(bucket_label("2026-09-23T00:00:00Z", "day"), "23");
        assert_eq!(bucket_label("2026-09-01T00:00:00Z", "month"), "09");
        assert_eq!(bucket_label("garbage", "hour"), "");
    }
}
