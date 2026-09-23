/// Human-readable rendering for listings and single records. Every command also
/// takes `--json`, which prints the API's own response verbatim instead.
use serde_json::Value;

/// One JSON object per line, so `--json` output stays pipeable into `jq`.
pub fn print_json(value: &Value) {
    println!("{value}");
}

/// Stringifies a field for a table cell: strings unquoted, absent or null as
/// `-`, anything structured as compact JSON.
pub fn cell(value: &Value, key: &str) -> String {
    match value.get(key) {
        None | Some(Value::Null) => "-".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    }
}

/// Left-aligned columns padded to the widest cell, header row included. Widths
/// count characters rather than bytes so non-ASCII names stay aligned.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    for row in rows {
        for (index, field) in row.iter().enumerate() {
            let width = field.chars().count();
            if width > widths[index] {
                widths[index] = width;
            }
        }
    }
    let mut lines = vec![join_padded(
        &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
        &widths,
    )];
    lines.extend(rows.iter().map(|row| join_padded(row, &widths)));
    lines.join("\n")
}

fn join_padded(row: &[String], widths: &[usize]) -> String {
    row.iter()
        .enumerate()
        .map(|(index, field)| {
            // The last column carries no trailing padding, so copy-pasting a
            // value off the end of a line does not pick up spaces.
            if index + 1 == row.len() {
                field.clone()
            } else {
                let padding = widths[index].saturating_sub(field.chars().count());
                format!("{field}{}", " ".repeat(padding))
            }
        })
        .collect::<Vec<_>>()
        .join("  ")
}

pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    if rows.is_empty() {
        println!("(none)");
        return;
    }
    println!("{}", render_table(headers, rows));
}

/// Confirmation for a delete, which has no response body of its own to print.
/// `--json` still gets JSON so a delete can sit in a pipeline like every other
/// command.
pub fn print_deleted(kind: &str, id: &str, name: &str, json_output: bool) {
    if json_output {
        print_json(&deleted_json(kind, id, name));
        return;
    }
    println!("{}", deleted_line(kind, name));
}

fn deleted_json(kind: &str, id: &str, name: &str) -> Value {
    serde_json::json!({"deleted": kind, "id": id, "name": name})
}

/// A trashed source is recoverable, so it does not claim to be deleted.
fn deleted_line(kind: &str, name: &str) -> String {
    let verb = if kind == "source" {
        "Moved to the trash"
    } else {
        "Deleted"
    };
    format!("{verb}: {kind} \"{name}\"")
}

/// Key/value block for a single record, used by the `get` commands.
pub fn render_record(fields: &[(&str, String)]) -> String {
    let width = fields
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    fields
        .iter()
        .map(|(label, value)| {
            let padding = width.saturating_sub(label.chars().count());
            format!("{label}:{} {value}", " ".repeat(padding))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn print_record(fields: &[(&str, String)]) {
    println!("{}", render_record(fields));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cell_renders_strings_nulls_and_structures() {
        let value = json!({"name": "stripe", "color": null, "retry": {"max": 3}});
        assert_eq!(cell(&value, "name"), "stripe");
        assert_eq!(cell(&value, "color"), "-");
        assert_eq!(cell(&value, "missing"), "-");
        assert_eq!(cell(&value, "retry"), r#"{"max":3}"#);
    }

    #[test]
    fn table_pads_columns_to_the_widest_cell() {
        let rows = vec![
            vec!["stripe".to_string(), "active".to_string()],
            vec!["github-prod".to_string(), "paused".to_string()],
        ];
        let rendered = render_table(&["NAME", "STATUS"], &rows);
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines[0], "NAME         STATUS");
        assert_eq!(lines[1], "stripe       active");
        assert_eq!(lines[2], "github-prod  paused");
    }

    #[test]
    fn table_aligns_multibyte_names_by_characters() {
        let rows = vec![
            vec!["héllo".to_string(), "a".to_string()],
            vec!["x".to_string(), "b".to_string()],
        ];
        let lines: Vec<String> = render_table(&["N", "V"], &rows)
            .lines()
            .map(str::to_string)
            .collect();
        assert_eq!(lines[1], "héllo  a");
        assert_eq!(lines[2], "x      b");
    }

    #[test]
    fn deleted_confirmation_has_both_shapes() {
        assert_eq!(
            deleted_json("source", "abc", "stripe"),
            json!({"deleted": "source", "id": "abc", "name": "stripe"})
        );
        assert_eq!(
            deleted_line("source", "stripe"),
            "Moved to the trash: source \"stripe\""
        );
        assert_eq!(
            deleted_line("destination", "sink"),
            "Deleted: destination \"sink\""
        );
    }

    #[test]
    fn record_aligns_labels() {
        let rendered = render_record(&[
            ("id", "abc".to_string()),
            ("ingest url", "https://webhooker.eu/in/tok".to_string()),
        ]);
        assert_eq!(
            rendered,
            "id:         abc\ningest url: https://webhooker.eu/in/tok"
        );
    }
}
