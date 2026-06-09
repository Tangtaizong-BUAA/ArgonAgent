pub const OPTIONAL_NULL_REPAIR_KEYS: &[&str] = &[
    "include_hidden",
    "root",
    "max_bytes",
    "max_results",
    "max_files",
    "max_depth",
    "offset",
    "limit",
    "replace_all",
    "job_id",
    "output_dir",
];

pub fn is_never_repair_field(tool_id: &str, issue_path: &str) -> bool {
    matches!(
        (tool_id, issue_path),
        ("shell.command", "command")
            | ("file.write", "content")
            | ("file.write", "path")
            | ("file.write", "base_hash")
            | ("file.edit", "old_string")
            | ("file.edit", "new_string")
            | ("file.edit", "path")
            | ("file.edit", "base_hash")
            | ("file.multi_edit", "edits")
            | ("file.multi_edit", "path")
            | ("file.multi_edit", "base_hash")
            | ("patch.apply", "old_string")
            | ("patch.apply", "new_string")
            | ("patch.apply", "diff")
            | ("patch.apply", "path")
            | ("patch.apply", "base_hash")
    )
}

pub fn can_repair_field(tool_id: &str, issue_path: &str) -> bool {
    !is_never_repair_field(tool_id, issue_path)
}

pub fn markdown_link_path_target(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if !value.starts_with('[') {
        return None;
    }
    let close = value.find("](")?;
    if !value.ends_with(')') {
        return None;
    }
    let label = &value[1..close];
    let target = &value[close + 2..value.len() - 1];
    if target.starts_with("http://") || target.starts_with("https://") {
        return Some(label.to_string());
    }
    Some(target.to_string())
}

pub fn optional_null_present(input: &str, key: &str) -> bool {
    let marker = format!("\"{key}\"");
    let Some(start) = input.find(&marker) else {
        return false;
    };
    let after_key = input[start + marker.len()..].trim_start();
    let Some(after_colon) = after_key.strip_prefix(':') else {
        return false;
    };
    after_colon.trim_start().starts_with("null")
}

pub fn quoted_usize_argument(input: &str, key: &str) -> Option<usize> {
    let raw = crate::tcml::parser::extract_json_string(input, key)?;
    raw.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_repairs_side_effect_payload_fields() {
        assert!(is_never_repair_field("file.write", "content"));
        assert!(is_never_repair_field("file.write", "path"));
        assert!(is_never_repair_field("shell.command", "command"));
        assert!(is_never_repair_field("patch.apply", "base_hash"));
        assert!(!is_never_repair_field("file.read", "limit"));
    }

    #[test]
    fn extracts_markdown_path_targets_without_touching_plain_values() {
        assert_eq!(
            markdown_link_path_target(Some("[README](README.md)")).as_deref(),
            Some("README.md")
        );
        assert_eq!(
            markdown_link_path_target(Some("[docs](https://example.test/docs)")).as_deref(),
            Some("docs")
        );
        assert_eq!(markdown_link_path_target(Some("README.md")), None);
    }

    #[test]
    fn detects_optional_null_and_quoted_usize_arguments() {
        let input = r#"{"path":"README.md","limit":"2000","offset":null}"#;
        assert!(optional_null_present(input, "offset"));
        assert_eq!(quoted_usize_argument(input, "limit"), Some(2000));
        assert_eq!(quoted_usize_argument(input, "path"), None);
    }
}
