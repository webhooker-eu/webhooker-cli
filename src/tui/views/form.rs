use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::common;
use crate::tui::app::App;
use crate::tui::forms::form::{FieldKind, Form};
use crate::tui::theme::Tone;

const LABEL_WIDTH: usize = 18;
const FORM_WIDTH: u16 = 72;
/// Pointer column plus label column.
const VALUE_COLUMN: u16 = LABEL_WIDTH as u16 + 2;
const ITEM_INDENT: &str = "    ";

pub fn render(frame: &mut Frame, area: Rect, app: &App, form: &Form) {
    let theme = &app.theme;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cursor: Option<(u16, u16)> = None;
    for (index, field) in form.fields.iter().enumerate() {
        let focused = index == form.focused;
        let marker = if focused {
            format!("{} ", theme.glyphs.pointer)
        } else {
            "  ".to_string()
        };
        let label_style = if focused {
            theme.title()
        } else {
            theme.fg(Tone::Muted)
        };
        let head = vec![
            Span::styled(marker, theme.fg(Tone::Accent)),
            Span::styled(format!("{:<LABEL_WIDTH$}", field.label), label_style),
        ];
        match &field.kind {
            FieldKind::Text(input) | FieldKind::Secret(input) => {
                if focused {
                    cursor = Some((VALUE_COLUMN + input.cursor() as u16, lines.len() as u16));
                }
                let mut spans = head;
                spans.push(Span::styled(
                    input.display(theme.glyphs.mask),
                    theme.fg(Tone::Text),
                ));
                lines.push(Line::from(spans));
            }
            FieldKind::Select { options, selected } => {
                let label = options
                    .get(*selected)
                    .map_or("-".to_string(), |option| option.label.clone());
                let mut spans = head;
                spans.push(Span::styled(
                    format!("{} {label} {}", theme.glyphs.previous, theme.glyphs.next),
                    theme.fg(Tone::Text),
                ));
                lines.push(Line::from(spans));
            }
            FieldKind::Toggle(value) => {
                let mut spans = head;
                spans.push(Span::styled(
                    format!(
                        "{} {} {}",
                        theme.glyphs.previous,
                        if *value { "on" } else { "off" },
                        theme.glyphs.next
                    ),
                    theme.fg(Tone::Text),
                ));
                lines.push(Line::from(spans));
            }
            FieldKind::Json { value, .. } => {
                let mut spans = head;
                spans.push(Span::styled(
                    crate::tui::forms::form::json_summary(value, 40),
                    theme.fg(Tone::Text),
                ));
                lines.push(Line::from(spans));
            }
            FieldKind::Checklist {
                items,
                cursor: item_cursor,
            } => {
                let mut spans = head;
                spans.push(Span::styled("space toggles", theme.fg(Tone::Muted)));
                lines.push(Line::from(spans));
                for (item_index, item) in items.iter().enumerate() {
                    let pointer = if focused && item_index == *item_cursor {
                        theme.glyphs.pointer
                    } else {
                        " "
                    };
                    let mut item_spans = vec![
                        Span::styled(format!("{ITEM_INDENT}{pointer} "), theme.fg(Tone::Accent)),
                        Span::styled(
                            format!("[{}] {}", if item.checked { "x" } else { " " }, item.label),
                            theme.fg(Tone::Text),
                        ),
                    ];
                    if let Some(note) = &item.note {
                        item_spans.push(Span::styled(format!("  {note}"), theme.fg(Tone::Warning)));
                    }
                    lines.push(Line::from(item_spans));
                }
            }
            FieldKind::Headers(editor) => {
                let mut spans = head;
                spans.push(Span::styled(
                    format!(
                        "a add {sep} x remove {sep} enter edit",
                        sep = theme.glyphs.separator
                    ),
                    theme.fg(Tone::Muted),
                ));
                lines.push(Line::from(spans));
                if editor.rows.is_empty() {
                    lines.push(Line::from(Span::styled(
                        format!("{ITEM_INDENT}  (none)"),
                        theme.fg(Tone::Muted),
                    )));
                }
                for (row_index, row) in editor.rows.iter().enumerate() {
                    let at_cursor = focused && row_index == editor.cursor;
                    let pointer = if at_cursor { theme.glyphs.pointer } else { " " };
                    if at_cursor && editor.editing {
                        cursor = Some((
                            ITEM_INDENT.len() as u16 + 2 + row.cursor() as u16,
                            lines.len() as u16,
                        ));
                    }
                    lines.push(Line::from(vec![
                        Span::styled(format!("{ITEM_INDENT}{pointer} "), theme.fg(Tone::Accent)),
                        Span::styled(row.value().to_string(), theme.fg(Tone::Text)),
                    ]));
                }
            }
        }
        if let Some(error) = &field.error {
            lines.push(Line::from(Span::styled(
                format!("{}{error}", " ".repeat(VALUE_COLUMN as usize)),
                theme.fg(Tone::Danger),
            )));
        }
    }
    if let Some(error) = &form.error {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            error.clone(),
            theme.fg(Tone::Danger),
        )));
    }

    let width = FORM_WIDTH.min(area.width.saturating_sub(2));
    let height = (lines.len() as u16 + 2).min(area.height);
    let popup = common::centered(area, width, height);
    let block = common::pane(app, &form.title, form.submitting, true);
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(Paragraph::new(lines).block(block), popup);
    if let Some((column, row)) = cursor {
        if column < inner.width && row < inner.height {
            frame.set_cursor_position((inner.x + column, inner.y + row));
        }
    }
}
