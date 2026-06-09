//! Python sidecar implementation of the live HTTP transport boundary.
//!
//! This adapter keeps real socket I/O outside the Product Kernel. The Rust
//! runtime still performs native DeepSeek/Qwen preflight, request construction,
//! event recording, stream parsing, and transcript sanitization. The sidecar
//! receives a prepared request, reads the API key only from the named
//! environment variable, and writes the raw provider body to a temporary file.

use crate::live_http_transport::{LiveHttpResponse, LiveHttpStreamEvent, LiveHttpTransport};
use crate::live_model_request::PreparedModelHttpRequest;
use crate::secret_scan::scan_text_for_secrets;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonSidecarLiveHttpTransport {
    pub python_bin: PathBuf,
    pub script_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSidecarHealthStatus {
    Skipped,
    Healthy,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSidecarHealthReport {
    pub status: ProviderSidecarHealthStatus,
    pub reason: Option<String>,
    pub http_status_code: Option<u16>,
    pub target_kind: Option<String>,
}

impl PythonSidecarLiveHttpTransport {
    pub fn new(python_bin: impl Into<PathBuf>, script_path: impl Into<PathBuf>) -> Self {
        Self {
            python_bin: python_bin.into(),
            script_path: script_path.into(),
        }
    }

    pub fn default_workspace_sidecar() -> Self {
        Self::new("python3", workspace_script_path())
    }

    pub fn health_check(
        &self,
        request: &PreparedModelHttpRequest,
    ) -> Result<ProviderSidecarHealthReport, String> {
        if !scan_text_for_secrets(&request.authorization_env).is_empty() {
            return Err("sidecar_rejected_secret_like_authorization_env".to_string());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-provider-health-{nonce}"));
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let response_body_path = root.join("unused_response_body.txt");
        let input = sidecar_input_json_with_mode(request, &response_body_path, "health_check");
        let output = run_sidecar_process(self, &input)?;
        let _ = fs::remove_dir_all(&root);
        if output.contains("\"skipped\": true") || output.contains("\"skipped\":true") {
            return Ok(ProviderSidecarHealthReport {
                status: ProviderSidecarHealthStatus::Skipped,
                reason: parse_json_string_field(&output, "reason")
                    .or_else(|| Some("unknown".to_string())),
                http_status_code: None,
                target_kind: None,
            });
        }
        let health_status = parse_json_string_field(&output, "health_status").ok_or_else(|| {
            format!(
                "sidecar_missing_health_status: {}",
                sanitize_sidecar_output(&output)
            )
        })?;
        let status = match health_status.as_str() {
            "healthy" => ProviderSidecarHealthStatus::Healthy,
            "unhealthy" => ProviderSidecarHealthStatus::Unhealthy,
            "skipped" => ProviderSidecarHealthStatus::Skipped,
            other => return Err(format!("sidecar_unknown_health_status: {other}")),
        };
        Ok(ProviderSidecarHealthReport {
            status,
            reason: None,
            http_status_code: parse_json_u16_field(&output, "status_code"),
            target_kind: parse_json_string_field(&output, "target_kind"),
        })
    }
}

impl LiveHttpTransport for PythonSidecarLiveHttpTransport {
    fn send(&self, request: &PreparedModelHttpRequest) -> Result<LiveHttpResponse, String> {
        if !scan_text_for_secrets(&request.authorization_env).is_empty() {
            return Err("sidecar_rejected_secret_like_authorization_env".to_string());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root = std::env::temp_dir().join(format!("researchcode-provider-sidecar-{nonce}"));
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let response_body_path = root.join("response_body.txt");
        let input = sidecar_input_json(request, &response_body_path);
        let stdout = match run_sidecar_process(self, &input) {
            Ok(stdout) => stdout,
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        };
        if stdout.contains("\"skipped\": true") || stdout.contains("\"skipped\":true") {
            let reason =
                parse_json_string_field(&stdout, "reason").unwrap_or_else(|| "unknown".to_string());
            let _ = fs::remove_dir_all(&root);
            return Err(format!("sidecar_skipped: {reason}"));
        }
        let status_code = parse_json_u16_field(&stdout, "status_code").ok_or_else(|| {
            format!(
                "sidecar_missing_status: {}",
                sanitize_sidecar_output(&stdout)
            )
        })?;
        let body = fs::read_to_string(&response_body_path)
            .map_err(|error| format!("sidecar_response_read_failed: {error}"))?;
        let _ = fs::remove_dir_all(&root);
        Ok(LiveHttpResponse { status_code, body })
    }

    fn send_with_stream_observer(
        &self,
        request: &PreparedModelHttpRequest,
        observer: &mut dyn FnMut(LiveHttpStreamEvent),
        interrupt: &AtomicBool,
    ) -> Result<LiveHttpResponse, String> {
        if !request.stream {
            return self.send(request);
        }
        if !scan_text_for_secrets(&request.authorization_env).is_empty() {
            return Err("sidecar_rejected_secret_like_authorization_env".to_string());
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("researchcode-provider-stream-sidecar-{nonce}"));
        fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        let response_body_path = root.join("response_body.txt");
        let input =
            sidecar_input_json_with_mode(request, &response_body_path, "stream_visible_text");
        let output = match run_sidecar_streaming_process(self, &input, observer, interrupt) {
            Ok(output) => output,
            Err(error) => {
                let _ = fs::remove_dir_all(&root);
                return Err(error);
            }
        };
        if let Some(reason) = output.skipped_reason {
            let _ = fs::remove_dir_all(&root);
            return Err(format!("sidecar_skipped: {reason}"));
        }
        let status_code = output.status_code.ok_or_else(|| {
            format!(
                "sidecar_missing_status: {}",
                sanitize_sidecar_output(&output.stdout_tail)
            )
        })?;
        let body = fs::read_to_string(&response_body_path).unwrap_or_else(|_| {
            output
                .http_error_preview
                .as_deref()
                .unwrap_or("")
                .to_string()
        });
        let _ = fs::remove_dir_all(&root);
        Ok(LiveHttpResponse { status_code, body })
    }
}

fn workspace_script_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("scripts")
        .join("provider_http_sidecar.py")
}

fn sidecar_input_json(request: &PreparedModelHttpRequest, response_body_path: &PathBuf) -> String {
    sidecar_input_json_with_mode(request, response_body_path, "request")
}

fn sidecar_input_json_with_mode(
    request: &PreparedModelHttpRequest,
    response_body_path: &PathBuf,
    mode: &str,
) -> String {
    format!(
        "{{\"mode\":\"{}\",\"method\":\"{}\",\"url\":\"{}\",\"authorization_env\":\"{}\",\"body_json\":\"{}\",\"stream\":{},\"response_body_path\":\"{}\"}}",
        escape_json(mode),
        escape_json(&request.method),
        escape_json(&request.url),
        escape_json(&request.authorization_env),
        escape_json(&request.body_json),
        request.stream,
        escape_json(&response_body_path.display().to_string())
    )
}

fn run_sidecar_process(
    transport: &PythonSidecarLiveHttpTransport,
    input: &str,
) -> Result<String, String> {
    let mut child = Command::new(&transport.python_bin)
        .arg(&transport.script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("sidecar_spawn_failed: {error}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "sidecar_stdin_unavailable".to_string())?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("sidecar_stdin_write_failed: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("sidecar_wait_failed: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(format!(
            "sidecar_failed: {}",
            sanitize_sidecar_output(&stderr)
        ));
    }
    Ok(stdout)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct StreamingSidecarOutput {
    status_code: Option<u16>,
    skipped_reason: Option<String>,
    transport_error_preview: Option<String>,
    http_error_preview: Option<String>,
    stdout_tail: String,
}

fn run_sidecar_streaming_process(
    transport: &PythonSidecarLiveHttpTransport,
    input: &str,
    observer: &mut dyn FnMut(LiveHttpStreamEvent),
    interrupt: &AtomicBool,
) -> Result<StreamingSidecarOutput, String> {
    let mut child = Command::new(&transport.python_bin)
        .arg(&transport.script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("sidecar_spawn_failed: {error}"))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| "sidecar_stdin_unavailable".to_string())?;
        stdin
            .write_all(input.as_bytes())
            .map_err(|error| format!("sidecar_stdin_write_failed: {error}"))?;
    }
    drop(child.stdin.take());
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sidecar_stdout_unavailable".to_string())?;
    let (line_sender, line_receiver) = mpsc::channel::<Result<String, String>>();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line_result in reader.lines() {
            let line = line_result.map_err(|error| format!("sidecar_stdout_read_failed: {error}"));
            if line_sender.send(line).is_err() {
                return;
            }
        }
    });
    let mut output = StreamingSidecarOutput::default();
    let idle_timeout = provider_stream_idle_timeout();
    let total_timeout = provider_stream_total_timeout();
    let stream_started_at = Instant::now();
    let mut last_line_at = Instant::now();
    let mut interrupted = false;
    loop {
        if interrupt.load(Ordering::Relaxed) {
            interrupted = true;
            let _ = child.kill();
            break;
        }
        if let Some(timeout) =
            total_timeout.filter(|timeout| stream_started_at.elapsed() >= *timeout)
        {
            let _ = child.kill();
            return Err(format!(
                "sidecar_stream_total_timeout_after_{}s",
                timeout.as_secs()
            ));
        }
        let line = match line_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(line_result) => line_result?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(_status)) = child.try_wait() {
                    break;
                }
                if last_line_at.elapsed() >= idle_timeout {
                    let _ = child.kill();
                    return Err(format!(
                        "sidecar_stream_idle_timeout_after_{}s",
                        idle_timeout.as_secs()
                    ));
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        last_line_at = Instant::now();
        output.stdout_tail.push_str(&line);
        output.stdout_tail.push('\n');
        if output.stdout_tail.chars().count() > 4096 {
            output.stdout_tail = output
                .stdout_tail
                .chars()
                .rev()
                .take(4096)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
        }
        match parse_json_string_field(&line, "event").as_deref() {
            Some("http_status") => {
                output.status_code = parse_json_u16_field(&line, "status_code");
                if let Some(status_code) = output.status_code {
                    observer(LiveHttpStreamEvent::HttpStatus { status_code });
                }
            }
            Some("http_error") => {
                output.status_code = parse_json_u16_field(&line, "status_code");
                if let Some(status_code) = output.status_code {
                    observer(LiveHttpStreamEvent::HttpStatus { status_code });
                }
                output.http_error_preview = parse_json_string_field(&line, "preview");
            }
            Some("transport_error") => {
                output.transport_error_preview = parse_json_string_field(&line, "preview")
                    .or_else(|| Some("unknown".to_string()));
            }
            Some("skipped") => {
                output.skipped_reason = parse_json_string_field(&line, "reason")
                    .or_else(|| Some("unknown".to_string()));
            }
            Some("text") => {
                if let Some(delta) = parse_json_string_field(&line, "delta") {
                    if !delta.is_empty() {
                        observer(LiveHttpStreamEvent::VisibleTextDelta(delta));
                    }
                }
            }
            Some("reasoning_sanitized") => {
                let chars = parse_json_usize_field(&line, "chars").unwrap_or(0);
                if chars > 0 {
                    observer(LiveHttpStreamEvent::ThinkingDelta { chars });
                }
            }
            Some("content_block_start") => {
                observer(LiveHttpStreamEvent::ContentBlockStarted {
                    index: parse_json_usize_field(&line, "index"),
                    block_type: parse_json_string_field(&line, "block_type")
                        .unwrap_or_else(|| "unknown".to_string()),
                });
            }
            Some("content_block_stop") => {
                let index = parse_json_usize_field(&line, "index");
                let block_type = parse_json_string_field(&line, "block_type")
                    .unwrap_or_else(|| "unknown".to_string());
                observer(LiveHttpStreamEvent::ContentBlockFinished {
                    index,
                    block_type: block_type.clone(),
                });
                if block_type == "tool_use" {
                    observer(LiveHttpStreamEvent::ToolCallFinished { index });
                }
            }
            Some("tool_call") => {
                observer(LiveHttpStreamEvent::ToolCallStarted {
                    index: None,
                    id: parse_json_string_field(&line, "id"),
                    name: parse_json_string_field(&line, "name").unwrap_or_default(),
                    input_json: None,
                    requires_finished: false,
                });
            }
            Some("tool_arguments_delta") => {
                if let Some(delta_hex) = parse_json_string_field(&line, "delta_hex") {
                    if let Some(delta) = decode_hex_string(&delta_hex) {
                        observer(LiveHttpStreamEvent::ToolCallArgumentsDelta {
                            index: None,
                            delta,
                        });
                    }
                }
            }
            Some("tool_call_stop") => {
                observer(LiveHttpStreamEvent::ToolCallFinished {
                    index: parse_json_usize_field(&line, "index"),
                });
            }
            _ => {}
        }
    }
    let status = child
        .wait()
        .map_err(|error| format!("sidecar_wait_failed: {error}"))?;
    if interrupted {
        return Err("sidecar_interrupted".to_string());
    }
    if !status.success() {
        return Err(format!("sidecar_failed: {status}"));
    }
    if let Some(error) = output.transport_error_preview {
        return Err(format!("sidecar_transport_error: {error}"));
    }
    Ok(output)
}

fn provider_stream_idle_timeout() -> Duration {
    let seconds = std::env::var("RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.trim().parse::<f64>().ok())
        .or_else(|| {
            std::env::var("RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS")
                .ok()
                .and_then(|value| value.trim().parse::<f64>().ok())
        })
        .unwrap_or(60.0)
        .clamp(1.0, 600.0);
    Duration::from_secs_f64(seconds)
}

fn provider_stream_total_timeout() -> Option<Duration> {
    let value = std::env::var("RESEARCHCODE_PROVIDER_STREAM_TOTAL_TIMEOUT_SECONDS")
        .ok()
        .or_else(|| std::env::var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS").ok())?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "0" | "none" | "off" | "false"
        )
    {
        return None;
    }
    let seconds = trimmed.parse::<f64>().ok()?.clamp(10.0, 7200.0);
    Some(Duration::from_secs_f64(seconds))
}

fn decode_hex_string(value: &str) -> Option<String> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for chunk_start in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[chunk_start..chunk_start + 2], 16).ok()?;
        bytes.push(byte);
    }
    String::from_utf8(bytes).ok()
}

fn escape_json(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn parse_json_u16_field(input: &str, key: &str) -> Option<u16> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    let digits = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn parse_json_usize_field(input: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    let digits = tail
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn parse_json_string_field(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let tail = input[start..].trim_start();
    if !tail.starts_with('"') {
        return None;
    }
    let mut result = String::new();
    let mut escaped = false;
    for character in tail[1..].chars() {
        if escaped {
            result.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            return Some(result);
        } else {
            result.push(character);
        }
    }
    None
}

fn sanitize_sidecar_output(value: &str) -> String {
    value
        .split_whitespace()
        .take(32)
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_is_disabled_by_default() {
        let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
        let result = transport.send(&PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com/anthropic".to_string(),
            authorization_env: "RESEARCHCODE_TEST_MISSING_API_KEY".to_string(),
            body_json: "{\"model\":\"deepseek-v4-flash\",\"messages\":[],\"stream\":false}"
                .to_string(),
            stream: false,
        });
        assert!(matches!(result, Err(error) if error.contains("network_not_enabled")));
    }

    #[test]
    fn sidecar_rejects_secret_like_authorization_env_before_spawn() {
        let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
        let result = transport.send(&PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com/anthropic".to_string(),
            authorization_env: "sk-testsecret".to_string(),
            body_json: "{\"model\":\"deepseek-v4-flash\",\"messages\":[],\"stream\":false}"
                .to_string(),
            stream: false,
        });
        assert!(matches!(result, Err(error) if error.contains("authorization_env")));
    }

    #[test]
    fn parses_sidecar_status_and_reason_fields() {
        assert_eq!(
            parse_json_u16_field(r#"{"ok":true,"status_code":200}"#, "status_code"),
            Some(200)
        );
        assert_eq!(
            parse_json_string_field(r#"{"reason":"network_not_enabled"}"#, "reason"),
            Some("network_not_enabled".to_string())
        );
    }

    #[test]
    fn stream_total_timeout_defaults_to_uncapped() {
        std::env::remove_var("RESEARCHCODE_PROVIDER_STREAM_TOTAL_TIMEOUT_SECONDS");
        std::env::remove_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS");
        assert_eq!(provider_stream_total_timeout(), None);

        std::env::set_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS", "0");
        assert_eq!(provider_stream_total_timeout(), None);

        std::env::set_var("RESEARCHCODE_PROVIDER_STREAM_TOTAL_TIMEOUT_SECONDS", "120");
        assert_eq!(
            provider_stream_total_timeout(),
            Some(Duration::from_secs(120))
        );

        std::env::remove_var("RESEARCHCODE_PROVIDER_STREAM_TOTAL_TIMEOUT_SECONDS");
        std::env::remove_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS");
    }

    #[test]
    fn provider_health_check_is_disabled_by_default() {
        let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
        let result = transport
            .health_check(&PreparedModelHttpRequest {
                method: "POST".to_string(),
                url: "https://api.deepseek.com/anthropic".to_string(),
                authorization_env: "RESEARCHCODE_TEST_MISSING_API_KEY".to_string(),
                body_json: "{\"model\":\"deepseek-v4-flash\",\"messages\":[],\"stream\":false}"
                    .to_string(),
                stream: false,
            })
            .unwrap();
        assert_eq!(result.status, ProviderSidecarHealthStatus::Skipped);
        assert_eq!(result.reason, Some("network_not_enabled".to_string()));
    }

    #[test]
    fn provider_health_check_rejects_secret_like_authorization_env() {
        let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
        let result = transport.health_check(&PreparedModelHttpRequest {
            method: "POST".to_string(),
            url: "https://api.deepseek.com/anthropic".to_string(),
            authorization_env: "sk-testsecret".to_string(),
            body_json: "{\"model\":\"deepseek-v4-flash\",\"messages\":[],\"stream\":false}"
                .to_string(),
            stream: false,
        });
        assert!(matches!(result, Err(error) if error.contains("authorization_env")));
    }
}
