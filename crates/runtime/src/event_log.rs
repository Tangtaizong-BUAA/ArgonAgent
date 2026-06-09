//! In-memory append-only event log.

use researchcode_kernel::{Actor, KernelEvent};
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventLogError {
    NonMonotonicSequence {
        expected: u64,
        actual: u64,
    },
    PrevHashMismatch {
        expected: String,
        actual: Option<String>,
    },
    EmptyHash,
    Io(String),
    Parse(String),
}

#[derive(Debug, Default, Clone)]
pub struct EventLog {
    events: Vec<KernelEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedContext {
    pub boundary_event: Option<String>,
    pub preserved_messages: Vec<String>,
    pub summary_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedEventRef {
    pub reference: String,
    pub event: KernelEvent,
    pub projected_message: String,
}

impl EventLog {
    pub fn append(&mut self, event: KernelEvent) -> Result<(), EventLogError> {
        let expected_sequence = self
            .events
            .last()
            .map(|last| last.sequence + 1)
            .unwrap_or(1);
        if event.sequence != expected_sequence {
            return Err(EventLogError::NonMonotonicSequence {
                expected: expected_sequence,
                actual: event.sequence,
            });
        }
        if event.hash.is_empty() {
            return Err(EventLogError::EmptyHash);
        }
        if let Some(last) = self.events.last() {
            if event.prev_hash.as_deref() != Some(last.hash.as_str()) {
                return Err(EventLogError::PrevHashMismatch {
                    expected: last.hash.clone(),
                    actual: event.prev_hash.clone(),
                });
            }
        }
        self.events.push(event);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn last(&self) -> Option<&KernelEvent> {
        self.events.last()
    }

    pub fn iter(&self) -> impl Iterator<Item = &KernelEvent> {
        self.events.iter()
    }

    pub fn event_by_sequence(&self, sequence: u64) -> Option<&KernelEvent> {
        self.events.iter().find(|event| event.sequence == sequence)
    }

    pub fn page_ref(&self, reference: &str) -> Option<PagedEventRef> {
        let sequence = reference.strip_prefix("ref://event/")?.parse().ok()?;
        let event = self.event_by_sequence(sequence)?;
        Some(PagedEventRef {
            reference: reference.to_string(),
            event: event.clone(),
            projected_message: project_event_message(event),
        })
    }

    pub fn export_jsonl(&self) -> String {
        let mut output = String::new();
        for event in &self.events {
            output.push_str(&event_to_json(event));
            output.push('\n');
        }
        output
    }

    pub fn write_jsonl(&self, path: &Path) -> Result<(), EventLogError> {
        fs::write(path, self.export_jsonl()).map_err(|error| EventLogError::Io(error.to_string()))
    }

    pub fn import_jsonl(input: &str) -> Result<Self, EventLogError> {
        let mut log = Self::default();
        for (index, line) in input.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let event = event_from_json(line)
                .map_err(|error| EventLogError::Parse(format!("line {}: {error}", index + 1)))?;
            log.append(event)?;
        }
        Ok(log)
    }

    pub fn parse_jsonl_event(line: &str) -> Result<KernelEvent, EventLogError> {
        event_from_json(line).map_err(EventLogError::Parse)
    }

    pub fn read_jsonl(path: &Path) -> Result<Self, EventLogError> {
        let text =
            fs::read_to_string(path).map_err(|error| EventLogError::Io(error.to_string()))?;
        Self::import_jsonl(&text)
    }

    pub fn project_context(
        &self,
        boundary_sequence: Option<u64>,
        preserved_event_count: usize,
        summary_text: impl Into<String>,
    ) -> ProjectedContext {
        let boundary_event = boundary_sequence
            .and_then(|sequence| self.events.iter().find(|event| event.sequence == sequence))
            .map(|event| event.event_id.clone());
        let preserved_messages = self
            .events
            .iter()
            .rev()
            .take(preserved_event_count)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(project_event_message)
            .collect();
        ProjectedContext {
            boundary_event,
            preserved_messages,
            summary_text: summary_text.into(),
        }
    }
}

impl From<io::Error> for EventLogError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

fn event_to_json(event: &KernelEvent) -> String {
    let session_id = optional_json_string(event.session_id.as_deref());
    let task_id = optional_json_string(event.task_id.as_deref());
    let prev_hash = optional_json_string(event.prev_hash.as_deref());
    format!(
        "{{\"event_id\":\"{}\",\"schema_version\":\"{}\",\"project_id\":\"{}\",\"session_id\":{},\"task_id\":{},\"sequence\":{},\"event_type\":\"{}\",\"actor\":\"{}\",\"created_at\":\"{}\",\"payload\":{},\"prev_hash\":{},\"hash\":\"{}\"}}",
        escape(&event.event_id),
        escape(&event.schema_version),
        escape(&event.project_id),
        session_id,
        task_id,
        event.sequence,
        escape(&event.event_type),
        actor_to_str(&event.actor),
        escape(&event.created_at),
        event.payload_json,
        prev_hash,
        escape(&event.hash)
    )
}

fn project_event_message(event: &KernelEvent) -> String {
    format!(
        "{}#{} {} {}",
        event.event_type,
        event.sequence,
        actor_label(&event.actor),
        event.payload_json
    )
}

fn actor_label(actor: &Actor) -> &'static str {
    match actor {
        Actor::User => "User",
        Actor::Agent => "Agent",
        Actor::Runtime => "Runtime",
        Actor::Tool => "Tool",
        Actor::Model => "Model",
        Actor::ResearchWorker => "ResearchWorker",
        Actor::Policy => "Policy",
    }
}

fn event_from_json(line: &str) -> Result<KernelEvent, String> {
    let event_id = extract_json_string(line, "event_id")?;
    let schema_version = extract_json_string(line, "schema_version")?;
    let project_id = extract_json_string(line, "project_id")?;
    let session_id = extract_optional_json_string(line, "session_id")?;
    let task_id = extract_optional_json_string(line, "task_id")?;
    let sequence = extract_json_u64(line, "sequence")?;
    let event_type = extract_json_string(line, "event_type")?;
    let actor = actor_from_str(&extract_json_string(line, "actor")?)?;
    let created_at = extract_json_string(line, "created_at")?;
    let payload_json = extract_json_object(line, "payload")?;
    let prev_hash = extract_optional_json_string_last(line, "prev_hash")?;
    let hash = extract_json_string_last(line, "hash")?;
    Ok(KernelEvent {
        event_id,
        schema_version,
        project_id,
        session_id,
        task_id,
        sequence,
        event_type,
        actor,
        created_at,
        payload_json,
        prev_hash,
        hash,
    })
}

fn optional_json_string(value: Option<&str>) -> String {
    match value {
        Some(value) => format!("\"{}\"", escape(value)),
        None => "null".to_string(),
    }
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

fn actor_to_str(actor: &Actor) -> &'static str {
    match actor {
        Actor::User => "user",
        Actor::Agent => "agent",
        Actor::Runtime => "runtime",
        Actor::Tool => "tool",
        Actor::Model => "model",
        Actor::ResearchWorker => "research_worker",
        Actor::Policy => "policy",
    }
}

fn actor_from_str(value: &str) -> Result<Actor, String> {
    match value {
        "user" => Ok(Actor::User),
        "agent" => Ok(Actor::Agent),
        "runtime" => Ok(Actor::Runtime),
        "tool" => Ok(Actor::Tool),
        "model" => Ok(Actor::Model),
        "research_worker" => Ok(Actor::ResearchWorker),
        "policy" => Ok(Actor::Policy),
        _ => Err(format!("unknown actor {value}")),
    }
}

fn extract_json_string(line: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\":\"");
    let start = line
        .find(&marker)
        .ok_or_else(|| format!("missing key {key}"))?
        + marker.len()
        - 1;
    parse_json_string_at(line, start).ok_or_else(|| format!("unterminated string {key}"))
}

fn extract_optional_json_string(line: &str, key: &str) -> Result<Option<String>, String> {
    let marker = format!("\"{key}\":");
    let start = match line.find(&marker) {
        Some(start) => start + marker.len(),
        None => return Ok(None),
    };
    let rest = line[start..].trim_start();
    if rest.starts_with("null") {
        return Ok(None);
    }
    if !rest.starts_with('"') {
        return Err(format!("optional key {key} is not string or null"));
    }
    parse_json_string_at(rest, 0)
        .map(Some)
        .ok_or_else(|| format!("unterminated string {key}"))
}

/// Like `extract_json_string` but searches from the end of the line.
/// Used for keys (`hash`, `prev_hash`) that appear after the payload and
/// could collide with identically-named fields inside the payload JSON.
fn extract_json_string_last(line: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\":\"");
    let start = line
        .rfind(&marker)
        .ok_or_else(|| format!("missing key {key}"))?
        + marker.len()
        - 1;
    parse_json_string_at(line, start).ok_or_else(|| format!("unterminated string {key}"))
}

/// Like `extract_optional_json_string` but searches from the end of the line.
fn extract_optional_json_string_last(line: &str, key: &str) -> Result<Option<String>, String> {
    let marker = format!("\"{key}\":");
    let start = match line.rfind(&marker) {
        Some(start) => start + marker.len(),
        None => return Ok(None),
    };
    let rest = line[start..].trim_start();
    if rest.starts_with("null") {
        return Ok(None);
    }
    if !rest.starts_with('"') {
        return Err(format!("optional key {key} is not string or null"));
    }
    parse_json_string_at(rest, 0)
        .map(Some)
        .ok_or_else(|| format!("unterminated string {key}"))
}

fn extract_json_u64(line: &str, key: &str) -> Result<u64, String> {
    let marker = format!("\"{key}\":");
    let start = line
        .find(&marker)
        .ok_or_else(|| format!("missing key {key}"))?
        + marker.len();
    let rest = &line[start..];
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return Err(format!("missing numeric value {key}"));
    }
    digits.parse::<u64>().map_err(|error| error.to_string())
}

fn extract_json_object(line: &str, key: &str) -> Result<String, String> {
    let marker = format!("\"{key}\":");
    let start = line
        .find(&marker)
        .ok_or_else(|| format!("missing key {key}"))?
        + marker.len();
    let rest = &line[start..];
    let object_start = rest
        .find('{')
        .ok_or_else(|| format!("payload {key} is not object"))?;
    let mut depth = 0_i32;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in rest[object_start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let end = object_start + offset + 1;
                    return Ok(rest[object_start..end].to_string());
                }
            }
            _ => {}
        }
    }
    Err(format!("unterminated object {key}"))
}

fn parse_json_string_at(input: &str, quote_start: usize) -> Option<String> {
    let text = &input[quote_start..];
    if !text.starts_with('"') {
        return None;
    }
    let bytes = text[1..].as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == b'\\' {
            i += 1;
            if i >= bytes.len() {
                break;
            }
            match bytes[i] {
                b'"' => out.push('"'),
                b'\\' => out.push('\\'),
                b'/' => out.push('/'),
                b'n' => out.push('\n'),
                b'r' => out.push('\r'),
                b't' => out.push('\t'),
                b'b' => out.push('\u{0008}'),
                b'f' => out.push('\u{000c}'),
                b'u' => {
                    // Decode \uXXXX escape
                    i += 1;
                    if i + 4 > bytes.len() {
                        break;
                    }
                    let hex_str = std::str::from_utf8(&bytes[i..i + 4]).ok()?;
                    let code = u32::from_str_radix(hex_str, 16).ok()?;
                    if let Some(decoded) = char::from_u32(code) {
                        out.push(decoded);
                    } else {
                        // Invalid Unicode scalar — keep the escape literal
                        out.push_str("\\u");
                        out.push_str(hex_str);
                    }
                    i += 3; // +1 from loop increment = total +4
                }
                _ => out.push(bytes[i] as char),
            }
        } else if ch == b'"' {
            return Some(out);
        } else {
            out.push(ch as char);
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn event(sequence: u64, prev_hash: Option<&str>, hash: &str) -> KernelEvent {
        KernelEvent {
            event_id: format!("evt_{sequence}"),
            schema_version: "v0".to_string(),
            project_id: "proj".to_string(),
            session_id: Some("sess".to_string()),
            task_id: Some("task".to_string()),
            sequence,
            event_type: "session.state_changed".to_string(),
            actor: Actor::Runtime,
            created_at: "now".to_string(),
            payload_json: "{}".to_string(),
            prev_hash: prev_hash.map(ToOwned::to_owned),
            hash: hash.to_string(),
        }
    }

    #[test]
    fn appends_valid_hash_chain() {
        let mut log = EventLog::default();
        log.append(event(1, None, "h1")).unwrap();
        log.append(event(2, Some("h1"), "h2")).unwrap();
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn rejects_bad_sequence() {
        let mut log = EventLog::default();
        let error = log.append(event(2, None, "h2")).unwrap_err();
        assert_eq!(
            error,
            EventLogError::NonMonotonicSequence {
                expected: 1,
                actual: 2
            }
        );
    }

    #[test]
    fn rejects_bad_prev_hash() {
        let mut log = EventLog::default();
        log.append(event(1, None, "h1")).unwrap();
        let error = log.append(event(2, Some("wrong"), "h2")).unwrap_err();
        assert_eq!(
            error,
            EventLogError::PrevHashMismatch {
                expected: "h1".to_string(),
                actual: Some("wrong".to_string())
            }
        );
    }

    #[test]
    fn exports_and_imports_jsonl() {
        let mut log = EventLog::default();
        log.append(event(1, None, "h1")).unwrap();
        log.append(event(2, Some("h1"), "h2")).unwrap();
        let jsonl = log.export_jsonl();
        let imported = EventLog::import_jsonl(&jsonl).unwrap();
        assert_eq!(imported.len(), 2);
        assert_eq!(imported.last().unwrap().hash, "h2");
    }

    #[test]
    fn imports_payload_with_json_braces_inside_string() {
        let mut log = EventLog::default();
        let mut first = event(1, None, "h1");
        first.event_type = "model.stream_delta".to_string();
        first.actor = Actor::Model;
        first.payload_json = "{\"stream_id\":\"s1\",\"provider\":\"deepseek\",\"delta_kind\":\"content\",\"preview\":\"<｜｜DSML｜｜tool_calls>{\\\"path\\\":\\\"README.md\\\",\\\"args\\\":{\\\"max_bytes\\\":8000}}</｜｜DSML｜｜tool_calls>\"}".to_string();
        log.append(first).unwrap();
        let jsonl = log.export_jsonl();
        let imported = EventLog::import_jsonl(&jsonl).unwrap();
        assert_eq!(imported.len(), 1);
        assert!(imported
            .last()
            .unwrap()
            .payload_json
            .contains("<｜｜DSML｜｜tool_calls>"));
    }

    #[test]
    fn import_round_trips_escaped_quotes_in_top_level_strings() {
        let mut log = EventLog::default();
        let mut first = event(1, None, "h1");
        first.event_id = "evt_\"quoted\"".to_string();
        log.append(first).unwrap();
        let jsonl = log.export_jsonl();
        let imported = EventLog::import_jsonl(&jsonl).unwrap();
        assert_eq!(imported.last().unwrap().event_id, "evt_\"quoted\"");
    }

    #[test]
    fn projects_context_without_mutating_raw_event_log() {
        let mut log = EventLog::default();
        log.append(event(1, None, "h1")).unwrap();
        log.append(event(2, Some("h1"), "h2")).unwrap();
        log.append(event(3, Some("h2"), "h3")).unwrap();

        let projection = log.project_context(Some(2), 2, "summary text");

        assert_eq!(log.len(), 3);
        assert_eq!(projection.boundary_event.as_deref(), Some("evt_2"));
        assert_eq!(projection.summary_text, "summary text");
        assert_eq!(projection.preserved_messages.len(), 2);
        assert!(projection.preserved_messages[0].contains("session.state_changed#2"));
        assert!(projection.preserved_messages[1].contains("session.state_changed#3"));
    }

    #[test]
    fn pages_raw_event_back_from_reversible_ref() {
        let mut log = EventLog::default();
        log.append(event(1, None, "h1")).unwrap();
        let mut second = event(2, Some("h1"), "h2");
        second.event_type = "tool.result_recorded".to_string();
        second.payload_json =
            "{\"tool_call_id\":\"tc1\",\"preview\":\"Package.swift contains XCTest\"}".to_string();
        log.append(second).unwrap();

        let paged = log.page_ref("ref://event/2").unwrap();

        assert_eq!(paged.reference, "ref://event/2");
        assert_eq!(paged.event.sequence, 2);
        assert_eq!(paged.event.event_type, "tool.result_recorded");
        assert!(paged.event.payload_json.contains("XCTest"));
        assert!(paged.projected_message.contains("tool.result_recorded#2"));
        assert!(log.page_ref("ref://event/999").is_none());
        assert!(log.page_ref("event/2").is_none());
    }
}
