//! Writes tables as CSV files with the system's list separator, for spreadsheets, and as
//! tab-separated text for the clipboard.

use std::path::Path;

use anyhow::{Context as _, Result};

/// The list separator of the user's locale (Windows: Region > Additional settings > List
/// separator), usually `,` or `;` where the decimal separator is a comma. `,` when unknown.
pub fn list_separator() -> char {
    system_list_separator().unwrap_or(',')
}

#[cfg(windows)]
fn system_list_separator() -> Option<char> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Control Panel\International").ok()?;
    let value: String = key.get_value("sList").ok()?;
    let mut chars = value.chars();
    match (chars.next(), chars.next()) {
        (Some(sep), None) if !sep.is_whitespace() && sep != '"' => Some(sep),
        _ => None,
    }
}

#[cfg(not(windows))]
fn system_list_separator() -> Option<char> {
    None
}

/// `rows` as CSV text, with fields separated by `separator` and lines ending in CRLF. Fields
/// that hold the separator, a quote, a line break or edge spaces are quoted; blank fields
/// (only whitespace) are left empty.
pub fn to_csv<R, F>(rows: R, separator: char) -> String
where
    R: IntoIterator,
    R::Item: IntoIterator<Item = F>,
    F: AsRef<str>,
{
    let mut out = String::new();
    for row in rows {
        for (ix, field) in row.into_iter().enumerate() {
            if ix > 0 {
                out.push(separator);
            }
            push_field(&mut out, field.as_ref(), separator);
        }
        out.push_str("\r\n");
    }
    out
}

fn push_field(out: &mut String, field: &str, separator: char) {
    // Blank fields, like the profile's `keyboard=" "`, are written empty: `,,` or `;;`.
    if field.trim().is_empty() {
        return;
    }
    let quote = field.contains([separator, '"', '\r', '\n']) || field.trim() != field;
    if !quote {
        out.push_str(field);
        return;
    }
    out.push('"');
    out.push_str(&field.replace('"', "\"\""));
    out.push('"');
}

/// `rows` as tab-separated text that pastes into table cells, in Excel and in other tables that
/// split on tabs and line breaks. Fields are never quoted, since simple paste handlers don't
/// understand quotes: tabs and line breaks in a field become spaces, and blank fields (only
/// whitespace) are left empty. Rows are separated by `\n`, with none after the last.
pub fn to_tsv<R, F>(rows: R) -> String
where
    R: IntoIterator,
    R::Item: IntoIterator<Item = F>,
    F: AsRef<str>,
{
    let lines: Vec<String> = rows
        .into_iter()
        .map(|row| row.into_iter().map(|field| tsv_field(field.as_ref())).collect::<Vec<_>>().join("\t"))
        .collect();
    lines.join("\n")
}

fn tsv_field(field: &str) -> String {
    if field.trim().is_empty() {
        return String::new();
    }
    field.replace("\r\n", " ").replace(['\t', '\r', '\n'], " ")
}

/// Writes `csv` to `path` as UTF-8 with a byte order mark, so Excel reads it as UTF-8.
pub fn save(csv: &str, path: &Path) -> Result<()> {
    let mut bytes = Vec::with_capacity(csv.len() + 3);
    bytes.extend_from_slice("\u{feff}".as_bytes());
    bytes.extend_from_slice(csv.as_bytes());
    std::fs::write(path, bytes).with_context(|| format!("Could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_quoted_only_when_needed() {
        let rows = [
            vec!["map", "name", "keyboard"],
            vec!["spaceship", "fire", "kb1_a, kb1_b"],
            vec!["x", "say \"hi\"", ""],
        ];
        assert_eq!(
            to_csv(rows.clone(), ','),
            "map,name,keyboard\r\nspaceship,fire,\"kb1_a, kb1_b\"\r\nx,\"say \"\"hi\"\"\",\r\n"
        );
        assert_eq!(
            to_csv(rows, ';'),
            "map;name;keyboard\r\nspaceship;fire;kb1_a, kb1_b\r\nx;\"say \"\"hi\"\"\";\r\n"
        );
    }

    #[test]
    fn tsv_keeps_rows_and_columns_aligned() {
        let rows = [vec!["a", " ", "tab\there"], vec!["two\r\nlines", "say \"hi\"", "one\nmore"]];
        assert_eq!(to_tsv(rows), "a\t\ttab here\ntwo lines\tsay \"hi\"\tone more");
        assert_eq!(to_tsv([["only"]]), "only");
    }

    #[test]
    fn blank_fields_are_left_empty() {
        assert_eq!(to_csv([["a", " ", "", "  ", "b"]], ';'), "a;;;;b\r\n");
        assert_eq!(to_csv([[" x"]], ','), "\" x\"\r\n");
    }
}
