//! Event bodies: JSON pretty-printed with syntax colors, other text as is,
//! non-UTF-8 bytes as a hex dump.

use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::tui::app::App;
use crate::tui::model::EventDetail;
use crate::tui::theme::Tone;

/// Larger binary bodies show their first 16 KB.
pub const HEX_DUMP_LIMIT: usize = 16 * 1_024;

pub fn body_lines(app: &App, event: &EventDetail) -> Vec<Line<'static>> {
    let theme = &app.theme;
    if let Some(bytes) = event.binary_body() {
        let shown = bytes.len().min(HEX_DUMP_LIMIT);
        let mut note = format!("Binary body, {} bytes, shown as hex", bytes.len());
        if shown < bytes.len() {
            note.push_str(&format!(" (first {shown})"));
        }
        let mut lines = vec![Line::from(Span::styled(note, theme.fg(Tone::Muted)))];
        lines.extend(
            hex_dump(&bytes[..shown])
                .into_iter()
                .map(|line| Line::from(Span::styled(line, theme.fg(Tone::Text)))),
        );
        return lines;
    }
    if event.body.is_empty() {
        return vec![Line::from(Span::styled(
            "(empty body)",
            theme.fg(Tone::Muted),
        ))];
    }
    match serde_json::from_str::<Value>(&event.body) {
        Ok(value) if value.is_object() || value.is_array() => json_lines(app, &value),
        _ => event
            .body
            .lines()
            .map(|line| Line::from(Span::styled(line.to_string(), theme.fg(Tone::Text))))
            .collect(),
    }
}

pub fn json_lines(app: &App, value: &Value) -> Vec<Line<'static>> {
    serde_json::to_string_pretty(value)
        .unwrap_or_default()
        .lines()
        .map(|line| highlight_line(app, line))
        .collect()
}

/// Colors one line of pretty-printed JSON: keys, strings, numbers, literals
/// and punctuation. Pretty-printed strings never span lines.
pub fn highlight_line(app: &App, line: &str) -> Line<'static> {
    let theme = &app.theme;
    let characters: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < characters.len() {
        let start = index;
        let character = characters[index];
        let tone = if character == '"' {
            index += 1;
            while index < characters.len() {
                match characters[index] {
                    '\\' => index += 2,
                    '"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            index = index.min(characters.len());
            let followed_by_colon = characters[index..]
                .iter()
                .find(|next| !next.is_whitespace())
                == Some(&':');
            if followed_by_colon {
                Tone::Accent
            } else {
                Tone::Success
            }
        } else if character == '-' || character.is_ascii_digit() {
            while index < characters.len()
                && (characters[index].is_ascii_digit()
                    || matches!(characters[index], '-' | '+' | '.' | 'e' | 'E'))
            {
                index += 1;
            }
            Tone::Warning
        } else if character.is_alphabetic() {
            while index < characters.len() && characters[index].is_alphabetic() {
                index += 1;
            }
            Tone::Danger
        } else {
            while index < characters.len()
                && !matches!(characters[index], '"' | '-')
                && !characters[index].is_ascii_digit()
                && !characters[index].is_alphabetic()
            {
                index += 1;
            }
            Tone::Muted
        };
        let token: String = characters[start..index].iter().collect();
        spans.push(Span::styled(token, theme.fg(tone)));
    }
    Line::from(spans)
}

/// `00000000  ff fe 00 01 …  |....|`, 16 bytes per line.
pub fn hex_dump(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(index, chunk)| {
            let hex: Vec<String> = chunk.iter().map(|byte| format!("{byte:02x}")).collect();
            let printable: String = chunk
                .iter()
                .map(|byte| {
                    if byte.is_ascii_graphic() || *byte == b' ' {
                        *byte as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:08x}  {:<47}  |{printable}|", index * 16, hex.join(" "))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::fixtures;

    #[test]
    fn json_lines_color_keys_strings_numbers_and_literals() {
        let app = fixtures::app();
        let line = highlight_line(&app, "  \"type\": \"invoice.paid\",");
        let parts: Vec<(&str, Tone)> = vec![
            ("  ", Tone::Muted),
            ("\"type\"", Tone::Accent),
            (": ", Tone::Muted),
            ("\"invoice.paid\"", Tone::Success),
            (",", Tone::Muted),
        ];
        assert_eq!(line.spans.len(), parts.len());
        for (span, (text, tone)) in line.spans.iter().zip(parts) {
            assert_eq!(span.content, text);
            assert_eq!(span.style, app.theme.fg(tone));
        }
        let literals = highlight_line(&app, "  \"paid\": true, \"amount\": -12.5e3");
        let tones: Vec<_> = literals
            .spans
            .iter()
            .map(|span| span.content.to_string())
            .collect();
        assert!(tones.contains(&"true".to_string()));
        assert!(tones.contains(&"-12.5e3".to_string()));
    }

    #[test]
    fn escaped_quotes_stay_inside_their_string() {
        let app = fixtures::app();
        let line = highlight_line(&app, r#"  "note": "say \"hi\"""#);
        assert_eq!(line.spans[3].content, r#""say \"hi\"""#);
    }

    #[test]
    fn hex_dumps_show_offsets_bytes_and_printables() {
        assert_eq!(
            hex_dump(&[0xFF, 0xFE, 0x00, 0x01, b'A']),
            vec![format!("00000000  {:<47}  |....A|", "ff fe 00 01 41")]
        );
        assert_eq!(hex_dump(&[b'x'; 17]).len(), 2);
        assert!(hex_dump(&[b'x'; 17])[1].starts_with("00000010  78"));
    }

    #[test]
    fn bodies_pick_json_text_or_hex() {
        let app = fixtures::app();
        let mut event = fixtures::event_detail();
        assert!(body_lines(&app, &event)[0].to_string().starts_with('{'));
        event.body = "plain text".into();
        assert_eq!(body_lines(&app, &event)[0].to_string(), "plain text");
        event.body = String::new();
        assert_eq!(body_lines(&app, &event)[0].to_string(), "(empty body)");
        event.body_base64 = Some("//4AAQ==".into());
        let binary = body_lines(&app, &event);
        assert_eq!(binary[0].to_string(), "Binary body, 4 bytes, shown as hex");
        assert!(binary[1].to_string().starts_with("00000000  ff fe 00 01"));
    }
}
