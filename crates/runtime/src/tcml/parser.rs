//! Structured tool-call parser boundary for native DeepSeek/Qwen outputs.
//!
//! DEPRECATED-COMPAT: Phase 2 TCML callers should import parser helpers from
//! `crate::tcml`. This file still hosts the parser implementation until the
//! migration can delete the legacy module without changing behavior.
//!
//! The policy parser decides whether a call may execute. This module only
//! extracts the first structured call and its arguments so execution does not
//! depend on ad hoc raw-output string searches.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallSyntax {
    NativeJson,
    DeepSeekXml,
    DeepSeekDsml,
    RepairedJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallParseStatus {
    Parsed,
    Repaired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolCall {
    pub provider_tool_call_id: Option<String>,
    pub tool_id: String,
    pub arguments_json: String,
    pub syntax: ToolCallSyntax,
    pub status: ToolCallParseStatus,
    pub repair_applied: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedToolArguments {
    pub path: Option<String>,
    pub root: Option<String>,
    pub include_hidden: Option<bool>,
    pub command: Option<String>,
    pub pattern: Option<String>,
    pub query: Option<String>,
    pub content: Option<String>,
    pub old_string: Option<String>,
    pub new_string: Option<String>,
    pub base_hash: Option<String>,
    pub replace_all: Option<bool>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub max_bytes: Option<usize>,
    pub max_results: Option<usize>,
    pub max_files: Option<usize>,
    pub max_depth: Option<usize>,
    pub edits_json: Option<String>,
    pub job_id: Option<String>,
    pub input_csv: Option<String>,
    pub answer: Option<String>,
    pub model_role: Option<String>,
    pub write_scope_json: Option<String>,
}

pub fn parse_tool_calls(raw: &str) -> Vec<ParsedToolCall> {
    let dsml_calls = expand_aggregate_tool_calls(parse_deepseek_dsml_tool_calls(raw));
    if !dsml_calls.is_empty() {
        return dsml_calls;
    }
    let function_calls = expand_aggregate_tool_calls(parse_deepseek_function_tool_calls(raw));
    if !function_calls.is_empty() {
        return function_calls;
    }
    let json_calls = expand_aggregate_tool_calls(parse_json_tool_calls(raw));
    if !json_calls.is_empty() {
        return json_calls;
    }
    expand_aggregate_tool_calls(parse_first_tool_call(raw).into_iter().collect())
}

pub fn parse_first_tool_call(raw: &str) -> Option<ParsedToolCall> {
    parse_deepseek_dsml_tool_calls(raw)
        .into_iter()
        .next()
        .or_else(|| parse_deepseek_function_tool_calls(raw).into_iter().next())
        .or_else(|| parse_deepseek_xml_tool_call(raw))
        .or_else(|| parse_json_tool_call(raw))
}

fn parse_deepseek_dsml_tool_calls(raw: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let start_marker = "<｜｜DSML｜｜invoke";
    let end_marker = "</｜｜DSML｜｜invoke>";
    let parameter_marker = "<｜｜DSML｜｜parameter";
    let parameter_end_marker = "</｜｜DSML｜｜parameter>";
    let mut cursor = 0usize;
    while let Some(relative_start) = raw[cursor..].find(start_marker) {
        let start = cursor + relative_start;
        let Some(relative_tag_end) = raw[start..].find('>') else {
            break;
        };
        let tag_end = start + relative_tag_end;
        let tag = &raw[start + start_marker.len()..tag_end];
        let attrs = parse_xml_like_attributes(tag);
        let Some(tool_id) = attrs
            .iter()
            .find(|(key, _)| key == "name" || key == "id")
            .map(|(_, value)| value.clone())
        else {
            cursor = tag_end + 1;
            continue;
        };
        let body_start = tag_end + 1;
        let closed = raw[body_start..].find(end_marker);
        let body_end = closed
            .map(|relative_end| body_start + relative_end)
            .unwrap_or(raw.len());
        let body = &raw[body_start..body_end];
        let mut parameters = Vec::new();
        let mut parameter_cursor = 0usize;
        while let Some(relative_parameter_start) = body[parameter_cursor..].find(parameter_marker) {
            let parameter_start = parameter_cursor + relative_parameter_start;
            let Some(relative_parameter_tag_end) = body[parameter_start..].find('>') else {
                break;
            };
            let parameter_tag_end = parameter_start + relative_parameter_tag_end;
            let parameter_tag = &body[parameter_start + parameter_marker.len()..parameter_tag_end];
            let parameter_attrs = parse_xml_like_attributes(parameter_tag);
            let Some(parameter_name) = parameter_attrs
                .iter()
                .find(|(key, _)| key == "name")
                .map(|(_, value)| value.clone())
            else {
                parameter_cursor = parameter_tag_end + 1;
                continue;
            };
            let value_start = parameter_tag_end + 1;
            let Some(relative_value_end) = body[value_start..].find(parameter_end_marker) else {
                break;
            };
            let value_end = value_start + relative_value_end;
            let raw_json = parameter_attrs
                .iter()
                .any(|(key, value)| key == "string" && value == "false");
            parameters.push((
                parameter_name,
                body[value_start..value_end].trim().to_string(),
                raw_json,
            ));
            parameter_cursor = value_end + parameter_end_marker.len();
        }
        calls.push(ParsedToolCall {
            provider_tool_call_id: attrs
                .iter()
                .find(|(key, _)| key == "tool_call_id" || key == "tool_use_id")
                .map(|(_, value)| value.clone()),
            tool_id,
            arguments_json: dsml_parameters_to_arguments_json(&parameters),
            syntax: ToolCallSyntax::DeepSeekDsml,
            status: ToolCallParseStatus::Parsed,
            repair_applied: false,
        });
        cursor = if closed.is_some() {
            body_end + end_marker.len()
        } else {
            raw.len()
        };
    }
    calls
}

pub fn strip_tool_call_markup_from_visible_text(raw: &str) -> String {
    let mut output = raw.to_string();
    output = strip_between_markers(
        &output,
        "<｜｜DSML｜｜tool_calls>",
        "</｜｜DSML｜｜tool_calls>",
    );
    output = strip_between_markers(&output, "<tool_call>", "</tool_call>");
    output = strip_function_tags(&output);
    strip_json_tool_call_lines(&output)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn strip_between_markers(input: &str, start_marker: &str, end_marker: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = input[cursor..].find(start_marker) {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let after_start = start + start_marker.len();
        if let Some(relative_end) = input[after_start..].find(end_marker) {
            cursor = after_start + relative_end + end_marker.len();
        } else {
            cursor = input.len();
            break;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn strip_function_tags(input: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = input[cursor..].find("<function") {
        let start = cursor + relative_start;
        output.push_str(&input[cursor..start]);
        let Some(relative_tag_end) = input[start..].find('>') else {
            cursor = input.len();
            break;
        };
        let tag_end = start + relative_tag_end + 1;
        if let Some(relative_end) = input[tag_end..].find("</function>") {
            cursor = tag_end + relative_end + "</function>".len();
        } else {
            cursor = tag_end;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn strip_json_tool_call_lines(input: &str) -> String {
    input
        .lines()
        .filter(|line| !line.trim().starts_with("{\"tool_calls\""))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn visible_text_without_tool_calls(raw: &str) -> String {
    strip_tool_call_markup_from_visible_text(raw)
}

pub fn parse_tool_arguments(arguments_json: &str) -> ParsedToolArguments {
    ParsedToolArguments {
        path: extract_json_string_any(
            arguments_json,
            &[
                "path",
                "filePath",
                "file_path",
                "filepath",
                "file",
                "target",
                "fileName",
                "filename",
                "relative_path",
            ],
        ),
        root: extract_json_string_any(
            arguments_json,
            &[
                "root",
                "cwd",
                "working_dir",
                "workingDirectory",
                "directory",
                "dir",
            ],
        ),
        include_hidden: extract_json_bool_any(arguments_json, &["include_hidden", "includeHidden"]),
        command: extract_json_string_any(
            arguments_json,
            &["command", "cmd", "shell_command", "exec", "script"],
        ),
        pattern: extract_json_string_any(arguments_json, &["pattern", "regex", "search_pattern"]),
        query: extract_json_string_any(arguments_json, &["query", "search", "text"]),
        content: extract_json_string_any(
            arguments_json,
            &[
                "content", "question", "plan", "goal", "items", "todos", "tasks", "text", "prompt",
            ],
        )
        .or_else(|| extract_json_value_any(arguments_json, &["items", "todos", "tasks"])),
        old_string: extract_json_string_any(arguments_json, &["old_string", "oldString", "before"]),
        new_string: extract_json_string_any(arguments_json, &["new_string", "newString", "after"]),
        base_hash: extract_json_string_any(
            arguments_json,
            &["base_hash", "baseHash", "file_hash", "fileHash"],
        ),
        replace_all: extract_json_bool_any(arguments_json, &["replace_all", "replaceAll"]),
        offset: extract_json_usize_any(arguments_json, &["offset", "start", "line_start"]),
        limit: extract_json_usize_any(arguments_json, &["limit", "max_lines", "maxLines"]),
        max_bytes: extract_json_usize_any(arguments_json, &["max_bytes", "maxBytes"]),
        max_results: extract_json_usize_any(
            arguments_json,
            &["max_results", "maxResults", "max_entries", "maxEntries"],
        ),
        max_files: extract_json_usize_any(arguments_json, &["max_files", "maxFiles"]),
        max_depth: extract_json_usize_any(arguments_json, &["max_depth", "maxDepth", "depth"]),
        edits_json: extract_json_value_any(arguments_json, &["edits", "changes"]),
        job_id: extract_json_string_any(arguments_json, &["job_id", "jobId"]),
        input_csv: extract_json_string_any(arguments_json, &["input_csv", "inputCsv", "csv_path"]),
        answer: extract_json_string_any(arguments_json, &["answer", "message", "response"]),
        model_role: extract_json_string_any(arguments_json, &["model_role", "modelRole"]),
        write_scope_json: extract_json_value_any(arguments_json, &["write_scope", "writeScope"]),
    }
}

pub fn normalize_tool_id(tool_id: &str) -> String {
    crate::tcml::canonical_tool_id(tool_id)
}

fn extract_json_string_any(input: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| extract_json_string(input, key))
}

fn extract_json_bool_any(input: &str, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| extract_json_bool(input, key))
}

fn extract_json_usize_any(input: &str, keys: &[&str]) -> Option<usize> {
    keys.iter().find_map(|key| extract_json_usize(input, key))
}

fn extract_json_value_any(input: &str, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| extract_json_value(input, key))
}

fn extract_json_string_array_any(input: &str, keys: &[&str]) -> Option<Vec<String>> {
    keys.iter()
        .find_map(|key| extract_json_string_array(input, key))
}

pub fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    let rest = after_colon.strip_prefix('"')?;
    decode_json_string_prefix(rest)
}

pub fn extract_json_bool(input: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    if after_colon.starts_with("true") {
        Some(true)
    } else if after_colon.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

pub fn extract_json_usize(input: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    if let Some(rest) = after_colon.strip_prefix('"') {
        let value = decode_json_string_prefix(rest)?;
        return value.trim().parse().ok();
    }
    let digits = after_colon
        .chars()
        .take_while(|char| char.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

pub fn extract_json_value(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    if after_colon.starts_with('{') {
        return extract_balanced_object(after_colon);
    }
    if after_colon.starts_with('[') {
        return extract_balanced_array(after_colon);
    }
    if after_colon.starts_with('"') {
        return extract_json_string(input, key);
    }
    None
}

fn extract_json_string_array(input: &str, key: &str) -> Option<Vec<String>> {
    let value = extract_json_value(input, key).or_else(|| extract_json_string(input, key))?;
    let trimmed = value.trim();
    if !trimmed.starts_with('[') {
        return None;
    }
    parse_json_string_array(trimmed)
}

fn parse_deepseek_xml_tool_call(raw: &str) -> Option<ParsedToolCall> {
    let tool_id = extract_xml_tag(raw, "name")?;
    let arguments = extract_xml_tag(raw, "arguments").unwrap_or_else(|| "{}".to_string());
    let (arguments_json, repaired) = repair_json_argument_string(&arguments);
    Some(ParsedToolCall {
        provider_tool_call_id: extract_xml_tag(raw, "id")
            .or_else(|| extract_xml_tag(raw, "tool_call_id"))
            .or_else(|| extract_xml_tag(raw, "tool_use_id")),
        tool_id,
        arguments_json,
        syntax: if repaired {
            ToolCallSyntax::RepairedJson
        } else {
            ToolCallSyntax::DeepSeekXml
        },
        status: if repaired {
            ToolCallParseStatus::Repaired
        } else {
            ToolCallParseStatus::Parsed
        },
        repair_applied: repaired,
    })
}

fn parse_deepseek_function_tool_calls(raw: &str) -> Vec<ParsedToolCall> {
    let mut calls = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative_start) = raw[cursor..].find("<function") {
        let start = cursor + relative_start;
        let Some(relative_end) = raw[start..].find('>') else {
            break;
        };
        let tag_end = start + relative_end;
        let tag = &raw[start + "<function".len()..tag_end];
        let attrs = parse_xml_like_attributes(tag);
        let tool_id = attrs
            .iter()
            .find(|(key, _)| key == "id" || key == "name")
            .map(|(_, value)| value.clone());
        if let Some(tool_id) = tool_id {
            let arguments_json = attrs_to_arguments_json(&attrs);
            calls.push(ParsedToolCall {
                provider_tool_call_id: attrs
                    .iter()
                    .find(|(key, _)| key == "tool_call_id" || key == "tool_use_id")
                    .map(|(_, value)| value.clone()),
                tool_id,
                arguments_json,
                syntax: ToolCallSyntax::DeepSeekXml,
                status: ToolCallParseStatus::Parsed,
                repair_applied: false,
            });
        }
        cursor = tag_end + 1;
    }
    calls
}

fn parse_json_tool_call(raw: &str) -> Option<ParsedToolCall> {
    let tool_id = extract_json_string(raw, "name")?;
    let (arguments_json, repaired) = extract_json_arguments(raw).unwrap_or_else(|| {
        let repaired = false;
        ("{}".to_string(), repaired)
    });
    Some(ParsedToolCall {
        provider_tool_call_id: extract_json_string(raw, "id")
            .or_else(|| extract_json_string(raw, "tool_call_id"))
            .or_else(|| extract_json_string(raw, "tool_use_id")),
        tool_id,
        arguments_json,
        syntax: if repaired {
            ToolCallSyntax::RepairedJson
        } else {
            ToolCallSyntax::NativeJson
        },
        status: if repaired {
            ToolCallParseStatus::Repaired
        } else {
            ToolCallParseStatus::Parsed
        },
        repair_applied: repaired,
    })
}

fn parse_json_tool_calls(raw: &str) -> Vec<ParsedToolCall> {
    let Some(array) = extract_json_array_for_key(raw, "tool_calls") else {
        return Vec::new();
    };
    split_top_level_json_objects(&array)
        .into_iter()
        .filter_map(|object| parse_json_tool_call(&object))
        .collect()
}

fn expand_aggregate_tool_calls(calls: Vec<ParsedToolCall>) -> Vec<ParsedToolCall> {
    let mut expanded = Vec::new();
    for call in calls {
        if is_multi_file_read_alias(&call.tool_id) {
            if let Some(paths) = extract_json_string_array_any(
                &call.arguments_json,
                &["files", "paths", "filePaths"],
            ) {
                for path in paths {
                    expanded.push(ParsedToolCall {
                        provider_tool_call_id: None,
                        tool_id: "file.read".to_string(),
                        arguments_json: format!("{{\"path\":\"{}\"}}", json_escape(&path)),
                        syntax: call.syntax.clone(),
                        status: call.status.clone(),
                        repair_applied: call.repair_applied,
                    });
                }
                continue;
            }
        }
        expanded.push(call);
    }
    expanded
}

fn is_multi_file_read_alias(tool_id: &str) -> bool {
    let normalized = tool_id
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() {
                char.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "read_files"
            | "file_reads"
            | "files_read"
            | "read_many_files"
            | "multi_read"
            | "file_read_many"
            | "read_file_many"
            | "file_read_batch"
            | "batch_file_read"
    )
}

fn parse_xml_like_attributes(input: &str) -> Vec<(String, String)> {
    let mut attrs = Vec::new();
    let bytes = input.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        let key_start = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'-' | b'.'))
        {
            index += 1;
        }
        if key_start == index {
            index += 1;
            continue;
        }
        let key = input[key_start..index].to_string();
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'=' {
            continue;
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() || !matches!(bytes[index], b'"' | b'\'') {
            continue;
        }
        let quote = bytes[index];
        index += 1;
        let value_start = index;
        while index < bytes.len() && bytes[index] != quote {
            index += 1;
        }
        if index > value_start {
            attrs.push((key, input[value_start..index].to_string()));
        }
        index = index.saturating_add(1);
    }
    attrs
}

fn attrs_to_arguments_json(attrs: &[(String, String)]) -> String {
    let mut parts = Vec::new();
    for (key, value) in attrs {
        if matches!(key.as_str(), "id" | "name") {
            continue;
        }
        parts.push(format!(
            "\"{}\":\"{}\"",
            json_escape(key),
            json_escape(value)
        ));
    }
    format!("{{{}}}", parts.join(","))
}

fn dsml_parameters_to_arguments_json(parameters: &[(String, String, bool)]) -> String {
    let mut parts = Vec::new();
    for (key, value, raw_json) in parameters {
        if *raw_json && is_safe_json_argument_value(value) {
            parts.push(format!("\"{}\":{}", json_escape(key), value.trim()));
        } else {
            parts.push(format!(
                "\"{}\":\"{}\"",
                json_escape(key),
                json_escape(value)
            ));
        }
    }
    format!("{{{}}}", parts.join(","))
}

fn is_safe_json_argument_value(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || trimmed == "true"
        || trimmed == "false"
        || trimmed == "null"
        || trimmed
            .chars()
            .next()
            .map(|char| char.is_ascii_digit() || char == '-')
            .unwrap_or(false)
}

fn json_escape(value: &str) -> String {
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

fn extract_json_arguments(raw: &str) -> Option<(String, bool)> {
    let marker = "\"arguments\"";
    let key_start = raw.find(marker)? + marker.len();
    let after_key = raw[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    if let Some(after_quote) = after_colon.strip_prefix('"') {
        let decoded = decode_json_string_prefix(after_quote)?;
        return Some(repair_json_argument_string(&decoded));
    }
    if after_colon.starts_with('{') {
        let object = extract_balanced_object(after_colon)?;
        return Some(repair_json_argument_string(&object));
    }
    None
}

fn extract_balanced_object(input: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut end_index = None;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    end_index = Some(index + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }
    end_index.map(|end| input[..end].to_string())
}

fn extract_balanced_array(input: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut end_index = None;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '[' => depth += 1,
            ']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    end_index = Some(index + ch.len_utf8());
                    break;
                }
            }
            _ => {}
        }
    }
    end_index.map(|end| input[..end].to_string())
}

fn extract_json_array_for_key(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\"");
    let key_start = input.find(&marker)? + marker.len();
    let after_key = input[key_start..].trim_start();
    let after_colon = after_key.strip_prefix(':')?.trim_start();
    extract_balanced_array(after_colon)
}

fn split_top_level_json_objects(array_json: &str) -> Vec<String> {
    let trimmed = array_json.trim();
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_string {
            if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = start.take() {
                        objects.push(inner[start..index + ch.len_utf8()].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

fn parse_json_string_array(array_json: &str) -> Option<Vec<String>> {
    let trimmed = array_json.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    let mut values = Vec::new();
    let mut cursor = 0usize;
    while cursor < inner.len() {
        let tail = inner[cursor..].trim_start();
        cursor = inner.len() - tail.len();
        if tail.is_empty() {
            break;
        }
        let rest = tail.strip_prefix('"')?;
        let value = decode_json_string_prefix(rest)?;
        let consumed = json_string_literal_len(tail)?;
        values.push(value);
        cursor += consumed;
        let after = inner[cursor..].trim_start();
        cursor = inner.len() - after.len();
        if after.starts_with(',') {
            cursor += 1;
        } else if after.is_empty() {
            break;
        } else {
            return None;
        }
    }
    Some(values)
}

fn json_string_literal_len(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    if bytes.first().copied() != Some(b'"') {
        return None;
    }
    let mut index = 1usize;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            index += 1;
            continue;
        }
        if byte == b'"' {
            return Some(index + 1);
        }
        index += 1;
    }
    None
}

fn decode_json_string_prefix(input_after_open_quote: &str) -> Option<String> {
    let mut output = String::new();
    let mut escaped = false;
    let mut chars = input_after_open_quote.chars();
    while let Some(ch) = chars.next() {
        if escaped {
            match ch {
                'n' => output.push('\n'),
                'r' => output.push('\r'),
                't' => output.push('\t'),
                '"' => output.push('"'),
                '\\' => output.push('\\'),
                '/' => output.push('/'),
                'u' => {
                    let mut hex = String::with_capacity(4);
                    for _ in 0..4 {
                        hex.push(chars.next()?);
                    }
                    let value = u32::from_str_radix(&hex, 16).ok()?;
                    output.push(char::from_u32(value)?);
                }
                other => output.push(other),
            }
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(output);
        } else {
            output.push(ch);
        }
    }
    None
}

fn extract_xml_tag(raw: &str, tag: &str) -> Option<String> {
    let start_marker = format!("<{tag}>");
    let end_marker = format!("</{tag}>");
    let start = raw.find(&start_marker)? + start_marker.len();
    let after = &raw[start..];
    let end = after.find(&end_marker)?;
    Some(after[..end].trim().to_string())
}

fn repair_json_argument_string(value: &str) -> (String, bool) {
    let mut repaired = value.trim().to_string();
    let before = repaired.clone();
    loop {
        let next = repaired
            .replace(",}", "}")
            .replace(", }", " }")
            .replace(",]", "]")
            .replace(", ]", " ]");
        if next == repaired {
            break;
        }
        repaired = next;
    }
    let changed = repaired != before;
    (repaired, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_json_tool_call_with_object_arguments() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"name":"file.read","arguments":{"path":"src/parser.ts"}}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.tool_id, "file.read");
        assert_eq!(parsed.syntax, ToolCallSyntax::NativeJson);
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(args.path.as_deref(), Some("src/parser.ts"));
    }

    #[test]
    fn parses_function_arguments_string() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":\"src/parser.ts\"}"}}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.tool_id, "file.read");
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(args.path.as_deref(), Some("src/parser.ts"));
    }

    #[test]
    fn preserves_provider_tool_call_id_from_openai_tool_calls() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"id":"call_readme_123","function":{"name":"file.read","arguments":"{\"path\":\"README.md\"}"}}]}"#,
        )
        .unwrap();
        assert_eq!(
            parsed.provider_tool_call_id.as_deref(),
            Some("call_readme_123")
        );
        assert_eq!(parsed.tool_id, "file.read");
        assert_eq!(
            parse_tool_arguments(&parsed.arguments_json).path.as_deref(),
            Some("README.md")
        );
    }

    #[test]
    fn preserves_provider_tool_call_id_from_dsml_attributes() {
        let parsed = parse_first_tool_call(
            r#"<｜｜DSML｜｜invoke name="file.read" tool_call_id="toolu_dsml_7"><｜｜DSML｜｜parameter name="path">README.md</｜｜DSML｜｜parameter></｜｜DSML｜｜invoke>"#,
        )
        .unwrap();
        assert_eq!(
            parsed.provider_tool_call_id.as_deref(),
            Some("toolu_dsml_7")
        );
        assert_eq!(normalize_tool_id(&parsed.tool_id), "file.read");
    }

    #[test]
    fn decodes_unicode_escaped_tool_argument_strings() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"function":{"name":"file.write","arguments":"{\"path\":\"generated_app.html\",\"content\":\"\\u003c!DOCTYPE html\\u003e\\n\\u003chtml\\u003e\\n\\u003c/html\\u003e\"}"}}]}"#,
        )
        .unwrap();
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(
            args.content.as_deref(),
            Some("<!DOCTYPE html>\n<html>\n</html>")
        );
    }

    #[test]
    fn parses_native_json_multi_tool_calls() {
        let parsed = parse_tool_calls(
            r#"{"tool_calls":[{"function":{"name":"file_read","arguments":"{\"path\":\"README.md\"}"}},{"function":{"name":"file_read","arguments":"{\"path\":\"Cargo.toml\"}"}}]}"#,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(normalize_tool_id(&parsed[0].tool_id), "file.read");
        assert_eq!(normalize_tool_id(&parsed[1].tool_id), "file.read");
        assert_eq!(
            parse_tool_arguments(&parsed[0].arguments_json)
                .path
                .as_deref(),
            Some("README.md")
        );
        assert_eq!(
            parse_tool_arguments(&parsed[1].arguments_json)
                .path
                .as_deref(),
            Some("Cargo.toml")
        );
    }

    #[test]
    fn expands_read_files_dsml_array_into_file_reads() {
        let parsed = parse_tool_calls(
            r#"<｜｜DSML｜｜tool_calls><｜｜DSML｜｜invoke name="read_files"><｜｜DSML｜｜parameter name="files" string="false">["crates/runtime/src/runtime_facade.rs","crates/runtime/src/native_agent_loop.rs"]</｜｜DSML｜｜parameter></｜｜DSML｜｜invoke></｜｜DSML｜｜tool_calls>"#,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].tool_id, "file.read");
        assert_eq!(
            parse_tool_arguments(&parsed[0].arguments_json)
                .path
                .as_deref(),
            Some("crates/runtime/src/runtime_facade.rs")
        );
        assert_eq!(
            parse_tool_arguments(&parsed[1].arguments_json)
                .path
                .as_deref(),
            Some("crates/runtime/src/native_agent_loop.rs")
        );
    }

    #[test]
    fn parses_incomplete_dsml_invoke_as_recoverable_tool_call() {
        let parsed = parse_tool_calls(
            r#"<｜｜DSML｜｜tool_calls><｜｜DSML｜｜invoke name="list_available_tools">"#,
        );
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].tool_id, "list_available_tools");
        assert_eq!(parsed[0].arguments_json, "{}");
    }

    #[test]
    fn parses_deepseek_xml_tool_call() {
        let parsed = parse_first_tool_call(
            r#"<tool_call><name>search.ripgrep</name><arguments>{"query":"retry_count"}</arguments></tool_call>"#,
        )
        .unwrap();
        assert_eq!(parsed.tool_id, "search.ripgrep");
        assert_eq!(parsed.syntax, ToolCallSyntax::DeepSeekXml);
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(args.query.as_deref(), Some("retry_count"));
    }

    #[test]
    fn parses_deepseek_function_attribute_tool_calls() {
        let parsed = parse_tool_calls(
            r#"<function id="file.read" path="/tmp/project/Cargo.toml"></function>
<function id="git.status" root="."></function>"#,
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].tool_id, "file.read");
        let args = parse_tool_arguments(&parsed[0].arguments_json);
        assert_eq!(args.path.as_deref(), Some("/tmp/project/Cargo.toml"));
        assert_eq!(parsed[1].tool_id, "git.status");
        let args = parse_tool_arguments(&parsed[1].arguments_json);
        assert_eq!(args.root.as_deref(), Some("."));
    }

    #[test]
    fn parses_deepseek_dsml_multi_tool_calls() {
        let parsed = parse_tool_calls(
            r#"<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜invoke name="file_read">
<｜｜DSML｜｜parameter name="path" string="true">README.md</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
<｜｜DSML｜｜invoke name="file_read">
<｜｜DSML｜｜parameter name="path" string="true">crates/cli/Cargo.toml</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
<｜｜DSML｜｜invoke name="repo_file_tree">
<｜｜DSML｜｜parameter name="root" string="true">.</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
</｜｜DSML｜｜tool_calls>"#,
        );
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].syntax, ToolCallSyntax::DeepSeekDsml);
        assert_eq!(normalize_tool_id(&parsed[0].tool_id), "file.read");
        let args = parse_tool_arguments(&parsed[0].arguments_json);
        assert_eq!(args.path.as_deref(), Some("README.md"));
        let args = parse_tool_arguments(&parsed[1].arguments_json);
        assert_eq!(args.path.as_deref(), Some("crates/cli/Cargo.toml"));
        assert_eq!(normalize_tool_id(&parsed[2].tool_id), "file.list_tree");
        let args = parse_tool_arguments(&parsed[2].arguments_json);
        assert_eq!(args.root.as_deref(), Some("."));
    }

    #[test]
    fn strips_deepseek_dsml_from_visible_text() {
        let text = strip_tool_call_markup_from_visible_text(
            r#"好的，我来读取。
<｜｜DSML｜｜tool_calls>
<｜｜DSML｜｜invoke name="file_read">
<｜｜DSML｜｜parameter name="path" string="true">README.md</｜｜DSML｜｜parameter>
</｜｜DSML｜｜invoke>
</｜｜DSML｜｜tool_calls>"#,
        );
        assert_eq!(text, "好的，我来读取。");
    }

    #[test]
    fn repairs_trailing_comma_argument_json() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":\"src/parser.ts\",}"}}]}"#,
        )
        .unwrap();
        assert_eq!(parsed.status, ToolCallParseStatus::Repaired);
        assert!(parsed.repair_applied);
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(args.path.as_deref(), Some("src/parser.ts"));
    }

    #[test]
    fn normalizes_patch_propose_to_apply() {
        assert_eq!(normalize_tool_id("patch.propose"), "patch.apply");
        assert_eq!(normalize_tool_id("file_read"), "file.read");
        assert_eq!(normalize_tool_id("read_file"), "file.read");
        assert_eq!(normalize_tool_id("create_file"), "file.write");
        assert_eq!(normalize_tool_id("new_file"), "file.write");
        assert_eq!(normalize_tool_id("read_source_code"), "file.read");
        assert_eq!(normalize_tool_id("read-project-code"), "file.read");
        assert_eq!(normalize_tool_id("repo_file_tree"), "file.list_tree");
        assert_eq!(normalize_tool_id("repo_ls"), "repo.map");
        assert_eq!(normalize_tool_id("repo_list_path"), "repo.map");
        assert_eq!(normalize_tool_id("list_paths"), "repo.map");
        assert_eq!(normalize_tool_id("read_file_tree"), "file.list_tree");
        assert_eq!(normalize_tool_id("tree"), "file.list_tree");
        assert_eq!(normalize_tool_id("list_dir"), "file.list_directory");
        assert_eq!(normalize_tool_id("file_ls"), "file.list_directory");
        assert_eq!(normalize_tool_id("project-file-tree"), "file.list_tree");
        assert_eq!(normalize_tool_id("list_code_definition_names"), "repo.map");
        assert_eq!(normalize_tool_id("list_files"), "repo.map");
        assert_eq!(normalize_tool_id("search_text"), "search.ripgrep");
        assert_eq!(normalize_tool_id("patch_apply"), "patch.apply");
        assert_eq!(normalize_tool_id("execute_command"), "shell.command");
        assert_eq!(normalize_tool_id("exec_command"), "shell.command");
        assert_eq!(normalize_tool_id("artifact_view"), "file.read");
        assert_eq!(normalize_tool_id("view"), "file.read");
        assert_eq!(normalize_tool_id("open_file"), "file.read");
        assert_eq!(normalize_tool_id("file.read"), "file.read");
    }

    #[test]
    fn normalizes_deepseek_high_frequency_aliases_from_doc39() {
        for alias in ["read", "Read", "readFile", "fileRead", "ReadFile"] {
            assert_eq!(normalize_tool_id(alias), "file.read", "{alias}");
        }
        for alias in ["ls", "list", "ListDirectory", "ListDir", "list_dir"] {
            assert_eq!(normalize_tool_id(alias), "file.list_directory", "{alias}");
        }
        for alias in ["grep", "search", "SearchFiles", "rg", "ripgrep"] {
            assert_eq!(normalize_tool_id(alias), "search.ripgrep", "{alias}");
        }
        for alias in ["bash", "shell", "exec", "RunCommand", "execute_command"] {
            assert_eq!(normalize_tool_id(alias), "shell.command", "{alias}");
        }
        for alias in ["write", "WriteFile", "write_to_file", "save", "save_file"] {
            assert_eq!(normalize_tool_id(alias), "file.write", "{alias}");
        }
        for alias in ["edit", "EditFile", "modify", "PatchFile"] {
            assert_eq!(normalize_tool_id(alias), "file.edit", "{alias}");
        }
        for alias in ["plan", "Plan", "enter_plan", "EnterPlanMode"] {
            assert_eq!(normalize_tool_id(alias), "plan.enter", "{alias}");
        }
        for alias in ["todo", "TodoWrite", "write_todo"] {
            assert_eq!(normalize_tool_id(alias), "todo.write", "{alias}");
        }
        assert_eq!(normalize_tool_id("status"), "git.status");
    }

    #[test]
    fn parses_alias_argument_keys() {
        let args = parse_tool_arguments(
            r#"{"filePath":"src/main.rs","cwd":".","cmd":"ls","maxBytes":4096,"jobId":"j1","inputCsv":"data.csv","includeHidden":true,"maxEntries":32,"depth":2}"#,
        );
        assert_eq!(args.path.as_deref(), Some("src/main.rs"));
        assert_eq!(args.root.as_deref(), Some("."));
        assert_eq!(args.command.as_deref(), Some("ls"));
        assert_eq!(args.max_bytes, Some(4096));
        assert_eq!(args.job_id.as_deref(), Some("j1"));
        assert_eq!(args.input_csv.as_deref(), Some("data.csv"));
        assert_eq!(args.include_hidden, Some(true));
        assert_eq!(args.max_results, Some(32));
        assert_eq!(args.max_depth, Some(2));
    }

    #[test]
    fn parses_todo_items_array_as_content() {
        let args = parse_tool_arguments(
            r#"{"items":[{"content":"inspect runtime","status":"in_progress"}]}"#,
        );
        assert!(args.content.as_deref().unwrap().contains("inspect runtime"));
    }

    #[test]
    fn parses_task_dispatch_role_and_write_scope() {
        let args = parse_tool_arguments(
            r#"{"prompt":"inspect child","model_role":"reviewer","write_scope":{"paths":["src","tests"]}}"#,
        );
        assert_eq!(args.content.as_deref(), Some("inspect child"));
        assert_eq!(args.model_role.as_deref(), Some("reviewer"));
        assert_eq!(
            args.write_scope_json.as_deref(),
            Some(r#"{"paths":["src","tests"]}"#)
        );
    }

    #[test]
    fn parses_research_csv_profile_arguments() {
        let parsed = parse_first_tool_call(
            r#"{"tool_calls":[{"name":"research.csv_profile","arguments":{"job_id":"job1","input_csv":"data/results.csv"}}]}"#,
        )
        .unwrap();
        let args = parse_tool_arguments(&parsed.arguments_json);
        assert_eq!(args.job_id.as_deref(), Some("job1"));
        assert_eq!(args.input_csv.as_deref(), Some("data/results.csv"));
    }

    #[test]
    fn no_tool_returns_none() {
        assert!(parse_first_tool_call("plain answer without a tool").is_none());
    }
}
