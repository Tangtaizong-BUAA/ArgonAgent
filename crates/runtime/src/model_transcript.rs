//! Model call transcript artifact support.
//!
//! V0 stores only sanitized request/response previews and adapter metadata.
//! Full raw provider traffic must stay out of the artifact store until privacy
//! policy and user approval rules are implemented.

use crate::artifact::{ArtifactKind, ArtifactRecord, ArtifactStore};
use crate::model_adapter::{ModelRole, PlannedModelCall, ThinkingMode};
use crate::native_profile::deepseek::stream::DeepSeekStreamAssembly;
use crate::qwen_stream::QwenStreamAssembly;
use researchcode_kernel::model::OptimizationLevel;
use std::io;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTranscript {
    pub transcript_id: String,
    pub adapter_id: String,
    pub optimization_level: OptimizationLevel,
    pub actual_model_name: String,
    pub display_model_name: String,
    pub role: ModelRole,
    pub thinking_mode: ThinkingMode,
    pub parser_profile: String,
    pub native_tool_calls: bool,
    pub request_preview: String,
    pub response_preview: String,
    pub prompt_tokens_estimate: u64,
    pub response_tokens_estimate: u64,
    pub reasoning_persisted: bool,
    pub privacy_class: String,
}

impl ModelTranscript {
    pub fn from_planned_call(
        transcript_id: impl Into<String>,
        role: ModelRole,
        plan: &PlannedModelCall,
        request_preview: impl Into<String>,
        response_preview: impl Into<String>,
    ) -> Self {
        Self {
            transcript_id: transcript_id.into(),
            adapter_id: plan.adapter_id.clone(),
            optimization_level: plan.optimization_level.clone(),
            actual_model_name: plan.actual_model_name.clone(),
            display_model_name: plan.display_model_name.clone(),
            role,
            thinking_mode: plan.thinking_mode.clone(),
            parser_profile: plan.parser_profile.clone(),
            native_tool_calls: plan.native_tool_calls,
            request_preview: sanitize_transcript_text(&request_preview.into()),
            response_preview: sanitize_transcript_text(&response_preview.into()),
            prompt_tokens_estimate: 0,
            response_tokens_estimate: 0,
            reasoning_persisted: false,
            privacy_class: "internal".to_string(),
        }
    }

    pub fn from_deepseek_stream_assembly(
        transcript_id: impl Into<String>,
        role: ModelRole,
        plan: &PlannedModelCall,
        request_preview: impl Into<String>,
        assembly: &DeepSeekStreamAssembly,
    ) -> Self {
        let mut transcript = Self::from_planned_call(
            transcript_id,
            role,
            plan,
            request_preview,
            &assembly.content,
        );
        transcript.prompt_tokens_estimate = assembly.telemetry.prompt_tokens.unwrap_or(0);
        transcript.response_tokens_estimate = assembly.telemetry.completion_tokens.unwrap_or(0);
        transcript.reasoning_persisted = !assembly.reasoning_sanitized.is_empty();
        transcript
    }

    pub fn from_qwen_stream_assembly(
        transcript_id: impl Into<String>,
        role: ModelRole,
        plan: &PlannedModelCall,
        request_preview: impl Into<String>,
        assembly: &QwenStreamAssembly,
    ) -> Self {
        let mut transcript = Self::from_planned_call(
            transcript_id,
            role,
            plan,
            request_preview,
            &assembly.content,
        );
        transcript.prompt_tokens_estimate = assembly.telemetry.prompt_tokens.unwrap_or(0);
        transcript.response_tokens_estimate = assembly.telemetry.completion_tokens.unwrap_or(0);
        transcript.reasoning_persisted = !assembly.thinking_sanitized.is_empty();
        transcript
    }
}

pub fn write_model_transcript_artifact(
    store: &ArtifactStore,
    transcript: &ModelTranscript,
) -> Result<ArtifactRecord, io::Error> {
    store.put_bytes_auto_hash(
        &transcript.transcript_id,
        ArtifactKind::ModelTranscript,
        &transcript.privacy_class,
        model_transcript_json(transcript).as_bytes(),
    )
}

pub fn model_transcript_json(transcript: &ModelTranscript) -> String {
    format!(
        "{{\"schema_version\":\"model_transcript.v0\",\"transcript_id\":\"{}\",\"adapter_id\":\"{}\",\"optimization_level\":\"{}\",\"actual_model_name\":\"{}\",\"display_model_name\":\"{}\",\"role\":\"{}\",\"thinking_mode\":\"{}\",\"parser_profile\":\"{}\",\"native_tool_calls\":{},\"request_preview\":\"{}\",\"response_preview\":\"{}\",\"prompt_tokens_estimate\":{},\"response_tokens_estimate\":{},\"reasoning_persisted\":{},\"privacy_class\":\"{}\"}}",
        escape(&transcript.transcript_id),
        escape(&transcript.adapter_id),
        optimization_level_to_str(&transcript.optimization_level),
        escape(&transcript.actual_model_name),
        escape(&transcript.display_model_name),
        role_to_str(&transcript.role),
        thinking_mode_to_str(&transcript.thinking_mode),
        escape(&transcript.parser_profile),
        transcript.native_tool_calls,
        escape(&transcript.request_preview),
        escape(&transcript.response_preview),
        transcript.prompt_tokens_estimate,
        transcript.response_tokens_estimate,
        transcript.reasoning_persisted,
        escape(&transcript.privacy_class)
    )
}

pub fn sanitize_transcript_text(value: &str) -> String {
    let mut sanitized = value.to_string();
    sanitized = redact_sk_secrets(&sanitized);
    sanitized = redact_after_prefix(&sanitized, "AKIA");
    sanitized = redact_dotenv_filename(&sanitized);
    sanitized
}

fn redact_after_prefix(value: &str, prefix: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find(prefix) {
        output.push_str(&rest[..index]);
        output.push_str("[REDACTED_SECRET]");
        let after_prefix = &rest[index + prefix.len()..];
        let end = after_prefix
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'' || ch == ',')
            .unwrap_or(after_prefix.len());
        rest = &after_prefix[end..];
    }
    output.push_str(rest);
    output
}

/// Redact "sk-" API key prefix only when it looks like a real secret:
/// preceded by a word boundary (whitespace, `=`, `:`, `"`, `'`) and followed
/// by at least 16 non-delimiter characters. This avoids false matches on
/// "ask-", "task-", "desk-", etc.
fn redact_sk_secrets(value: &str) -> String {
    let mut output = String::new();
    let mut rest = value;
    while let Some(index) = rest.find("sk-") {
        // Check that "sk-" is at a word boundary (not part of "ask-", "task-", etc.)
        let is_boundary = index == 0 || {
            let prev = rest.as_bytes()[index - 1] as char;
            prev.is_whitespace()
                || prev == '='
                || prev == ':'
                || prev == '"'
                || prev == '\''
                || prev == ','
        };
        let after_prefix = &rest[index + 3..];
        let end = after_prefix
            .find(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'' || ch == ',')
            .unwrap_or(after_prefix.len());
        if is_boundary && end > 0 {
            output.push_str(&rest[..index]);
            output.push_str("[REDACTED_SECRET]");
            rest = &after_prefix[end..];
        } else {
            output.push_str(&rest[..index + 3]);
            rest = after_prefix;
        }
    }
    output.push_str(rest);
    output
}

/// Only replace ".env" when it is a standalone filename (surrounded by path
/// separators, whitespace, quotes, or string boundaries). This avoids
/// redacting "process.env.NODE_ENV" and similar dot-access expressions.
fn redact_dotenv_filename(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    for (index, _) in value.match_indices(".env") {
        let before = value[..index]
            .chars()
            .next_back()
            .map(is_dotenv_boundary_before)
            .unwrap_or(true);
        let after_index = index + ".env".len();
        let after = value[after_index..]
            .chars()
            .next()
            .map(is_dotenv_boundary_after)
            .unwrap_or(true);
        if before && after {
            output.push_str(&value[cursor..index]);
            output.push_str("[REDACTED_PATH]");
            cursor = after_index;
        }
    }
    output.push_str(&value[cursor..]);
    output
}

fn is_dotenv_boundary_before(c: char) -> bool {
    c == '/' || c == '\\' || c.is_whitespace() || c == '"' || c == '\'' || c == '`'
}

fn is_dotenv_boundary_after(c: char) -> bool {
    c == '/' || c == '\\' || c.is_whitespace() || c == '"' || c == '\'' || c == ','
}

fn optimization_level_to_str(value: &OptimizationLevel) -> &'static str {
    match value {
        OptimizationLevel::Native => "native",
        OptimizationLevel::Compatible => "compatible",
        OptimizationLevel::Baseline => "baseline",
    }
}

fn role_to_str(value: &ModelRole) -> &'static str {
    match value {
        ModelRole::Planner => "planner",
        ModelRole::Executor => "executor",
        ModelRole::Reviewer => "reviewer",
        ModelRole::Researcher => "researcher",
        ModelRole::Summarizer => "summarizer",
    }
}

fn thinking_mode_to_str(value: &ThinkingMode) -> &'static str {
    match value {
        ThinkingMode::Thinking => "thinking",
        ThinkingMode::NonThinking => "non_thinking",
        ThinkingMode::PreserveThinking => "preserve_thinking",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_adapter::{DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest};
    use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn transcript_sanitizes_secrets_and_writes_artifact() {
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "plan".to_string(),
                requires_tools: true,
                context_tokens_estimate: 100,
            })
            .unwrap();
        let transcript = ModelTranscript::from_planned_call(
            "transcript_1",
            ModelRole::Planner,
            &plan,
            "read .env sk-testsecret",
            "ok",
        );
        assert!(!transcript.request_preview.contains("sk-testsecret"));
        assert!(!transcript.request_preview.contains(".env"));

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-model-transcript-{nonce}"));
        let store = ArtifactStore::new(&root);
        let record = write_model_transcript_artifact(&store, &transcript).unwrap();
        assert_eq!(record.kind, ArtifactKind::ModelTranscript);
        let content = String::from_utf8(store.read_bytes(&record).unwrap()).unwrap();
        assert!(content.contains("\"schema_version\":\"model_transcript.v0\""));
        assert!(content.contains("\"optimization_level\":\"native\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn transcript_sanitizer_handles_utf8_and_short_sk_tokens() {
        assert_eq!(
            sanitize_transcript_text("我先看看关键文件。"),
            "我先看看关键文件。"
        );
        let sanitized = sanitize_transcript_text("read .env sk-testsecret");
        assert!(!sanitized.contains(".env"));
        assert!(!sanitized.contains("sk-testsecret"));
        assert!(sanitized.contains("[REDACTED_PATH]"));
        assert!(sanitized.contains("[REDACTED_SECRET]"));
        assert_eq!(
            sanitize_transcript_text("ask-user task-run desk-lamp"),
            "ask-user task-run desk-lamp"
        );
    }

    #[test]
    fn deepseek_stream_transcript_uses_visible_content_and_reasoning_flag() {
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "plan".to_string(),
                requires_tools: true,
                context_tokens_estimate: 100,
            })
            .unwrap();
        let assembly = crate::native_profile::deepseek::stream::assemble_deepseek_sse_lines(&[
            r#"data: {"choices":[{"delta":{"reasoning_content":"Secret sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Visible answer"}}]}"#,
            r#"data: {"usage":{"prompt_tokens":10,"completion_tokens":5,"reasoning_tokens":4}}"#,
        ])
        .unwrap();
        let transcript = ModelTranscript::from_deepseek_stream_assembly(
            "stream_transcript_1",
            ModelRole::Planner,
            &plan,
            "request",
            &assembly,
        );
        assert_eq!(transcript.response_preview, "Visible answer");
        assert!(transcript.reasoning_persisted);
        assert_eq!(transcript.prompt_tokens_estimate, 10);
        assert_eq!(transcript.response_tokens_estimate, 5);
        assert!(!transcript.response_preview.contains("sk-testsecret"));
    }

    #[test]
    fn qwen_stream_transcript_uses_visible_content_and_thinking_flag() {
        let adapter = crate::model_adapter::QwenNativeAdapter::new(
            NativeModelProfile {
                profile_id: "qwen3-6-27b-native".to_string(),
                family: NativeModelFamily::Qwen,
                optimization_level: OptimizationLevel::Native,
            },
            "Qwen/Qwen3.6-27B",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Executor,
                task_summary: "patch".to_string(),
                requires_tools: true,
                context_tokens_estimate: 100,
            })
            .unwrap();
        let assembly = crate::qwen_stream::assemble_qwen_sse_lines(&[
            r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"reasoning_content":"Secret sk-testsecret from .env"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":"Visible patch summary"}}]}"#,
            r#"data: {"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        ])
        .unwrap();
        let transcript = ModelTranscript::from_qwen_stream_assembly(
            "qwen_stream_transcript_1",
            ModelRole::Executor,
            &plan,
            "request",
            &assembly,
        );
        assert_eq!(transcript.response_preview, "Visible patch summary");
        assert!(transcript.reasoning_persisted);
        assert_eq!(transcript.prompt_tokens_estimate, 10);
        assert_eq!(transcript.response_tokens_estimate, 5);
        assert!(!transcript.response_preview.contains("sk-testsecret"));
    }
}
