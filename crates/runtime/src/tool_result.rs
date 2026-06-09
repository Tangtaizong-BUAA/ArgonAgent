//! Tool result artifact helpers.
//!
//! Tool calls have a small completion event, but large or reviewable outputs
//! need a stable artifact. This module creates a common JSON shape for file,
//! search, patch, command, and research tool outputs.

use crate::artifact::{ArtifactKind, ArtifactRecord, ArtifactStore};
use std::io;

/// Maximum size for `detail_json` in bytes (64 KB). Content beyond this limit
/// is truncated and a truncation marker is appended.
const MAX_DETAIL_JSON_LEN: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolResultRecord {
    pub tool_call_id: String,
    pub tool_id: String,
    pub ok: bool,
    pub preview: String,
    pub detail_json: String,
    pub privacy_class: String,
}

impl ToolResultRecord {
    pub fn new(
        tool_call_id: impl Into<String>,
        tool_id: impl Into<String>,
        ok: bool,
        preview: impl Into<String>,
        detail_json: impl Into<String>,
    ) -> Self {
        let detail: String = detail_json.into();
        let (detail, _was_truncated) = truncate_detail_json(detail, MAX_DETAIL_JSON_LEN);
        Self {
            tool_call_id: tool_call_id.into(),
            tool_id: tool_id.into(),
            ok,
            preview: truncate_preview(&preview.into(), 2_000),
            detail_json: detail,
            privacy_class: "internal".to_string(),
        }
    }
}

pub fn write_tool_result_artifact(
    store: &ArtifactStore,
    artifact_id: impl Into<String>,
    result: &ToolResultRecord,
) -> Result<ArtifactRecord, io::Error> {
    store.put_bytes_auto_hash(
        artifact_id,
        ArtifactKind::ToolResult,
        &result.privacy_class,
        tool_result_json(result).as_bytes(),
    )
}

pub fn tool_result_json(result: &ToolResultRecord) -> String {
    format!(
        "{{\"schema_version\":\"tool_result.v0\",\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"ok\":{},\"preview\":\"{}\",\"detail\":{},\"privacy_class\":\"{}\"}}",
        escape(&result.tool_call_id),
        escape(&result.tool_id),
        result.ok,
        escape(&result.preview),
        result.detail_json, // detail_json is now properly escaped in ToolResultRecord::new
        escape(&result.privacy_class)
    )
}

/// Truncate a detail_json string to at most `max_bytes` bytes and escape it
/// for safe JSON embedding. Returns the escaped detail string and whether it
/// was truncated.
fn truncate_detail_json(mut raw: String, max_bytes: usize) -> (String, bool) {
    // First ensure the raw string is not too large.
    if raw.len() > max_bytes {
        // Truncate to max_bytes, then attempt to truncate at a valid UTF-8 boundary.
        let mut truncate_at = max_bytes;
        while truncate_at > 0 && !raw.is_char_boundary(truncate_at) {
            truncate_at -= 1;
        }
        if truncate_at == 0 {
            raw = String::new();
        } else {
            raw.truncate(truncate_at);
        }
        // Append truncation marker. Escape it to avoid injection.
        raw.push_str("\n[detail_truncated]");
        return (escape(&raw), true);
    }
    (escape(&raw), false)
}

pub fn json_string(value: &str) -> String {
    format!("\"{}\"", escape(value))
}

pub fn truncate_preview(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("\n[truncated]");
    output
}

fn escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            other if other.is_control() => format!("\\u{:04x}", other as u32).chars().collect(),
            other => vec![other],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn writes_tool_result_artifact_with_preview_and_detail() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-tool-result-{nonce}"));
        let store = ArtifactStore::new(&root);
        let result = ToolResultRecord::new(
            "tool_1",
            "file.read",
            true,
            "src/lib.rs",
            "{\"path\":\"src/lib.rs\",\"bytes\":12}",
        );
        let artifact = write_tool_result_artifact(&store, "artifact_tool_1", &result).unwrap();
        assert_eq!(artifact.kind, ArtifactKind::ToolResult);
        let content = String::from_utf8(store.read_bytes(&artifact).unwrap()).unwrap();
        assert!(content.contains("\"schema_version\":\"tool_result.v0\""));
        assert!(content.contains("\"tool_id\":\"file.read\""));
        // Verify detail_json is escaped — double-quotes in the raw detail should
        // become escaped quotes in the output.
        assert!(content.contains("\\\"path\\\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn truncates_long_preview() {
        let preview = truncate_preview("abcdef", 3);
        assert_eq!(preview, "abc\n[truncated]");
    }

    #[test]
    fn escapes_json_injection_in_detail() {
        let record = ToolResultRecord::new(
            "id1",
            "tool",
            true,
            "ok",
            r#"{"key": "val\"}, \"injected\": true, \"x\": \""#,
        );
        let json = tool_result_json(&record);
        // The detail should not break out of the JSON string.
        // It should contain \", not raw " inside the detail value.
        assert!(!json.contains(r#""detail":{"key""#));
        assert!(json.contains("\\\""));
    }

    #[test]
    fn limits_large_detail_json() {
        let big = "x".repeat(100_000);
        let record = ToolResultRecord::new("id1", "tool", true, "ok", &big);
        // After escaping, the length should be less than max + truncation marker.
        assert!(record.detail_json.len() < MAX_DETAIL_JSON_LEN + 100);
        assert!(record.detail_json.contains("[detail_truncated]"));
    }
}
