//! Injectable live HTTP transport boundary.
//!
//! The runtime owns preflight, event recording, sanitization, and native
//! DeepSeek/Qwen response normalization. The actual socket implementation is a
//! replaceable boundary so tests can prove live-call orchestration without
//! network access or key material.

use crate::artifact::ArtifactStore;
use crate::live_model_executor::{
    prepare_live_model_execution, record_live_model_response, record_live_model_stream_response,
    LiveModelExecutionRequest, LiveModelExecutionStatus, LiveModelResponseRecordRequest,
    LiveModelStreamRecordRequest,
};
use crate::live_model_request::PreparedModelHttpRequest;
use crate::model_adapter::{ModelRole, PlannedModelCall};
use crate::native_provider::NativeLiveCallGate;
use crate::provider_response_adapter::NativeProviderResponseResult;
use crate::session::AgentSession;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::Actor;
use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHttpResponse {
    pub status_code: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveHttpStreamEvent {
    HttpStatus {
        status_code: u16,
    },
    VisibleTextDelta(String),
    ThinkingDelta {
        chars: usize,
    },
    ContentBlockStarted {
        index: Option<usize>,
        block_type: String,
    },
    ContentBlockFinished {
        index: Option<usize>,
        block_type: String,
    },
    ToolCallStarted {
        index: Option<usize>,
        id: Option<String>,
        name: String,
        input_json: Option<String>,
        requires_finished: bool,
    },
    ToolCallArgumentsDelta {
        index: Option<usize>,
        delta: String,
    },
    ToolCallFinished {
        index: Option<usize>,
    },
}

pub trait LiveHttpTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String>;

    fn send_with_stream_observer(
        &self,
        request: &PreparedModelHttpRequest,
        _observer: &mut dyn FnMut(LiveHttpStreamEvent),
        _interrupt: &AtomicBool,
    ) -> Result<LiveHttpResponse, String> {
        self.send(request)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedLiveHttpTransport {
    pub status_code: u16,
    pub body: String,
}

impl LiveHttpTransport for RecordedLiveHttpTransport {
    fn send(&self, _request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
        Ok(LiveHttpResponse {
            status_code: self.status_code,
            body: self.body.clone(),
        })
    }
}

#[derive(Debug)]
pub struct ScriptedLiveHttpTransport {
    responses: Mutex<VecDeque<LiveHttpResponse>>,
    requests: Mutex<Vec<PreparedModelHttpRequest>>,
}

impl ScriptedLiveHttpTransport {
    pub fn new(responses: Vec<LiveHttpResponse>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn sent_requests(&self) -> Vec<PreparedModelHttpRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl LiveHttpTransport for ScriptedLiveHttpTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
        self.requests
            .lock()
            .map_err(|_| "scripted transport request lock poisoned".to_string())?
            .push(request.clone());
        let mut responses = self
            .responses
            .lock()
            .map_err(|_| "scripted transport lock poisoned".to_string())?;
        responses
            .pop_front()
            .ok_or_else(|| "scripted transport has no remaining responses".to_string())
    }

    fn send_with_stream_observer(
        &self,
        request: &PreparedModelHttpRequest,
        observer: &mut dyn FnMut(LiveHttpStreamEvent),
        interrupt: &AtomicBool,
    ) -> Result<LiveHttpResponse, String> {
        let response = self.send(request)?;
        if request.stream {
            observer(LiveHttpStreamEvent::HttpStatus {
                status_code: response.status_code,
            });
            emit_scripted_stream_events(&response.body, observer, interrupt)?;
        }
        Ok(response)
    }
}

fn emit_scripted_stream_events(
    body: &str,
    observer: &mut dyn FnMut(LiveHttpStreamEvent),
    interrupt: &AtomicBool,
) -> Result<(), String> {
    let mut content_block_types: BTreeMap<usize, String> = BTreeMap::new();
    for line in body.lines() {
        if interrupt.load(Ordering::Relaxed) {
            break;
        }
        if line.contains("\"content_block_start\"") {
            if let Some(index) = extract_json_u64_local(line, "index").map(|value| value as usize) {
                if let Some(block_type) = extract_content_block_type_local(line) {
                    content_block_types.insert(index, block_type.clone());
                    observer(LiveHttpStreamEvent::ContentBlockStarted {
                        index: Some(index),
                        block_type,
                    });
                }
            }
        }
        for delta in crate::native_profile::deepseek::stream::parse_deepseek_sse_line_all(line)? {
            match delta {
                crate::native_profile::deepseek::stream::DeepSeekStreamDelta::Content { delta } => {
                    if !delta.is_empty() {
                        observer(LiveHttpStreamEvent::VisibleTextDelta(delta));
                    }
                }
                crate::native_profile::deepseek::stream::DeepSeekStreamDelta::ToolCall {
                    index,
                    id,
                    name,
                    arguments_fragment,
                } => {
                    if id.is_some() || !name.is_empty() {
                        observer(LiveHttpStreamEvent::ToolCallStarted {
                            index,
                            id,
                            name,
                            input_json: if arguments_fragment.is_empty() {
                                None
                            } else {
                                Some(arguments_fragment)
                            },
                            requires_finished: false,
                        });
                    } else if !arguments_fragment.is_empty() {
                        observer(LiveHttpStreamEvent::ToolCallArgumentsDelta {
                            index,
                            delta: arguments_fragment,
                        });
                    }
                }
                _ => {}
            }
        }
        if line.contains("\"content_block_stop\"") {
            let index = extract_json_u64_local(line, "index").map(|value| value as usize);
            let block_type = index
                .and_then(|value| content_block_types.remove(&value))
                .unwrap_or_else(|| "unknown".to_string());
            observer(LiveHttpStreamEvent::ContentBlockFinished {
                index,
                block_type: block_type.clone(),
            });
            if block_type == "tool_use" {
                observer(LiveHttpStreamEvent::ToolCallFinished { index });
            }
        }
    }
    Ok(())
}

fn extract_content_block_type_local(input: &str) -> Option<String> {
    let marker = "\"content_block\":";
    let start = input.find(marker)? + marker.len();
    let object = input[start..].trim_start();
    if !object.starts_with('{') {
        return None;
    }
    let type_marker = "\"type\":";
    let type_start = object.find(type_marker)? + type_marker.len();
    let rest = object[type_start..].trim_start().strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_json_u64_local(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let digits = input[start..]
        .trim_start()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

#[derive(Debug, Clone)]
pub struct LiveModelHttpRunRequest<'a> {
    pub execution: LiveModelExecutionRequest,
    pub stream_id: &'a str,
    pub role: ModelRole,
    pub plan: &'a PlannedModelCall,
    pub request_preview: &'a str,
    pub transcript_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveModelHttpRunStatus {
    Blocked,
    HttpFailed,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveModelHttpRunResult {
    pub status: LiveModelHttpRunStatus,
    pub gate: Option<NativeLiveCallGate>,
    pub http_status_code: Option<u16>,
    pub http_error_preview: Option<String>,
    pub prepared_request: Option<PreparedModelHttpRequest>,
    pub response: Option<NativeProviderResponseResult>,
}

pub fn run_live_model_http_once<T: LiveHttpTransport>(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    transport: &T,
    request: LiveModelHttpRunRequest<'_>,
) -> Result<LiveModelHttpRunResult, String> {
    run_live_model_http_once_inner(session, artifact_store, transport, request, None)
}

pub fn run_live_model_http_once_with_stream_observer<T: LiveHttpTransport>(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    transport: &T,
    request: LiveModelHttpRunRequest<'_>,
    observer: &mut dyn FnMut(LiveHttpStreamEvent),
) -> Result<LiveModelHttpRunResult, String> {
    run_live_model_http_once_inner(session, artifact_store, transport, request, Some(observer))
}

fn run_live_model_http_once_inner<T: LiveHttpTransport>(
    session: &mut AgentSession,
    artifact_store: &ArtifactStore,
    transport: &T,
    request: LiveModelHttpRunRequest<'_>,
    mut observer: Option<&mut dyn FnMut(LiveHttpStreamEvent)>,
) -> Result<LiveModelHttpRunResult, String> {
    let prepared = prepare_live_model_execution(session, &request.execution)
        .map_err(|error| format!("{error:?}"))?;
    let Some(prepared_request) = prepared.prepared_request.clone() else {
        return Ok(LiveModelHttpRunResult {
            status: LiveModelHttpRunStatus::Blocked,
            gate: prepared.gate,
            http_status_code: None,
            http_error_preview: None,
            prepared_request: None,
            response: None,
        });
    };
    debug_assert_eq!(prepared.status, LiveModelExecutionStatus::Prepared);
    let mut live_visible_delta_count = 0usize;
    let mut attempt = 1u32;
    let mut retry_count = 0u32;
    let http_response = loop {
        let visible_count_before_attempt = live_visible_delta_count;
        let response = if prepared_request.stream {
            match observer.as_deref_mut() {
                Some(observer) => {
                    let mut counting_observer = |event: LiveHttpStreamEvent| {
                        match &event {
                            LiveHttpStreamEvent::VisibleTextDelta(delta) if !delta.is_empty() => {
                                live_visible_delta_count += 1;
                            }
                            _ => {}
                        }
                        observer(event);
                    };
                    transport.send_with_stream_observer(
                        &prepared_request,
                        &mut counting_observer,
                        &AtomicBool::new(false),
                    )?
                }
                None => transport.send(&prepared_request)?,
            }
        } else {
            transport.send(&prepared_request)?
        };
        let retry_allowed_for_stream =
            !prepared_request.stream || live_visible_delta_count == visible_count_before_attempt;
        if !is_retryable_http_status(response.status_code)
            || attempt >= MAX_HTTP_RETRY_ATTEMPTS
            || !retry_allowed_for_stream
        {
            break response;
        }
        let delay_ms = retry_delay_ms(attempt, &request.execution.call_id);
        session
            .record_runtime_event(
                "model.http_retry_scheduled",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"attempt\":{},\"next_attempt\":{},\"status_code\":{},\"delay_ms\":{},\"strategy\":\"transient_http_retry\"}}",
                    json_string(&request.execution.call_id),
                    attempt,
                    attempt + 1,
                    response.status_code,
                    delay_ms
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        retry_count += 1;
        attempt += 1;
    };
    if (200..300).contains(&http_response.status_code) && retry_count > 0 {
        session
            .record_runtime_event(
                "agent.recovery.completed",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"strategy\":\"transient_http_retry\",\"retries\":{},\"final_status_code\":{}}}",
                    json_string(&request.execution.call_id),
                    retry_count,
                    http_response.status_code
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
    }
    if !(200..300).contains(&http_response.status_code) {
        session
            .record_model_call_blocked(
                &request.execution.call_id,
                provider_label(&request.execution.endpoint.family),
                format!("http_status_{}", http_response.status_code),
            )
            .map_err(|error| format!("{error:?}"))?;
        return Ok(LiveModelHttpRunResult {
            status: LiveModelHttpRunStatus::HttpFailed,
            gate: prepared.gate,
            http_status_code: Some(http_response.status_code),
            http_error_preview: Some(sanitize_http_error_preview(&http_response.body)),
            prepared_request: Some(prepared_request),
            response: None,
        });
    }
    let response = if prepared_request.stream {
        record_live_model_stream_response(
            session,
            artifact_store,
            LiveModelStreamRecordRequest {
                call_id: &request.execution.call_id,
                stream_id: request.stream_id,
                endpoint: &request.execution.endpoint,
                role: request.role,
                plan: request.plan,
                request_preview: request.request_preview,
                transcript_id: request.transcript_id,
                response_sse_body: &http_response.body,
                record_content_deltas: live_visible_delta_count == 0,
            },
        )?
    } else {
        record_live_model_response(
            session,
            artifact_store,
            LiveModelResponseRecordRequest {
                call_id: &request.execution.call_id,
                stream_id: request.stream_id,
                endpoint: &request.execution.endpoint,
                role: request.role,
                plan: request.plan,
                request_preview: request.request_preview,
                transcript_id: request.transcript_id,
                response_body: &http_response.body,
            },
        )?
    };
    Ok(LiveModelHttpRunResult {
        status: LiveModelHttpRunStatus::Completed,
        gate: prepared.gate,
        http_status_code: Some(http_response.status_code),
        http_error_preview: None,
        prepared_request: Some(prepared_request),
        response: Some(response),
    })
}

fn sanitize_http_error_preview(value: &str) -> String {
    value
        .replace('\n', " ")
        .replace('\r', " ")
        .split_whitespace()
        .take(80)
        .collect::<Vec<_>>()
        .join(" ")
}

const MAX_HTTP_RETRY_ATTEMPTS: u32 = 6;

fn is_retryable_http_status(status_code: u16) -> bool {
    matches!(status_code, 408 | 409 | 429 | 500 | 502 | 503 | 504)
}

fn retry_delay_ms(attempt: u32, call_id: &str) -> u64 {
    let base = 100u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(5));
    let jitter = call_id.bytes().fold(attempt as u64, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u64)
    }) % 37;
    base + jitter
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => escaped.push_str(&format!("\\u{:04x}", other as u32)),
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

fn provider_label(family: &NativeModelFamily) -> &'static str {
    match family {
        NativeModelFamily::DeepSeek => "deepseek",
        NativeModelFamily::Qwen => "qwen",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::ArtifactStore;
    use crate::live_model_request::ModelRequestMessage;
    use crate::model_adapter::{
        DeepSeekNativeAdapter, ModelAdapter, ModelAdapterRequest, QwenNativeAdapter,
    };
    use crate::native_provider::NativeProviderEndpoint;
    use crate::state::AgentState;
    use researchcode_kernel::model::{NativeModelFamily, NativeModelProfile, OptimizationLevel};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recorded_transport_runs_deepseek_without_duplicate_call_started() {
        let (root, store) = temp_store("deepseek-live-http");
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "live http".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let mut session = ready_session();
        let result = run_live_model_http_once(
            &mut session,
            &store,
            &RecordedLiveHttpTransport {
                status_code: 200,
                body: r#"{"model":"deepseek-v4-flash","content":[{"type":"text","text":"Visible"}],"reasoning_content":"Need sk-testsecret from .env","usage":{"input_tokens":10,"output_tokens":5,"reasoning_tokens":3,"cache_read_input_tokens":7,"cache_creation_input_tokens":3}}"#.to_string(),
            },
            LiveModelHttpRunRequest {
                execution: LiveModelExecutionRequest {
                    call_id: "call_1".to_string(),
                    role: "planner".to_string(),
                    endpoint,
                    messages: vec![ModelRequestMessage {
                        role: "user".to_string(),
                        content: "Plan".to_string(),
                        cache_control_ttl: None,
                    }],
                    max_tokens: 1024,
                    stream: false,
                    tools_json: None,
                    live_calls_enabled: true,
                    network_approved: true,
                },
                stream_id: "stream_1",
                role: ModelRole::Planner,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_1",
            },
        )
        .unwrap();
        assert_eq!(result.status, LiveModelHttpRunStatus::Completed);
        let jsonl = session.export_events_jsonl();
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            1
        );
        assert!(jsonl.contains("\"event_type\":\"model.call_completed\""));
        assert!(!jsonl.contains("sk-testsecret"));
        assert!(!jsonl.contains(".env"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorded_transport_runs_qwen_and_rejects_http_failure() {
        let (root, store) = temp_store("qwen-live-http");
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let adapter = QwenNativeAdapter::new(
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
                task_summary: "live http".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let mut session = ready_session();
        let result = run_live_model_http_once(
            &mut session,
            &store,
            &RecordedLiveHttpTransport {
                status_code: 200,
                body: r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"reasoning_content":"Need sk-testsecret from .env","content":"Patch"}}],"usage":{"prompt_tokens":10,"completion_tokens":5,"reasoning_tokens":3}}"#.to_string(),
            },
            LiveModelHttpRunRequest {
                execution: qwen_execution("call_1", endpoint.clone()),
                stream_id: "stream_1",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_1",
            },
        )
        .unwrap();
        assert_eq!(result.status, LiveModelHttpRunStatus::Completed);

        let failed = run_live_model_http_once(
            &mut session,
            &store,
            &RecordedLiveHttpTransport {
                status_code: 503,
                body: "unavailable".to_string(),
            },
            LiveModelHttpRunRequest {
                execution: qwen_execution("call_2", endpoint),
                stream_id: "stream_2",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_2",
            },
        )
        .unwrap();
        assert_eq!(failed.status, LiveModelHttpRunStatus::HttpFailed);
        assert_eq!(failed.http_error_preview.as_deref(), Some("unavailable"));
        assert!(session.export_events_jsonl().contains("http_status_503"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scripted_transport_retries_transient_http_failure_and_records_recovery() {
        let (root, store) = temp_store("qwen-live-http-retry");
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        let adapter = QwenNativeAdapter::new(
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
                task_summary: "live http retry".to_string(),
                requires_tools: false,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 429,
                body: "rate limited".to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"{"model":"Qwen/Qwen3.6-27B","choices":[{"message":{"content":"Recovered"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#.to_string(),
            },
        ]);
        let mut session = ready_session();
        let result = run_live_model_http_once(
            &mut session,
            &store,
            &transport,
            LiveModelHttpRunRequest {
                execution: qwen_execution("call_retry", endpoint),
                stream_id: "stream_retry",
                role: ModelRole::Executor,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_retry",
            },
        )
        .unwrap();

        assert_eq!(result.status, LiveModelHttpRunStatus::Completed);
        assert_eq!(result.http_status_code, Some(200));
        let jsonl = session.export_events_jsonl();
        assert!(jsonl.contains("\"event_type\":\"model.http_retry_scheduled\""));
        assert!(jsonl.contains("\"event_type\":\"agent.recovery.completed\""));
        assert!(jsonl.contains("\"status_code\":429"));
        assert!(jsonl.contains("\"retries\":1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recorded_transport_routes_stream_body_through_native_stream_adapter() {
        let (root, store) = temp_store("deepseek-live-http-stream");
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let adapter = DeepSeekNativeAdapter::new(
            NativeModelProfile {
                profile_id: "deepseek-v4-native".to_string(),
                family: NativeModelFamily::DeepSeek,
                optimization_level: OptimizationLevel::Native,
            },
            "deepseek-v4-flash",
        )
        .unwrap();
        let plan = adapter
            .plan_call(&ModelAdapterRequest {
                role: ModelRole::Planner,
                task_summary: "live http stream".to_string(),
                requires_tools: true,
                context_tokens_estimate: 2_000,
            })
            .unwrap();
        let mut session = ready_session();
        let result = run_live_model_http_once(
            &mut session,
            &store,
            &RecordedLiveHttpTransport {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"reasoning_content":"Need sk-testsecret from .env"}}]}
data: {"choices":[{"delta":{"tool_calls":[{"function":{"name":"file.read","arguments":"{\"path\":\"src/parser.ts\"}"}}]}}]}
data: {"usage":{"prompt_tokens":10,"completion_tokens":5,"reasoning_tokens":3,"prompt_cache_hit_tokens":7,"prompt_cache_miss_tokens":3}}
data: [DONE]"#
                    .to_string(),
            },
            LiveModelHttpRunRequest {
                execution: LiveModelExecutionRequest {
                    call_id: "call_stream_1".to_string(),
                    role: "planner".to_string(),
                    endpoint,
                    messages: vec![ModelRequestMessage {
                        role: "user".to_string(),
                        content: "Plan".to_string(),
                        cache_control_ttl: None,
                    }],
                    max_tokens: 1024,
                    stream: true,
                    tools_json: None,
                    live_calls_enabled: true,
                    network_approved: true,
                },
                stream_id: "stream_1",
                role: ModelRole::Planner,
                plan: &plan,
                request_preview: "request",
                transcript_id: "transcript_stream_1",
            },
        )
        .unwrap();
        assert_eq!(result.status, LiveModelHttpRunStatus::Completed);
        let response = result.response.unwrap();
        assert!(response.visible_content_preview.contains("\"tool_calls\""));
        assert_eq!(response.reasoning_tokens, 3);
        assert_eq!(response.prompt_cache_hit_tokens, 7);
        let jsonl = session.export_events_jsonl();
        assert_eq!(
            jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            1
        );
        assert!(jsonl.contains("\"delta_kind\":\"reasoning_sanitized\""));
        assert!(!jsonl.contains("sk-testsecret"));
        assert!(!jsonl.contains(".env"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn scripted_transport_returns_responses_in_order() {
        let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: "first".to_string(),
            },
            LiveHttpResponse {
                status_code: 201,
                body: "second".to_string(),
            },
        ]);
        let request = PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "http://127.0.0.1".to_string(),
            authorization_env: "PATH".to_string(),
            body_json: "{}".to_string(),
            stream: false,
        };
        assert_eq!(transport.send(&request).unwrap().body, "first");
        assert_eq!(transport.send(&request).unwrap().status_code, 201);
        assert!(transport.send(&request).is_err());
    }

    fn qwen_execution(
        call_id: &str,
        endpoint: NativeProviderEndpoint,
    ) -> LiveModelExecutionRequest {
        LiveModelExecutionRequest {
            call_id: call_id.to_string(),
            role: "executor".to_string(),
            endpoint,
            messages: vec![ModelRequestMessage {
                role: "user".to_string(),
                content: "Patch".to_string(),
                cache_control_ttl: None,
            }],
            max_tokens: 1024,
            stream: false,
            tools_json: None,
            live_calls_enabled: true,
            network_approved: true,
        }
    }

    fn ready_session() -> AgentSession {
        let mut session = AgentSession::new("proj", "sess", "task").unwrap();
        session.transition_to(AgentState::Planning).unwrap();
        session
            .transition_to(AgentState::RetrievingContext)
            .unwrap();
        session
    }

    fn temp_store(name: &str) -> (std::path::PathBuf, ArtifactStore) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-{name}-{nonce}"));
        let store = ArtifactStore::new(root.join("artifacts"));
        (root, store)
    }
}
