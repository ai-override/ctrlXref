//! Writes structured exports as JSON files.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde_json::Value;

/// Writes `value` to `path` as indented UTF-8 JSON, without a byte order mark since JSON
/// must not start with one.
pub fn save(value: &Value, path: &Path) -> Result<()> {
    let mut text = serde_json::to_string_pretty(value).context("Could not serialize JSON")?;
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("Could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn saved_file_round_trips_without_a_byte_order_mark() {
        let path = std::env::temp_dir().join("ctrlxref_json_save_test.json");
        let value = json!({ "actionmaps": [{ "name": "m\"1", "UILabel": "Käsi" }] });
        save(&value, &path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(bytes[0], b'{');
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap(), value);
    }
}
