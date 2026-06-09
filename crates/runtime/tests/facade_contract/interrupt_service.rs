use super::support::*;
use researchcode_runtime::agent_kernel::PermissionMode;
use researchcode_runtime::live_http_transport::{LiveHttpResponse, ScriptedLiveHttpTransport};
use researchcode_runtime::native_agent_loop::{NativeAgentLoopV2Request, NativeAgentToolExposure};
use researchcode_runtime::native_provider::NativeProviderEndpoint;
use researchcode_runtime::runtime_facade::RuntimeFacade;
use std::sync::atomic::{AtomicBool, Ordering};

fn blocked_request(fx: &FacadeFixture, session_id: &str) -> NativeAgentLoopV2Request {
    NativeAgentLoopV2Request {
        project_id: "local".to_string(),
        session_id: session_id.to_string(),
        task_id: "task".to_string(),
        turn_id: Some("turn".to_string()),
        workspace_root: fx.workspace.clone(),
        artifact_root: fx.artifacts.join(session_id),
        endpoint: NativeProviderEndpoint::deepseek_v4_flash_openai(),
        prompt: "say hi".to_string(),
        max_tokens: 64,
        max_iterations: 1,
        max_tool_calls: 0,
        tool_exposure: NativeAgentToolExposure::ReadOnly,
        permission_mode: PermissionMode::Default,
        provided_permission_decisions: Vec::new(),
        deepseek_adaptation: None,
        error_recovery: None,
        hook_dispatcher: None,
        concurrent_tool_execution: false,
    }
}

#[test]
fn interrupt_boundary_accepts_false_flag() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(Vec::new());
    let interrupt = AtomicBool::new(false);
    let result = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        None,
        &interrupt,
    )
    .unwrap();
    assert!(format!("{:?}", result.status).contains("Blocked"));
}

#[test]
fn interrupt_boundary_accepts_true_flag() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(Vec::new());
    let interrupt = AtomicBool::new(true);
    let result = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        None,
        &interrupt,
    )
    .unwrap();
    assert!(format!("{:?}", result.status).contains("Interrupted"));
}

#[test]
fn interrupt_boundary_does_not_clear_true_flag() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(Vec::new());
    let interrupt = AtomicBool::new(true);
    let _ = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        None,
        &interrupt,
    )
    .unwrap();
    assert!(interrupt.load(Ordering::Relaxed));
}

#[test]
fn interrupt_boundary_does_not_set_false_flag() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(Vec::new());
    let interrupt = AtomicBool::new(false);
    let _ = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        None,
        &interrupt,
    )
    .unwrap();
    assert!(!interrupt.load(Ordering::Relaxed));
}

#[test]
fn interrupt_boundary_preserves_event_sink_when_gate_blocks_before_transport() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: "unused".to_string(),
    }]);
    let interrupt = AtomicBool::new(false);
    let mut lines = Vec::new();
    let mut sink = |line: &str| lines.push(line.to_string());
    let _ = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        Some(&mut sink),
        &interrupt,
    )
    .unwrap();
    assert!(lines.iter().any(|line| line.contains("model.call_blocked")));
}

#[test]
fn interrupt_boundary_leaves_scripted_transport_unused_when_gate_blocks() {
    let fx = FacadeFixture::new();
    let handle = fx.start();
    let transport = ScriptedLiveHttpTransport::new(vec![LiveHttpResponse {
        status_code: 200,
        body: "unused".to_string(),
    }]);
    let interrupt = AtomicBool::new(false);
    let _ = RuntimeFacade::run_deepseek_agent_loop_request_with_interrupt(
        &transport,
        blocked_request(&fx, &handle.session_id),
        None,
        &interrupt,
    )
    .unwrap();
    assert!(transport.sent_requests().is_empty());
}
