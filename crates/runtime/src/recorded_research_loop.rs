//! Recorded research agent loop.
//!
//! This is the Research Coworker counterpart to the coding agent loop. It
//! records a native Qwen researcher response, parses a `research.csv_profile`
//! tool call, executes the Python sidecar with bounded local-only policy, and
//! stores the result as a runtime tool artifact.

use crate::artifact::ArtifactStore;
use crate::model_adapter::{ModelAdapter, ModelAdapterRequest, ModelRole, QwenNativeAdapter};
use crate::parser::{classify_qwen_output, ParserAction};
use crate::provider_response_adapter::{
    record_native_provider_response, NativeProviderResponseInput, NativeProviderStreamKind,
};
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::tool_dispatcher::{schedule_tool_calls, ScheduledToolCall};
use crate::tool_execution::{
    execute_tool, ToolExecutionArgs, ToolExecutionMode, ToolExecutionRequest,
};
use crate::tool_result::{json_string, write_tool_result_artifact, ToolResultRecord};
use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedResearchLoopConfig {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub cleanup: bool,
}

impl Default for RecordedResearchLoopConfig {
    fn default() -> Self {
        Self {
            project_id: "proj".to_string(),
            session_id: "sess_recorded_research_loop".to_string(),
            task_id: "research_task".to_string(),
            cleanup: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedResearchLoopResult {
    pub final_state: AgentState,
    pub event_count: usize,
    pub manifest_hash: String,
    pub artifact_count: usize,
    pub event_jsonl: String,
}

pub fn run_recorded_research_loop_fixture(
    config: &RecordedResearchLoopConfig,
) -> Result<RecordedResearchLoopResult, String> {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| "cannot resolve workspace root".to_string())?
        .to_path_buf();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let root = std::env::temp_dir().join(format!("researchcode-recorded-research-loop-{nonce}"));
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let artifact_store = ArtifactStore::new(root.join("artifacts"));
    let input_csv = workspace_root
        .join("eval/fixtures/research/csv-quality-small/input.csv")
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let output_dir = root.join("research_worker_output");
    let mut session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
        .map_err(|error| format!("{error:?}"))?;
    session
        .transition_to(AgentState::Planning)
        .and_then(|_| session.transition_to(AgentState::RetrievingContext))
        .and_then(|_| session.transition_to(AgentState::Executing))
        .map_err(|error| format!("{error:?}"))?;

    let adapter = QwenNativeAdapter::new(
        NativeModelProfile {
            profile_id: "qwen3-6-27b-native".to_string(),
            family: NativeModelFamily::Qwen,
            optimization_level: OptimizationLevel::Native,
        },
        "Qwen/Qwen3.6-27B",
    )?;
    let plan = adapter.plan_call(&ModelAdapterRequest {
        role: ModelRole::Researcher,
        task_summary: "profile a research CSV".to_string(),
        requires_tools: true,
        context_tokens_estimate: 4_000,
    })?;
    let raw_response = format!(
        "{{\"model\":\"Qwen/Qwen3.6-27B\",\"tool_calls\":[{{\"name\":\"research.csv_profile\",\"arguments\":{{\"input_csv\":{},\"job_id\":\"recorded_research_loop\"}}}}]}}",
        json_string(&input_csv.to_string_lossy())
    );
    record_native_provider_response(
        &mut session,
        &artifact_store,
        NativeProviderResponseInput {
            provider: NativeProviderStreamKind::Qwen,
            call_id: "recorded_research_call_1",
            stream_id: "recorded_research_stream_1",
            role: ModelRole::Researcher,
            plan: &plan,
            request_preview: "recorded research loop request",
            transcript_id: "recorded_research_transcript_1",
            live: false,
            visible_content: &raw_response,
            hidden_reasoning_sanitized: None,
            prompt_tokens: 64,
            completion_tokens: 24,
            reasoning_tokens: 0,
            prompt_cache_hit_tokens: 0,
            prompt_cache_miss_tokens: 0,
        },
    )?;
    let parsed = classify_qwen_output(&raw_response);
    if parsed.action != ParserAction::Execute
        || parsed.tool_id.as_deref() != Some("research.csv_profile")
    {
        return Err(format!("unexpected research parser result: {parsed:?}"));
    }
    schedule_tool_calls(vec![ScheduledToolCall {
        tool_call_id: "recorded_research_tool_1".to_string(),
        tool_id: "research.csv_profile".to_string(),
    }])
    .map_err(|error| format!("{error:?}"))?;
    session
        .record_tool_call_requested("recorded_research_tool_1", "research.csv_profile")
        .map_err(|error| format!("{error:?}"))?;
    let worker_result = execute_tool(&ToolExecutionRequest {
        workspace_root: workspace_root.clone(),
        tool_call_id: "recorded_research_tool_1".to_string(),
        tool_id: "research.csv_profile".to_string(),
        mode: ToolExecutionMode::ReadOnlyPreview,
        args: ToolExecutionArgs {
            input_csv: Some(input_csv.to_string_lossy().to_string()),
            output_dir: Some(output_dir.to_string_lossy().to_string()),
            job_id: Some("recorded_research_loop".to_string()),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    let ok = worker_result.ok;
    session
        .record_tool_call_completed("recorded_research_tool_1", "research.csv_profile", ok)
        .map_err(|error| format!("{error:?}"))?;
    if !ok {
        return Err(format!(
            "research worker failed: {}",
            worker_result.detail_json
        ));
    }
    let manifest_hash = extract_json_string(&worker_result.detail_json, "manifest_hash")
        .ok_or_else(|| "missing research manifest hash".to_string())?;
    let artifact_count = extract_json_usize(&worker_result.detail_json, "artifact_count")
        .ok_or_else(|| "missing research artifact count".to_string())?;
    let artifact = write_tool_result_artifact(
        &artifact_store,
        "recorded_research_tool_result_1",
        &ToolResultRecord::new(
            "recorded_research_tool_1",
            "research.csv_profile",
            true,
            worker_result.preview.clone(),
            worker_result.detail_json,
        ),
    )
    .map_err(|error| error.to_string())?;
    session
        .record_tool_result_artifact(
            "recorded_research_tool_1",
            "research.csv_profile",
            artifact.artifact_id,
            artifact.content_hash,
            worker_result.preview,
        )
        .and_then(|_| session.start_review())
        .and_then(|_| session.complete_after_review())
        .map_err(|error| format!("{error:?}"))?;

    let result = RecordedResearchLoopResult {
        final_state: session.state(),
        event_count: session.event_count(),
        manifest_hash,
        artifact_count,
        event_jsonl: session.export_events_jsonl(),
    };
    if config.cleanup {
        let _ = fs::remove_dir_all(&root);
    }
    Ok(result)
}

fn extract_json_string(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker)? + marker.len();
    let rest = &input[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_json_usize(input: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = &input[start..];
    let end = rest
        .find(|ch: char| !ch.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_research_loop_runs_csv_profile_tool() {
        let result =
            run_recorded_research_loop_fixture(&RecordedResearchLoopConfig::default()).unwrap();
        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(result.artifact_count, 5);
        assert!(result.manifest_hash.starts_with("fnv64_"));
        assert!(result
            .event_jsonl
            .contains("\"tool_id\":\"research.csv_profile\""));
        assert!(result.event_jsonl.contains("\"provider\":\"qwen\""));
        assert!(!result.event_jsonl.contains("sk-"));
        assert!(!result.event_jsonl.contains(".env"));
    }
}
