//! Sync HTTP/1.1 server for GUI/runtime integration.
//!
//! Replaces the Python `local_api_server.py` mock layer with real RuntimeFacade
//! calls. Uses only `std::net::TcpListener` + `std::thread` — no async deps.

use crate::native_provider::NativeProviderEndpoint;
use crate::runtime_facade::{
    AutonomyMode, RuntimeFacade, RuntimeModelMode, RuntimePermissionDecisionOutcome,
    RuntimeSessionSnapshot,
};
use crate::sidecar_http_transport::PythonSidecarLiveHttpTransport;
use crate::state::AgentState;
use crate::tool_execution::ToolExecutionArgs;
use researchcode_kernel::{PermissionDecisionKind, PlanApprovalDecisionKind};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

const DEFAULT_CONTINUE_PROMPT: &str = "Continue the current session using prior context.";
const APPROVED_PLAN_CONTINUE_PROMPT: &str =
    "The plan was approved. Continue implementing the approved plan using existing evidence. Do not call plan.enter again unless the user asks for a new plan.";

fn continuation_prompt_from_last_user_task(
    preamble: Option<&str>,
    last_user_prompt: Option<&str>,
) -> String {
    let last_user_prompt = last_user_prompt
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty());
    match (
        preamble.map(str::trim).filter(|prompt| !prompt.is_empty()),
        last_user_prompt,
    ) {
        (Some(preamble), Some(original)) if preamble != original => {
            format!("{preamble}\n\nOriginal user request to continue:\n{original}")
        }
        (Some(preamble), _) => preamble.to_string(),
        (None, Some(original)) => original.to_string(),
        (None, None) => DEFAULT_CONTINUE_PROMPT.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct LocalApiServerConfig {
    pub host: String,
    pub port: u16,
    pub static_root: PathBuf,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
}

impl Default for LocalApiServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 8765,
            static_root: PathBuf::from("apps/desktop/dist"),
            workspace_root: PathBuf::from("."),
            artifact_root: PathBuf::from("artifacts"),
        }
    }
}

pub struct LocalApiServer {
    config: LocalApiServerConfig,
    running: Arc<AtomicBool>,
}

#[derive(Debug)]
struct LocalApiRuntimeState {
    facade: Arc<RuntimeFacade>,
    active_turns: Mutex<HashMap<String, u64>>,
    next_turn_generation: AtomicU64,
    last_prompts: Mutex<HashMap<String, String>>,
}

impl LocalApiRuntimeState {
    fn new(workspace_root: PathBuf, artifact_root: PathBuf) -> Self {
        Self {
            facade: Arc::new(RuntimeFacade::new(workspace_root, artifact_root)),
            active_turns: Mutex::new(HashMap::new()),
            next_turn_generation: AtomicU64::new(1),
            last_prompts: Mutex::new(HashMap::new()),
        }
    }

    fn release_turn(&self, session_id: &str) {
        self.active_turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(session_id);
    }

    fn release_turn_generation(&self, session_id: &str, generation: u64) {
        let mut active_turns = self
            .active_turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_turns.get(session_id).copied() == Some(generation) {
            active_turns.remove(session_id);
        }
    }

    fn acquire_turn_generation(&self, session_id: &str) -> Option<u64> {
        let mut active_turns = self
            .active_turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if active_turns.contains_key(session_id) {
            let is_terminal = self
                .facade
                .get_session_snapshot(session_id)
                .map(|snapshot| is_terminal_agent_state(snapshot.state))
                .unwrap_or(false);
            if is_terminal {
                active_turns.remove(session_id);
            } else {
                return None;
            }
        }
        let generation = self.next_turn_generation.fetch_add(1, Ordering::Relaxed);
        active_turns.insert(session_id.to_string(), generation);
        Some(generation)
    }

    fn clear_active_turn_if_resumable_boundary(&self, session_id: &str) -> Result<bool, String> {
        let observed_generation = self
            .active_turns
            .lock()
            .map_err(|_| "runtime active-turn lock poisoned".to_string())?
            .get(session_id)
            .copied();
        let Some(observed_generation) = observed_generation else {
            return Ok(false);
        };
        let resumable = self
            .facade
            .get_session_snapshot(session_id)
            .map(|snapshot| is_resumable_approval_state(snapshot.state))
            .unwrap_or(false);
        if !resumable {
            return Ok(true);
        }
        let mut active_turns = self
            .active_turns
            .lock()
            .map_err(|_| "runtime active-turn lock poisoned".to_string())?;
        Ok(!clear_matching_active_turn_generation(
            &mut active_turns,
            session_id,
            observed_generation,
        ))
    }
}

fn clear_matching_active_turn_generation(
    active_turns: &mut HashMap<String, u64>,
    session_id: &str,
    observed_generation: u64,
) -> bool {
    if active_turns.get(session_id).copied() == Some(observed_generation) {
        active_turns.remove(session_id);
        true
    } else {
        false
    }
}

impl LocalApiServer {
    pub fn new(config: LocalApiServerConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the server in a background thread. Returns immediately.
    pub fn start(&self) -> Result<u16, String> {
        let addr = format!("{}:{}", self.config.host, self.config.port);
        let listener = TcpListener::bind(&addr).map_err(|e| format!("bind {addr}: {e}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("set_nonblocking: {e}"))?;

        let port = listener
            .local_addr()
            .map_err(|e| format!("local_addr: {e}"))?
            .port();

        let running = Arc::clone(&self.running);
        running.store(true, Ordering::SeqCst);
        let static_root = self.config.static_root.clone();
        let workspace_root = self.config.workspace_root.clone();
        let artifact_root = self.config.artifact_root.clone();

        let runtime_state = Arc::new(LocalApiRuntimeState::new(workspace_root, artifact_root));

        thread::Builder::new()
            .name("local-api-server".to_string())
            .spawn(move || {
                for stream in listener.incoming() {
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                    match stream {
                        Ok(stream) => {
                            let state = Arc::clone(&runtime_state);
                            let root = static_root.clone();
                            thread::spawn(move || handle_connection(stream, state, &root));
                        }
                        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            // No connection ready — sleep briefly before retrying
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("spawn server thread: {e}"))?;

        Ok(port)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Connection handler
// ---------------------------------------------------------------------------

fn handle_connection(mut stream: TcpStream, state: Arc<LocalApiRuntimeState>, static_root: &Path) {
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
    let facade = state.facade.as_ref();

    let req = match parse_request(&mut stream) {
        Ok(r) => r,
        Err(_) => {
            write_text(&mut stream, 400, "Bad Request");
            return;
        }
    };

    // CORS preflight — no auth required
    if req.method == "OPTIONS" {
        write_empty(&mut stream, 204);
        return;
    }

    let path = req.path.clone();
    let method = req.method.clone();

    // Auth check — skip only for GET /health
    if !(method == "GET" && path == "/health") {
        if !is_authorized_header(req.headers.get("authorization")) {
            write_json(
                &mut stream,
                401,
                r#"{"error":"unauthorized","hint":"Set Authorization: Bearer <token> matching RESEARCHCODE_LOCAL_API_TOKEN"}"#,
            );
            return;
        }
    }

    match (method.as_str(), path.as_str()) {
        // -- health -----------------------------------------------------------------
        ("GET", "/health") => {
            write_json(
                &mut stream,
                200,
                r#"{"ok":true,"service":"researchcode-local-api"}"#,
            );
        }

        // -- static file serving ----------------------------------------------------
        ("GET", "/") | ("GET", "/app") => {
            serve_static_file(
                &mut stream,
                static_root,
                "index.html",
                "text/html; charset=utf-8",
            );
        }
        ("GET", "/app.js") => {
            serve_static_file(
                &mut stream,
                static_root,
                "app.js",
                "application/javascript; charset=utf-8",
            );
        }

        // -- GET runtime endpoints ---------------------------------------------------
        ("GET", p) if p == "/runtime/stream-events" || p.starts_with("/runtime/stream-events?") => {
            handle_runtime_stream_events(&mut stream, &state, &req);
        }
        ("GET", p) if p == "/runtime/get-snapshot" || p.starts_with("/runtime/get-snapshot?") => {
            handle_runtime_get_snapshot(&mut stream, facade, &req);
        }
        ("GET", "/runtime/list-tools") => {
            let tools = list_tools_json();
            write_json(&mut stream, 200, &tools);
        }
        ("GET", "/runtime/list-commands") => {
            let cmds = list_commands_json();
            write_json(&mut stream, 200, &cmds);
        }
        ("GET", "/runtime/list-agents") => {
            let agents = list_agents_json();
            write_json(&mut stream, 200, &agents);
        }
        ("GET", "/tool-catalog") => {
            let catalog = tool_catalog_json();
            write_json(&mut stream, 200, &catalog);
        }

        // -- POST runtime endpoints --------------------------------------------------
        ("POST", "/runtime/start-session") => {
            handle_start_session(&mut stream, facade, &req);
        }
        ("POST", "/runtime/resume-session") => {
            handle_resume_session(&mut stream, facade, &req);
        }
        ("POST", "/runtime/submit-user-message") => {
            handle_submit_user_message(&mut stream, Arc::clone(&state), &req);
        }
        ("POST", "/runtime/interrupt-session") => {
            handle_interrupt_session(&mut stream, Arc::clone(&state), &req);
        }
        ("POST", "/runtime/submit-permission-decision") => {
            handle_submit_permission_decision(&mut stream, Arc::clone(&state), &req);
        }
        ("POST", "/runtime/submit-plan-decision") => {
            handle_submit_plan_decision(&mut stream, Arc::clone(&state), &req);
        }
        ("POST", "/runtime/preview-tool") => {
            handle_preview_tool(&mut stream, facade, &req);
        }
        // Legacy route: desktop client (local_api_client.mjs) calls /tool/preview
        // with { tool_id, arguments: {...} } instead of flat ToolExecutionArgs fields.
        ("POST", "/tool/preview") => {
            handle_legacy_tool_preview(&mut stream, facade, &req);
        }
        ("POST", "/runtime/export-events") => {
            handle_export_events(&mut stream, facade, &req);
        }

        // -- legacy native-loop passthrough (stubs for compat) -----------------------
        ("POST", "/native-loop/pending-package")
        | ("POST", "/native-loop/live-pending-package")
        | ("POST", "/native-loop/pending-package-from-session")
        | ("POST", "/native-loop/resume-pending-package") => {
            write_json(
                &mut stream,
                200,
                r#"{"ok":true,"message":"native-loop endpoint: use /runtime/* endpoints instead"}"#,
            );
        }

        _ => {
            write_json(&mut stream, 404, r#"{"error":"not_found"}"#);
        }
    }
}

// ---------------------------------------------------------------------------
// Request / response helpers
// ---------------------------------------------------------------------------

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_request(stream: &mut TcpStream) -> Result<HttpRequest, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .map_err(|e| format!("read request line: {e}"))?;
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return Err("malformed request line".to_string());
    }
    let method = parts[0].to_uppercase();
    let raw_path = parts[1].to_string();

    let mut headers = HashMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("read header: {e}"))?;
        let line = line.trim().to_string();
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }

    const MAX_BODY_SIZE: usize = 10 * 1024 * 1024; // 10 MB
    let content_length: usize = headers
        .get("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_SIZE {
        return Err(format!(
            "request body too large ({content_length} bytes); max {MAX_BODY_SIZE}"
        ));
    }

    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("read body: {e}"))?;
    }

    Ok(HttpRequest {
        method,
        path: raw_path,
        headers,
        body,
    })
}

fn query_param(req: &HttpRequest, key: &str) -> Option<String> {
    let marker = format!("?{key}=");
    if let Some(pos) = req.path.find(&marker) {
        let start = pos + marker.len();
        let rest = &req.path[start..];
        let end = rest.find('&').unwrap_or(rest.len());
        let raw = &rest[..end];
        return Some(url_decode(raw));
    }
    // Also check if key is at start of query string
    if let Some(q) = req.path.find('?') {
        let query = &req.path[q + 1..];
        for part in query.split('&') {
            if let Some((k, v)) = part.split_once('=') {
                if k == key {
                    return Some(url_decode(v));
                }
            }
        }
    }
    None
}

fn url_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

fn body_as_string(req: &HttpRequest) -> String {
    String::from_utf8_lossy(&req.body).to_string()
}

fn parse_json_body(body: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice(body).ok()
}

fn extract_json_string_field(body: &str, field: &str) -> Option<String> {
    // Simple JSON string field extractor — matches "field":"value"
    let pattern = format!("\"{}\":\"", field);
    let start = body.find(&pattern)? + pattern.len();
    let mut result = String::new();
    let mut chars = body[start..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    match n {
                        '"' => result.push('"'),
                        '\\' => result.push('\\'),
                        'n' => result.push('\n'),
                        'r' => result.push('\r'),
                        't' => result.push('\t'),
                        '/' => result.push('/'),
                        other => {
                            result.push('\\');
                            result.push(other);
                        }
                    }
                }
            }
            '"' => break,
            other => result.push(other),
        }
    }
    Some(result)
}

fn extract_json_int_field(body: &str, field: &str) -> Option<i64> {
    let pattern = format!("\"{}\":", field);
    let start = body.find(&pattern)? + pattern.len();
    let rest = body[start..].trim();
    let end = rest
        .find(|c: char| !c.is_ascii_digit() && c != '-')
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Extract a JSON object value for a field (e.g., `"arguments": { ... }`).
/// Returns the raw substring including the braces.
fn extract_json_object_field(body: &str, field: &str) -> Option<String> {
    let pattern = format!("\"{}\":", field);
    let start = body.find(&pattern)? + pattern.len();
    let rest = body[start..].trim();
    if !rest.starts_with('{') {
        return None;
    }
    let mut depth = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' && in_string {
            escaped = true;
            continue;
        }
        if c == '"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return Some(rest[..=i].to_string());
            }
        }
    }
    None
}

fn write_json(stream: &mut TcpStream, status: u16, json: &str) {
    write_response(
        stream,
        status,
        "application/json; charset=utf-8",
        json.as_bytes(),
    );
}

fn write_text(stream: &mut TcpStream, status: u16, text: &str) {
    write_response(stream, status, "text/plain; charset=utf-8", text.as_bytes());
}

fn write_empty(stream: &mut TcpStream, status: u16) {
    write_response(stream, status, "text/plain", &[]);
}

fn write_response(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Unknown",
    };
    let mut resp = format!("HTTP/1.1 {status} {status_text}\r\n");
    resp.push_str("Access-Control-Allow-Origin: *\r\n");
    resp.push_str("Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n");
    resp.push_str("Access-Control-Allow-Headers: Authorization, Content-Type\r\n");
    resp.push_str(&format!("Content-Type: {content_type}\r\n"));
    // This server serves runtime polling over short-lived HTTP/1.1 connections.
    // Do not emit Content-Length: event payloads may be large and concurrent GUI
    // polling has observed browser-side ERR_CONTENT_LENGTH_MISMATCH on partial
    // closes. Connection-close framing is valid here and more robust.
    resp.push_str("Cache-Control: no-cache\r\n");
    resp.push_str("Connection: close\r\n");
    resp.push_str("\r\n");
    let _ = stream.write_all(resp.as_bytes());
    if !body.is_empty() {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Write);
}

fn serve_static_file(stream: &mut TcpStream, root: &Path, filename: &str, content_type: &str) {
    let path = root.join(filename);
    match fs::read(&path) {
        Ok(body) => write_response(stream, 200, content_type, &body),
        Err(_) => write_json(stream, 404, r#"{"error":"not_found"}"#),
    }
}

// ---------------------------------------------------------------------------
// Endpoint handlers
// ---------------------------------------------------------------------------

fn handle_start_session(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let json: Option<serde_json::Value> = parse_json_body(&req.body);
    let model_mode = match json
        .as_ref()
        .and_then(|v| v.get("model_mode"))
        .and_then(|v| v.as_str())
    {
        Some("qwen") => RuntimeModelMode::Qwen,
        _ => RuntimeModelMode::DeepSeek,
    };
    let autonomy_mode = match json
        .as_ref()
        .and_then(|v| v.get("autonomy_mode"))
        .and_then(|v| v.as_str())
    {
        Some("conservative") => AutonomyMode::Conservative,
        Some("manual_review") => AutonomyMode::ManualReview,
        _ => AutonomyMode::FastAuto,
    };
    let workspace = json
        .as_ref()
        .and_then(|v| v.get("workspace"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    match facade.start_session(workspace.map(PathBuf::from), model_mode, autonomy_mode) {
        Ok(handle) => {
            let json = format!(
                "{{\"ok\":true,\"session\":{{\"session_id\":\"{}\",\"task_id\":\"{}\",\"workspace_root\":\"{}\",\"model_mode\":\"{}\",\"autonomy_mode\":\"{}\",\"state\":\"Executing\"}}}}",
                json_escape(&handle.session_id),
                json_escape(&handle.task_id),
                json_escape(&handle.workspace_root.to_string_lossy()),
                handle.model_mode.as_str(),
                handle.autonomy_mode.as_str(),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn handle_resume_session(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };

    // Try to resume from event log path if provided
    if let Some(path) = extract_json_string_field(&body, "event_log_path") {
        match facade.resume_session_from_eventlog(Path::new(&path)) {
            Ok(handle) => {
                let json = format!(
                    "{{\"ok\":true,\"snapshot\":{{\"session_id\":\"{}\",\"state\":\"Executing\"}}}}",
                    json_escape(&handle.session_id)
                );
                write_json(stream, 200, &json);
                return;
            }
            Err(e) => {
                let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
                write_json(stream, 400, &json);
                return;
            }
        }
    }

    // Otherwise just return the current snapshot
    match facade.get_session_snapshot(&session_id) {
        Ok(snapshot) => {
            let json = snapshot_to_json(&snapshot);
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn handle_submit_user_message(
    stream: &mut TcpStream,
    state: Arc<LocalApiRuntimeState>,
    req: &HttpRequest,
) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };
    let text = match extract_json_string_field(&body, "text") {
        Some(t) => t,
        None => {
            write_json(stream, 400, r#"{"ok":false,"error":"text is required"}"#);
            return;
        }
    };

    let turn_generation = match state.acquire_turn_generation(&session_id) {
        Some(generation) => generation,
        None => {
            let json = format!(
                "{{\"ok\":false,\"session_id\":{},\"error_code\":\"runtime_turn_in_progress\"}}",
                json_string(&session_id)
            );
            write_json(stream, 200, &json);
            return;
        }
    };

    match state.facade.submit_user_message(&session_id, &text) {
        Ok(()) => {
            if let Ok(mut prompts) = state.last_prompts.lock() {
                prompts.insert(session_id.clone(), text.clone());
            }
            match spawn_local_api_runtime_turn(
                Arc::clone(&state),
                session_id.clone(),
                text,
                turn_generation,
            ) {
                Ok(_) => {
                    let json = format!(
                        "{{\"ok\":true,\"session_id\":{},\"error_code\":null}}",
                        json_string(&session_id)
                    );
                    write_json(stream, 200, &json);
                }
                Err(e) => {
                    state.release_turn_generation(&session_id, turn_generation);
                    let json = format!(
                        "{{\"ok\":false,\"session_id\":{},\"error_code\":\"runtime_turn_spawn_failed\",\"error\":{}}}",
                        json_string(&session_id),
                        json_string(&e.to_string())
                    );
                    write_json(stream, 500, &json);
                }
            }
        }
        Err(e) => {
            state.release_turn_generation(&session_id, turn_generation);
            let json = format!(
                "{{\"ok\":false,\"session_id\":{},\"error_code\":{}}}",
                json_string(&session_id),
                json_string(&e)
            );
            write_json(stream, 400, &json);
        }
    }
}

fn spawn_local_api_runtime_turn(
    state: Arc<LocalApiRuntimeState>,
    session_id: String,
    text: String,
    generation: u64,
) -> Result<(), String> {
    thread::Builder::new()
        .name("local-api-runtime-turn".to_string())
        .spawn(move || supervise_local_api_runtime_turn(state, session_id, text, generation))
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn supervise_local_api_runtime_turn(
    state: Arc<LocalApiRuntimeState>,
    session_id: String,
    text: String,
    generation: u64,
) {
    let timeout = local_api_turn_timeout();
    let (result_tx, result_rx) = mpsc::channel::<Result<(), String>>();
    let worker_state = Arc::clone(&state);
    let worker_session_id = session_id.clone();
    let worker_text = text;
    match thread::Builder::new()
        .name("local-api-runtime-turn-worker".to_string())
        .spawn(move || {
            let mut sink = |_line: &str| {};
            let result = run_live_runtime_turn_with_sink(
                worker_state.facade.as_ref(),
                &worker_session_id,
                &worker_text,
                &mut sink,
            );
            let _ = result_tx.send(result);
        }) {
        Ok(_) => match recv_runtime_turn_result(&result_rx, timeout) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                if !is_runtime_interrupted_error(&e) {
                    let _ =
                        state
                            .facade
                            .record_runtime_error(&session_id, "runtime_turn_failed", &e);
                }
            }
            Err(LocalApiTurnWaitError::Timeout(timeout)) => {
                state.facade.interrupt();
                let _ = state.facade.record_runtime_error(
                    &session_id,
                    "runtime_turn_timeout",
                    &format!("live runtime turn exceeded {}s", timeout.as_secs()),
                );
            }
            Err(LocalApiTurnWaitError::Disconnected) => {
                let _ = state.facade.record_runtime_error(
                    &session_id,
                    "runtime_turn_worker_disconnected",
                    "live runtime turn worker disconnected before reporting a result",
                );
            }
        },
        Err(e) => {
            let _ = state.facade.record_runtime_error(
                &session_id,
                "runtime_turn_worker_spawn_failed",
                &e.to_string(),
            );
        }
    }
    state.release_turn_generation(&session_id, generation);
}

fn spawn_local_api_continue_from_last_prompt(
    state: Arc<LocalApiRuntimeState>,
    session_id: String,
    prompt_override: Option<String>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("local-api-runtime-continue".to_string())
        .spawn(move || {
            let mut acquired_turn_generation = None;
            for attempt in 0..=100 {
                match state.active_turns.lock() {
                    Ok(mut turns) => {
                        if !turns.contains_key(&session_id) {
                            let generation =
                                state.next_turn_generation.fetch_add(1, Ordering::Relaxed);
                            turns.insert(session_id.clone(), generation);
                            acquired_turn_generation = Some(generation);
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = state.facade.record_runtime_error(
                            &session_id,
                            "runtime_continue_failed",
                            "runtime active-turn lock poisoned",
                        );
                        return;
                    }
                }
                match state.clear_active_turn_if_resumable_boundary(&session_id) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(error) => {
                        let _ = state.facade.record_runtime_error(
                            &session_id,
                            "runtime_continue_failed",
                            &error,
                        );
                        return;
                    }
                }
                if attempt == 100 {
                    let _ = state.facade.record_runtime_error(
                        &session_id,
                        "runtime_continue_failed",
                        "runtime_turn_still_active_after_approval_resume",
                    );
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }

            if let Some(generation) = acquired_turn_generation {
                let last_user_prompt = state
                    .last_prompts
                    .lock()
                    .ok()
                    .and_then(|prompts| prompts.get(&session_id).cloned());
                let prompt = continuation_prompt_from_last_user_task(
                    prompt_override.as_deref(),
                    last_user_prompt.as_deref(),
                );
                supervise_local_api_runtime_turn(state, session_id, prompt, generation);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn is_terminal_agent_state(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::Completed | AgentState::Failed | AgentState::Cancelled
    )
}

fn is_resumable_approval_state(state: AgentState) -> bool {
    matches!(
        state,
        AgentState::WaitingForToolApproval | AgentState::WaitingForPlanApproval
    )
}

fn is_runtime_interrupted_error(error: &str) -> bool {
    error.contains("sidecar_interrupted") || error.contains("interrupted")
}

#[derive(Debug)]
enum LocalApiTurnWaitError {
    Timeout(Duration),
    Disconnected,
}

fn recv_runtime_turn_result(
    result_rx: &mpsc::Receiver<Result<(), String>>,
    timeout: Option<Duration>,
) -> Result<Result<(), String>, LocalApiTurnWaitError> {
    match timeout {
        Some(timeout) => result_rx
            .recv_timeout(timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => LocalApiTurnWaitError::Timeout(timeout),
                mpsc::RecvTimeoutError::Disconnected => LocalApiTurnWaitError::Disconnected,
            }),
        None => result_rx
            .recv()
            .map_err(|_| LocalApiTurnWaitError::Disconnected),
    }
}

fn local_api_turn_timeout() -> Option<Duration> {
    let value = env::var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS").ok()?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || matches!(
            trimmed.to_ascii_lowercase().as_str(),
            "0" | "none" | "off" | "false"
        )
    {
        return None;
    }
    let seconds = trimmed.parse::<f64>().ok()?.clamp(5.0, 7200.0);
    Some(Duration::from_secs_f64(seconds))
}

fn handle_interrupt_session(
    stream: &mut TcpStream,
    state: Arc<LocalApiRuntimeState>,
    req: &HttpRequest,
) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };

    match state.facade.cancel_session(&session_id) {
        Ok(()) => {
            state.release_turn(&session_id);
            let state_label = state
                .facade
                .get_session_snapshot(&session_id)
                .map(|snapshot| agent_state_as_str(snapshot.state).to_string())
                .unwrap_or_else(|_| "Cancelled".to_string());
            let json = format!(
                "{{\"ok\":true,\"session_id\":{},\"state\":{}}}",
                json_string(&session_id),
                json_string(&state_label)
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!(
                "{{\"ok\":false,\"session_id\":{},\"error\":{}}}",
                json_string(&session_id),
                json_string(&e)
            );
            write_json(stream, 400, &json);
        }
    }
}

fn run_live_runtime_turn_with_sink(
    facade: &RuntimeFacade,
    session_id: &str,
    text: &str,
    event_sink: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let snapshot = facade.get_session_snapshot(session_id)?;
    let transport = PythonSidecarLiveHttpTransport::default_workspace_sidecar();
    let max_iterations = env_usize("RESEARCHCODE_LOCAL_API_MAX_ITERATIONS", 0);
    let max_tool_calls = env_usize("RESEARCHCODE_LOCAL_API_MAX_TOOL_CALLS", 0);
    match snapshot.model_mode {
        RuntimeModelMode::DeepSeek => {
            let primary = deepseek_live_endpoint_from_env();
            match facade.run_deepseek_agent_loop_with_transport_and_event_sink(
                &transport,
                session_id,
                text,
                primary.clone(),
                max_iterations,
                max_tool_calls,
                event_sink,
            ) {
                Ok(_) => Ok(()),
                Err(error)
                    if primary.protocol == "anthropic_compatible"
                        && error.contains("http failure")
                        && error.contains("400") =>
                {
                    facade
                        .run_deepseek_agent_loop_with_transport_and_event_sink(
                            &transport,
                            session_id,
                            text,
                            deepseek_openai_fallback_endpoint_from(&primary),
                            max_iterations,
                            max_tool_calls,
                            event_sink,
                        )
                        .map(|_| ())
                }
                Err(error) => Err(error),
            }
        }
        RuntimeModelMode::Qwen => facade
            .run_qwen_agent_loop_with_transport_and_event_sink(
                &transport,
                session_id,
                text,
                qwen_live_endpoint_from_env(),
                max_iterations,
                max_tool_calls,
                event_sink,
            )
            .map(|_| ()),
    }
}

/// GET /runtime/stream-events
///
/// **CONTRACT NOTE (non-SSE)**: This endpoint returns a single JSON document
/// (content-type: application/json), NOT Server-Sent Events (text/event-stream).
/// The desktop GUI (apps/desktop/dist/app.js) expects SSE via `new EventSource(url)`
/// and will fall back to polling on error. This incompatibility is deliberate until
/// SSE support is implemented — the GUI's polling fallback handles it gracefully.
/// Response shape:
///   {"session_id":"…","from_cursor":0,"next_cursor":5,"has_more":false,
///    "events":["…","…"],"jsonl":"…"}
fn handle_runtime_stream_events(
    stream: &mut TcpStream,
    state: &LocalApiRuntimeState,
    req: &HttpRequest,
) {
    let session_id = match query_param(req, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id query param is required"}"#,
            );
            return;
        }
    };
    let cursor: usize = query_param(req, "cursor")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let max_events = query_param(req, "max_events")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100)
        .clamp(1, 200);

    match state
        .facade
        .stream_agent_events_since(&session_id, cursor, Some(max_events))
    {
        Ok(delta) => {
            let events_json = delta.events.iter().fold(String::new(), |mut acc, e| {
                if !acc.is_empty() {
                    acc.push(',');
                }
                acc.push_str(&event_line_to_http_json(e));
                acc
            });
            let json = format!(
                "{{\"session_id\":{},\"from_cursor\":{},\"next_cursor\":{},\"has_more\":{},\"events\":[{events_json}],\"jsonl\":{}}}",
                json_string(&delta.session_id),
                delta.from_cursor,
                delta.next_cursor,
                delta.has_more,
                json_string(&delta.jsonl),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn event_line_to_http_json(line: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
        if let Ok(normalized) = serde_json::to_string(&value) {
            return normalized;
        }
    }
    let raw_preview = line.chars().take(500).collect::<String>();
    format!(
        "{{\"event_id\":\"runtime_http_decode_failed\",\"schema_version\":\"v0\",\"project_id\":\"local\",\"session_id\":null,\"task_id\":null,\"sequence\":0,\"event_type\":\"runtime.event_decode_failed\",\"actor\":\"Runtime\",\"created_at\":\"now\",\"payload\":{{\"error_code\":\"invalid_jsonl_event\",\"raw_preview\":{}}},\"prev_hash\":null,\"hash\":\"runtime_http_decode_failed\"}}",
        json_string(&raw_preview),
    )
}

fn handle_runtime_get_snapshot(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let session_id = match query_param(req, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id query param is required"}"#,
            );
            return;
        }
    };

    match facade.get_session_snapshot(&session_id) {
        Ok(snapshot) => {
            // Return snapshot fields at the root level (not wrapped in {"snapshot":...})
            // so that desktop GUI `snap.events` access (apps/desktop/dist/app.js:82) works.
            let json = snapshot_to_json(&snapshot);
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn handle_submit_permission_decision(
    stream: &mut TcpStream,
    state: Arc<LocalApiRuntimeState>,
    req: &HttpRequest,
) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };
    let permission_id = match extract_json_string_field(&body, "permission_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"permission_id is required"}"#,
            );
            return;
        }
    };
    let decision = match extract_json_string_field(&body, "decision").as_deref() {
        Some("allow_once") => PermissionDecisionKind::AllowOnce,
        Some("allow_session") => PermissionDecisionKind::AllowSession,
        Some("allow_project") | Some("allow_project_rule") => {
            PermissionDecisionKind::AllowProjectRule
        }
        Some("deny") => PermissionDecisionKind::Deny,
        Some("modify") => PermissionDecisionKind::Modify,
        _ => PermissionDecisionKind::Deny,
    };
    let should_continue = should_continue_after_permission_decision(&decision);

    match submit_permission_decision_when_ready(
        state.as_ref(),
        &session_id,
        &permission_id,
        decision,
    ) {
        Ok(outcome) => {
            let mut resume_strategy = "none";
            if outcome.model_continuation_required && should_continue {
                match spawn_local_api_continue_from_last_prompt(
                    Arc::clone(&state),
                    session_id.clone(),
                    None,
                ) {
                    Ok(()) => {
                        resume_strategy = "async_permission_resume";
                    }
                    Err(error) => {
                        let _ = state.facade.record_runtime_error(
                            &session_id,
                            "runtime_continue_failed",
                            &error,
                        );
                        let json = format!(
                            "{{\"ok\":false,\"session_id\":{},\"permission_id\":{},\"error_code\":\"runtime_continue_failed\",\"error\":{},\"outcome\":{}}}",
                            json_string(&outcome.session_id),
                            json_string(&outcome.permission_id),
                            json_string(&error),
                            permission_outcome_to_json(&outcome),
                        );
                        write_json(stream, 500, &json);
                        return;
                    }
                }
            }
            let json = format!(
                "{{\"ok\":true,\"session_id\":{},\"permission_id\":{},\"resume_strategy\":{},\"outcome\":{}}}",
                json_string(&outcome.session_id),
                json_string(&outcome.permission_id),
                json_string(resume_strategy),
                permission_outcome_to_json(&outcome),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn handle_submit_plan_decision(
    stream: &mut TcpStream,
    state: Arc<LocalApiRuntimeState>,
    req: &HttpRequest,
) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };
    let plan_approval_id = match extract_json_string_field(&body, "plan_approval_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"plan_approval_id is required"}"#,
            );
            return;
        }
    };
    let decision = match extract_json_string_field(&body, "decision").as_deref() {
        Some("approve") => PlanApprovalDecisionKind::Approve,
        Some("reject") => PlanApprovalDecisionKind::Reject,
        _ => PlanApprovalDecisionKind::RequestRevision,
    };
    let continue_after = matches!(decision, PlanApprovalDecisionKind::Approve);

    match state
        .facade
        .submit_plan_decision(&session_id, &plan_approval_id, decision)
    {
        Ok(()) => {
            let mut resume_strategy = "none";
            if continue_after {
                match spawn_local_api_continue_from_last_prompt(
                    Arc::clone(&state),
                    session_id.clone(),
                    Some(APPROVED_PLAN_CONTINUE_PROMPT.to_string()),
                ) {
                    Ok(()) => {
                        resume_strategy = "async_plan_resume";
                    }
                    Err(error) => {
                        let _ = state.facade.record_runtime_error(
                            &session_id,
                            "runtime_continue_failed",
                            &error,
                        );
                        let json = format!(
                            "{{\"ok\":false,\"session_id\":{},\"plan_approval_id\":{},\"error_code\":\"runtime_continue_failed\",\"error\":{}}}",
                            json_string(&session_id),
                            json_string(&plan_approval_id),
                            json_string(&error),
                        );
                        write_json(stream, 500, &json);
                        return;
                    }
                }
            }
            let json = format!(
                "{{\"ok\":true,\"session_id\":{},\"plan_approval_id\":{},\"resume_strategy\":{}}}",
                json_string(&session_id),
                json_string(&plan_approval_id),
                json_string(resume_strategy),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

fn should_continue_after_permission_decision(decision: &PermissionDecisionKind) -> bool {
    matches!(
        decision,
        PermissionDecisionKind::AllowOnce
            | PermissionDecisionKind::AllowSession
            | PermissionDecisionKind::AllowProjectRule
    )
}

fn submit_permission_decision_when_ready(
    state: &LocalApiRuntimeState,
    session_id: &str,
    permission_id: &str,
    decision: PermissionDecisionKind,
) -> Result<RuntimePermissionDecisionOutcome, String> {
    let mut last_error = None;
    let mut inactive_settle_attempts = 0usize;
    for attempt in 0..=60 {
        match state.facade.submit_permission_decision_with_outcome(
            session_id,
            permission_id,
            decision.clone(),
        ) {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                last_error = Some(error);
                let active = state.clear_active_turn_if_resumable_boundary(session_id)?
                    || state
                        .active_turns
                        .lock()
                        .map_err(|_| "runtime active-turn lock poisoned".to_string())?
                        .contains_key(session_id);
                if !active {
                    inactive_settle_attempts += 1;
                }
                if attempt == 60 || (!active && inactive_settle_attempts >= 6) {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "permission decision was not submitted".to_string()))
}

fn handle_preview_tool(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let body = body_as_string(req);
    let tool_id = match extract_json_string_field(&body, "tool_id") {
        Some(id) => id,
        None => {
            write_json(stream, 400, r#"{"ok":false,"error":"tool_id is required"}"#);
            return;
        }
    };
    let tool_call_id = extract_json_string_field(&body, "tool_call_id")
        .unwrap_or_else(|| format!("preview_{tool_id}"));

    // Build ToolExecutionArgs from the body (simple string args)
    let args = ToolExecutionArgs {
        path: extract_json_string_field(&body, "path")
            .or_else(|| extract_json_string_field(&body, "file_path")),
        content: extract_json_string_field(&body, "content")
            .or_else(|| extract_json_string_field(&body, "new_string")),
        old_string: extract_json_string_field(&body, "old_string"),
        new_string: extract_json_string_field(&body, "new_string"),
        command: extract_json_string_field(&body, "command"),
        max_bytes: extract_json_int_field(&body, "max_bytes").map(|v| v as usize),
        max_results: extract_json_int_field(&body, "max_results").map(|v| v as usize),
        max_files: extract_json_int_field(&body, "max_files").map(|v| v as usize),
        max_depth: extract_json_int_field(&body, "max_depth").map(|v| v as usize),
        offset: extract_json_int_field(&body, "offset").map(|v| v as usize),
        pattern: extract_json_string_field(&body, "pattern"),
        root: extract_json_string_field(&body, "root"),
        base_hash: extract_json_string_field(&body, "base_hash"),
        replace_all: extract_json_string_field(&body, "replace_all").map(|v| v == "true"),
        edits_json: extract_json_string_field(&body, "edits_json")
            .or_else(|| extract_json_string_field(&body, "edits")),
        query: extract_json_string_field(&body, "query"),
        job_id: extract_json_string_field(&body, "job_id"),
        ..Default::default()
    };

    match facade.preview_tool(&facade.workspace_root(), &tool_call_id, &tool_id, args) {
        Ok(result) => {
            let json = format!(
                "{{\"ok\":true,\"tool_id\":{},\"result\":{{\"ok\":{},\"preview\":{},\"detail_json\":{}}}}}",
                json_string(&tool_id),
                result.ok,
                json_string(&result.preview),
                json_string(&result.detail_json),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!(
                "{{\"ok\":false,\"tool_id\":{},\"error\":{}}}",
                json_string(&tool_id),
                json_string(&e),
            );
            write_json(stream, 400, &json);
        }
    }
}

fn handle_legacy_tool_preview(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let body = body_as_string(req);
    let tool_id = match extract_json_string_field(&body, "tool_id") {
        Some(id) => id,
        None => {
            write_json(stream, 400, r#"{"ok":false,"error":"tool_id is required"}"#);
            return;
        }
    };
    let tool_call_id = format!("preview_{tool_id}");
    // Legacy format: { tool_id, arguments: { path, content, command, ... } }
    let arguments_str = extract_json_object_field(&body, "arguments").unwrap_or_default();
    let args = ToolExecutionArgs {
        path: extract_json_string_field(&arguments_str, "path"),
        content: extract_json_string_field(&arguments_str, "content")
            .or_else(|| extract_json_string_field(&arguments_str, "new_string")),
        old_string: extract_json_string_field(&arguments_str, "old_string"),
        new_string: extract_json_string_field(&arguments_str, "new_string"),
        command: extract_json_string_field(&arguments_str, "command"),
        max_bytes: extract_json_int_field(&arguments_str, "max_bytes").map(|v| v as usize),
        max_results: extract_json_int_field(&arguments_str, "max_results").map(|v| v as usize),
        max_files: extract_json_int_field(&arguments_str, "max_files").map(|v| v as usize),
        max_depth: extract_json_int_field(&arguments_str, "max_depth").map(|v| v as usize),
        offset: extract_json_int_field(&arguments_str, "offset").map(|v| v as usize),
        pattern: extract_json_string_field(&arguments_str, "pattern"),
        root: extract_json_string_field(&arguments_str, "root")
            .or_else(|| extract_json_string_field(&arguments_str, "directory")),
        base_hash: extract_json_string_field(&arguments_str, "base_hash"),
        replace_all: extract_json_string_field(&arguments_str, "replace_all").map(|v| v == "true"),
        edits_json: extract_json_string_field(&arguments_str, "edits_json")
            .or_else(|| extract_json_string_field(&arguments_str, "edits")),
        query: extract_json_string_field(&arguments_str, "query"),
        job_id: extract_json_string_field(&arguments_str, "job_id"),
        ..Default::default()
    };

    match facade.preview_tool(facade.workspace_root(), &tool_call_id, &tool_id, args) {
        Ok(result) => {
            let json = format!(
                "{{\"ok\":true,\"tool_id\":{},\"result\":{{\"ok\":{},\"preview\":{},\"detail_json\":{}}}}}",
                json_string(&tool_id),
                result.ok,
                json_string(&result.preview),
                json_string(&result.detail_json),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!(
                "{{\"ok\":false,\"tool_id\":{},\"error\":{}}}",
                json_string(&tool_id),
                json_string(&e),
            );
            write_json(stream, 400, &json);
        }
    }
}

fn handle_export_events(stream: &mut TcpStream, facade: &RuntimeFacade, req: &HttpRequest) {
    let body = body_as_string(req);
    let session_id = match extract_json_string_field(&body, "session_id") {
        Some(id) => id,
        None => {
            write_json(
                stream,
                400,
                r#"{"ok":false,"error":"session_id is required"}"#,
            );
            return;
        }
    };
    let path = extract_json_string_field(&body, "path")
        .unwrap_or_else(|| format!("events/{session_id}.jsonl"));

    match facade.export_events(&session_id, Path::new(&path)) {
        Ok(()) => {
            let json = format!(
                "{{\"ok\":true,\"session_id\":{},\"path\":{}}}",
                json_string(&session_id),
                json_string(&path),
            );
            write_json(stream, 200, &json);
        }
        Err(e) => {
            let json = format!("{{\"ok\":false,\"error\":{}}}", json_string(&e));
            write_json(stream, 400, &json);
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot / outcome serialization
// ---------------------------------------------------------------------------

fn snapshot_to_json(s: &RuntimeSessionSnapshot) -> String {
    // NOTE: The `events` field is an empty placeholder. The desktop GUI
    // (apps/desktop/dist/app.js) expects `snap.events` at the top level for
    // polling fallback (line 82-84). When the runtime can cheaply export the
    // event log alongside the snapshot, populate this array directly to avoid
    // an extra round-trip.
    format!(
        "{{\"session_id\":{},\"state\":{},\"event_count\":{},\"model_mode\":{},\"autonomy_mode\":{},\"workspace_root\":{},\"pending_permission_count\":{},\"pending_plan_approval_count\":{},\"plan_mode_active\":{},\"session_memory_count\":{},\"events\":[],\"discovered_roots\":[],\"path_corrections\":{{}}}}",
        json_string(&s.session_id),
        json_string(agent_state_as_str(s.state)),
        s.event_count,
        json_string(s.model_mode.as_str()),
        json_string(s.autonomy_mode.as_str()),
        json_string(&s.workspace_root.to_string_lossy()),
        s.pending_permission_count,
        s.pending_plan_approval_count,
        s.plan_mode_active,
        s.session_memory_count,
    )
}

fn agent_state_as_str(state: AgentState) -> &'static str {
    match state {
        AgentState::Created => "Created",
        AgentState::Planning => "Planning",
        AgentState::WaitingForPlanApproval => "WaitingForPlanApproval",
        AgentState::RetrievingContext => "RetrievingContext",
        AgentState::Executing => "Executing",
        AgentState::WaitingForToolApproval => "WaitingForToolApproval",
        AgentState::ApplyingPatch => "ApplyingPatch",
        AgentState::RunningCommand => "RunningCommand",
        AgentState::DiagnosingFailure => "DiagnosingFailure",
        AgentState::Reviewing => "Reviewing",
        AgentState::WaitingForUser => "WaitingForUser",
        AgentState::Completed => "Completed",
        AgentState::Failed => "Failed",
        AgentState::Cancelled => "Cancelled",
    }
}

fn permission_outcome_to_json(o: &RuntimePermissionDecisionOutcome) -> String {
    format!(
        "{{\"session_id\":{},\"permission_id\":{},\"tool_call_id\":{},\"provider_tool_call_id\":{},\"tool_id\":{},\"resume_strategy\":{},\"tool_executed\":{},\"model_continuation_required\":{},\"error_code\":{},\"tool_result\":{}}}",
        json_string(&o.session_id),
        json_string(&o.permission_id),
        opt_json_string(&o.tool_call_id),
        opt_json_string(&o.provider_tool_call_id),
        opt_json_string(&o.tool_id),
        json_string(&o.resume_strategy),
        o.tool_executed,
        o.model_continuation_required,
        opt_json_string(&o.error_code),
        opt_json_string(&None::<String>),
    )
}

// ---------------------------------------------------------------------------
// Static catalog data
// ---------------------------------------------------------------------------

fn list_tools_json() -> String {
    r#"{"tools":[{"tool_id":"file.read","category":"file","risk":"read_only"},{"tool_id":"file.edit","category":"file","risk":"writes_files"},{"tool_id":"file.write","category":"file","risk":"writes_files"},{"tool_id":"search.ripgrep","category":"search","risk":"read_only"},{"tool_id":"repo.map","category":"search","risk":"read_only"},{"tool_id":"shell.command","category":"shell","risk":"executes_command"},{"tool_id":"patch.apply","category":"patch","risk":"writes_files"},{"tool_id":"git.status","category":"git","risk":"read_only"},{"tool_id":"todo.write","category":"todo","risk":"interactive"},{"tool_id":"plan.enter","category":"plan","risk":"interactive"},{"tool_id":"plan.exit","category":"plan","risk":"interactive"}]}"#
        .to_string()
}

fn list_commands_json() -> String {
    r#"{"commands":[{"name":"/repo","description":"Build repo map"},{"name":"/read","description":"Read a file"},{"name":"/search","description":"Search files"},{"name":"/git","description":"Git status"},{"name":"/plan","description":"Enter plan mode"},{"name":"/run","description":"Run a shell command"},{"name":"/tools","description":"List available tools"}]}"#
        .to_string()
}

fn list_agents_json() -> String {
    r#"{"agents":[{"type":"general-purpose","description":"General-purpose agent"},{"type":"explorer","description":"Read-only exploration agent"},{"type":"reviewer","description":"Code review agent"},{"type":"bug-analyzer","description":"Bug analysis agent"},{"type":"plan","description":"Planning agent"}]}"#
        .to_string()
}

fn tool_catalog_json() -> String {
    r#"{"tool_catalog":[{"tool_id":"file.read","display_name":"Read File","category":"file","risk":"read_only","permission_required":false,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":80000,"result_policy":"inline"},{"tool_id":"file.edit","display_name":"Edit File","category":"file","risk":"writes_files","permission_required":true,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":40000,"result_policy":"preview_and_artifact"},{"tool_id":"file.write","display_name":"Write File","category":"file","risk":"writes_files","permission_required":true,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":40000,"result_policy":"preview_and_artifact"},{"tool_id":"search.ripgrep","display_name":"Search Files","category":"search","risk":"read_only","permission_required":false,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":40000,"result_policy":"preview_and_artifact"},{"tool_id":"repo.map","display_name":"Build Repo Map","category":"search","risk":"read_only","permission_required":false,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":40000,"result_policy":"preview_and_artifact"},{"tool_id":"shell.command","display_name":"Run Shell Command","category":"shell","risk":"executes_command","permission_required":true,"enabled_by_default":true,"concurrency_safe":false,"max_result_size_chars":20000,"result_policy":"preview_and_artifact"},{"tool_id":"patch.apply","display_name":"Apply Patch","category":"patch","risk":"writes_files","permission_required":true,"enabled_by_default":true,"concurrency_safe":false,"max_result_size_chars":40000,"result_policy":"preview_and_artifact"},{"tool_id":"git.status","display_name":"Git Status","category":"git","risk":"read_only","permission_required":false,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":20000,"result_policy":"inline"},{"tool_id":"todo.write","display_name":"Write Todo List","category":"todo","risk":"interactive","permission_required":false,"enabled_by_default":true,"concurrency_safe":true,"max_result_size_chars":8000,"result_policy":"inline"},{"tool_id":"plan.enter","display_name":"Enter Plan Mode","category":"plan","risk":"interactive","permission_required":true,"enabled_by_default":true,"concurrency_safe":false,"max_result_size_chars":20000,"result_policy":"inline"},{"tool_id":"plan.exit","display_name":"Exit Plan Mode","category":"plan","risk":"interactive","permission_required":true,"enabled_by_default":true,"concurrency_safe":false,"max_result_size_chars":4000,"result_policy":"inline"}]}"#
        .to_string()
}

// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\u{:04x}", other as u32);
            }
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => {
                use std::fmt::Write;
                let _ = write!(escaped, "\\u{:04x}", other as u32);
            }
            other => escaped.push(other),
        }
    }
    escaped
}

fn opt_json_string(value: &Option<String>) -> String {
    match value {
        Some(v) => json_string(v),
        None => "null".to_string(),
    }
}

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn deepseek_live_endpoint_from_env() -> NativeProviderEndpoint {
    let protocol = env::var("RESEARCHCODE_DEEPSEEK_PROTOCOL")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let env_base_url = env::var("DEEPSEEK_BASE_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let mut endpoint = if matches!(protocol.as_str(), "openai" | "openai_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_openai()
    } else if matches!(protocol.as_str(), "anthropic" | "anthropic_compatible") {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else if env_base_url
        .as_deref()
        .is_some_and(|value| value.contains("/anthropic"))
    {
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    } else {
        // DS4's Anthropic-compatible Messages API exposes Claude-like content
        // blocks, so it is the native DeepSeek loop path. OpenAI-compatible
        // chat/completions remains an explicit fallback via
        // RESEARCHCODE_DEEPSEEK_PROTOCOL=openai.
        NativeProviderEndpoint::deepseek_v4_flash_anthropic()
    };
    endpoint.live_calls_enabled_by_default = true;
    if let Some(base_url) = env_base_url {
        endpoint.base_url = normalize_deepseek_base_url_for_protocol(&endpoint.protocol, &base_url);
    }
    if let Ok(model_name) = env::var("DEEPSEEK_MODEL") {
        let model_name = model_name.trim();
        if !model_name.is_empty() {
            endpoint.actual_model_name = model_name.to_string();
        }
    }
    endpoint
}

fn deepseek_openai_fallback_endpoint_from(
    primary: &NativeProviderEndpoint,
) -> NativeProviderEndpoint {
    let mut fallback = NativeProviderEndpoint::deepseek_v4_flash_openai();
    fallback.live_calls_enabled_by_default = true;
    fallback.actual_model_name = primary.actual_model_name.clone();
    fallback.display_model_name = primary.display_model_name.clone();
    if let Ok(base_url) = env::var("DEEPSEEK_OPENAI_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            fallback.base_url = base_url.to_string();
            return fallback;
        }
    }
    if primary.base_url.contains("/anthropic") {
        fallback.base_url = primary.base_url.replace("/anthropic", "");
    }
    fallback
}

fn normalize_deepseek_base_url_for_protocol(protocol: &str, base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if protocol == "anthropic_compatible" {
        if trimmed.ends_with("/anthropic") {
            return trimmed.to_string();
        }
        if let Some(root) = trimmed.strip_suffix("/v1") {
            return format!("{root}/anthropic");
        }
        return format!("{trimmed}/anthropic");
    }
    trimmed.to_string()
}

fn qwen_live_endpoint_from_env() -> NativeProviderEndpoint {
    let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
    endpoint.live_calls_enabled_by_default = true;
    if let Ok(base_url) = env::var("QWEN_BASE_URL") {
        let base_url = base_url.trim();
        if !base_url.is_empty() {
            endpoint.base_url = base_url.to_string();
        }
    }
    endpoint
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

fn is_authorized_header(auth_header: Option<&String>) -> bool {
    let token = std::env::var("RESEARCHCODE_LOCAL_API_TOKEN")
        .unwrap_or_default()
        .trim()
        .to_string();
    if token.is_empty() {
        return true;
    }
    match auth_header {
        Some(value) => value.trim() == format!("Bearer {token}"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn http_test_lock() -> MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn active_turn_generation_ignores_stale_release() {
        let root = std::env::temp_dir().join("researchcode-active-turn-generation-test");
        let state = LocalApiRuntimeState::new(root.clone(), root.join("artifacts"));
        let session_id = "runtime_session_generation_test";

        {
            let mut turns = state.active_turns.lock().unwrap();
            turns.insert(session_id.to_string(), 10);
        }

        state.release_turn_generation(session_id, 9);
        assert_eq!(
            state.active_turns.lock().unwrap().get(session_id).copied(),
            Some(10)
        );

        {
            let mut turns = state.active_turns.lock().unwrap();
            turns.insert(session_id.to_string(), 11);
        }

        state.release_turn_generation(session_id, 10);
        assert_eq!(
            state.active_turns.lock().unwrap().get(session_id).copied(),
            Some(11)
        );

        state.release_turn_generation(session_id, 11);
        assert!(!state.active_turns.lock().unwrap().contains_key(session_id));
    }

    #[test]
    fn active_turn_waiting_approval_boundary_is_released_for_resume() {
        let root = std::env::temp_dir().join("researchcode-active-turn-approval-boundary-test");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let state = LocalApiRuntimeState::new(root.clone(), root.join("artifacts"));
        let handle = state
            .facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        state
            .facade
            .execute_session_tool(
                &handle.session_id,
                "plan_boundary_tool",
                "plan.enter",
                ToolExecutionArgs {
                    content: Some("Plan: test approval boundary".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        state
            .active_turns
            .lock()
            .unwrap()
            .insert(handle.session_id.clone(), 42);

        let still_active = state
            .clear_active_turn_if_resumable_boundary(&handle.session_id)
            .unwrap();

        assert!(!still_active);
        assert!(!state
            .active_turns
            .lock()
            .unwrap()
            .contains_key(&handle.session_id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn active_turn_generation_checked_clear_does_not_remove_new_generation() {
        let session_id = "runtime_session_generation_race_test";
        let mut active_turns = HashMap::new();
        active_turns.insert(session_id.to_string(), 43);

        let cleared = clear_matching_active_turn_generation(&mut active_turns, session_id, 42);

        assert!(!cleared);
        assert_eq!(active_turns.get(session_id).copied(), Some(43));
        assert!(clear_matching_active_turn_generation(
            &mut active_turns,
            session_id,
            43
        ));
        assert!(!active_turns.contains_key(session_id));
    }

    #[test]
    fn approved_plan_resume_prompt_preserves_original_user_task() {
        let prompt = continuation_prompt_from_last_user_task(
            Some(APPROVED_PLAN_CONTINUE_PROMPT),
            Some("Create VoiceNote XCTest files, then run swift test."),
        );

        assert!(prompt.contains("The plan was approved."));
        assert!(prompt.contains("Original user request to continue:"));
        assert!(prompt.contains("Create VoiceNote XCTest files"));
        assert!(prompt.contains("swift test"));
    }

    #[test]
    fn permission_resume_prompt_uses_last_user_task_without_preamble() {
        let prompt = continuation_prompt_from_last_user_task(
            None,
            Some("Continue deleting obsolete generated files."),
        );

        assert_eq!(prompt, "Continue deleting obsolete generated files.");
    }

    #[test]
    fn active_turn_acquire_does_not_overwrite_live_generation() {
        let root = std::env::temp_dir().join("researchcode-active-turn-acquire-test");
        let state = LocalApiRuntimeState::new(root.clone(), root.join("artifacts"));
        let session_id = "runtime_session_acquire_test";

        {
            let mut turns = state.active_turns.lock().unwrap();
            turns.insert(session_id.to_string(), 20);
        }

        assert_eq!(state.acquire_turn_generation(session_id), None);
        assert_eq!(
            state.active_turns.lock().unwrap().get(session_id).copied(),
            Some(20)
        );
    }

    #[test]
    fn json_string_escapes_special_chars() {
        let s = json_string("hello\"world\\test\nline2");
        assert!(s.contains("\\\""));
        assert!(s.contains("\\\\"));
        assert!(s.contains("\\n"));
    }

    #[test]
    fn json_string_handles_control_chars() {
        let s = json_string("tab:\there");
        assert!(s.contains("\\u0009") || s.contains("\\t"));
    }

    #[test]
    fn json_string_wraps_in_quotes() {
        let s = json_string("hello");
        assert_eq!(s, "\"hello\"");
    }

    #[test]
    fn url_decode_handles_percent_encoding() {
        assert_eq!(url_decode("hello%20world"), "hello world");
        assert_eq!(url_decode("a%2Bb"), "a+b"); // %2B is '+', not space
    }

    #[test]
    fn url_decode_handles_plus() {
        assert_eq!(url_decode("hello+world"), "hello world");
    }

    #[test]
    fn query_param_extracts_value() {
        let req = HttpRequest {
            method: "GET".to_string(),
            path: "/test?session_id=abc123&cursor=5".to_string(),
            headers: HashMap::new(),
            body: vec![],
        };
        assert_eq!(query_param(&req, "session_id"), Some("abc123".to_string()));
        assert_eq!(query_param(&req, "cursor"), Some("5".to_string()));
        assert_eq!(query_param(&req, "missing"), None);
    }

    #[test]
    fn extract_json_string_field_handles_escapes() {
        let body = r#"{"text":"hello \"world\"","other":true}"#;
        assert_eq!(
            extract_json_string_field(body, "text"),
            Some("hello \"world\"".to_string())
        );
    }

    #[test]
    fn extract_json_int_field_parses_numbers() {
        let body = r#"{"cursor":42,"other":"x"}"#;
        assert_eq!(extract_json_int_field(body, "cursor"), Some(42));
    }

    #[test]
    fn server_start_stop_lifecycle() {
        let _guard = http_test_lock();
        let config = LocalApiServerConfig {
            port: 0, // OS-assigned
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");
        assert!(port > 0);
        assert!(server.is_running());
        server.stop();
        // Give the accept thread time to notice
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    #[test]
    fn health_endpoint_returns_ok() {
        let _guard = http_test_lock();
        let config = LocalApiServerConfig {
            port: 0,
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");

        // Connect and send a GET /health request
        let mut stream =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("write request");

        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(response.contains("200 OK"));
        assert!(response.contains("researchcode-local-api"));

        server.stop();
    }

    #[test]
    fn start_session_endpoint_returns_session_json() {
        let _guard = http_test_lock();
        let config = LocalApiServerConfig {
            port: 0,
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");

        let mut stream =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        let body = r#"{"workspace":".","model_mode":"deepseek","autonomy_mode":"fast_auto"}"#;
        let request = format!(
            "POST /runtime/start-session HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
        stream.write_all(request.as_bytes()).expect("write request");

        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(response.contains("200 OK"));
        assert!(response.contains("\"ok\":true"));
        assert!(response.contains("session_id"));
        assert!(response.contains("runtime_session_"));

        server.stop();
    }

    #[test]
    fn not_found_endpoint_returns_404() {
        let _guard = http_test_lock();
        let config = LocalApiServerConfig {
            port: 0,
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");

        let mut stream =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream
            .write_all(b"GET /nonexistent HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("write request");

        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(response.contains("404 Not Found"));

        server.stop();
    }

    #[test]
    fn list_tools_returns_catalog() {
        let _guard = http_test_lock();
        let config = LocalApiServerConfig {
            port: 0,
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");

        let mut stream =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream
            .write_all(
                b"GET /runtime/list-tools HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");

        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(response.contains("200 OK"));
        assert!(response.contains("file.read"));
        assert!(response.contains("shell.command"));

        server.stop();
    }

    #[test]
    fn deepseek_anthropic_protocol_normalizes_openai_base_url() {
        assert_eq!(
            normalize_deepseek_base_url_for_protocol(
                "anthropic_compatible",
                "https://api.deepseek.com/v1"
            ),
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            normalize_deepseek_base_url_for_protocol(
                "anthropic_compatible",
                "https://api.deepseek.com/anthropic"
            ),
            "https://api.deepseek.com/anthropic"
        );
        assert_eq!(
            normalize_deepseek_base_url_for_protocol(
                "openai_compatible",
                "https://api.deepseek.com/v1"
            ),
            "https://api.deepseek.com/v1"
        );
    }

    #[test]
    fn local_api_turn_timeout_defaults_to_uncapped() {
        std::env::remove_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS");
        assert!(local_api_turn_timeout().is_none());
        std::env::set_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS", "0");
        assert!(local_api_turn_timeout().is_none());
        std::env::set_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS", "120");
        assert_eq!(local_api_turn_timeout(), Some(Duration::from_secs(120)));
        std::env::remove_var("RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS");
    }

    #[test]
    fn unauthorized_request_returns_401_when_token_set() {
        let _guard = http_test_lock();
        // Set a token so auth is enforced; clear it when done
        std::env::set_var("RESEARCHCODE_LOCAL_API_TOKEN", "test-token-123");

        let config = LocalApiServerConfig {
            port: 0,
            ..Default::default()
        };
        let server = LocalApiServer::new(config);
        let port = server.start().expect("start server");

        // Request without auth header → should get 401
        let mut stream =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream
            .write_all(
                b"GET /runtime/list-tools HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .expect("write request");

        let mut response = String::new();
        std::io::Read::read_to_string(&mut stream, &mut response).expect("read response");
        assert!(response.contains("401") || response.contains("unauthorized"));

        // Request with wrong token → should get 401
        let mut stream2 =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream2
            .write_all(b"GET /runtime/list-tools HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer wrong-token\r\nConnection: close\r\n\r\n")
            .expect("write request");

        let mut response2 = String::new();
        std::io::Read::read_to_string(&mut stream2, &mut response2).expect("read response");
        assert!(response2.contains("401") || response2.contains("unauthorized"));

        // Request with correct token → should succeed
        let mut stream3 =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream3
            .write_all(b"GET /runtime/list-tools HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer test-token-123\r\nConnection: close\r\n\r\n")
            .expect("write request");

        let mut response3 = String::new();
        std::io::Read::read_to_string(&mut stream3, &mut response3).expect("read response");
        assert!(response3.contains("200 OK"));

        // Health endpoint should still be accessible without auth
        let mut stream4 =
            std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
        stream4
            .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .expect("write request");

        let mut response4 = String::new();
        std::io::Read::read_to_string(&mut stream4, &mut response4).expect("read response");
        assert!(response4.contains("200 OK"));

        server.stop();
        std::env::remove_var("RESEARCHCODE_LOCAL_API_TOKEN");
    }
}
