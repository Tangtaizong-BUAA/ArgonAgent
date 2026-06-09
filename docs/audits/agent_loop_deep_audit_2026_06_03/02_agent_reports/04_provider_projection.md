# Agent 4: Provider Projection Audit

## Conclusion

The provider projection layer has correct architecture for the three native paths (DeepSeek-Anthropic, DeepSeek-OpenAI, Qwen-OpenAI) but contains critical issues in the generic compatible provider Anthropic path (flat strings instead of content blocks) and several P1/P2 issues in dual-protocol fallback streaming and error flag detection. DeepSeek OpenAI reasoning_content replay is well-implemented.

**Severity:** P0 (generic compatible Anthropic path sends malformed requests)

## Files Involved

- `crates/runtime/src/compatible_provider.rs` (117-153) — generic Anthropic body builder
- `crates/runtime/src/live_model_request.rs` — all native request builders (305-363 Anthropic, 590-662 OpenAI, 841-873 legacy history)
- `crates/runtime/src/native_agent_loop_continuation.rs` (246-264 reasoning, 395-404 error flag, 413-421 streaming observer)
- `crates/runtime/src/agent_kernel/conversation_history.rs` (91-106 orphan risk)
- `crates/runtime/src/sidecar_http_transport.rs`

## Key Findings

### Finding 1: Generic Compatible Anthropic Path Broken (P0)

`compatible_provider.rs:144-148` serializes all messages as `{"role":"...","content":"..."}` with flat strings. Never produces structured content blocks (`type: "tool_use"`, `type: "tool_result"`). Tool results use role `"tool"` which is invalid in Anthropic protocol. **Any compatible provider using Anthropic protocol with tool calls will get 400 errors.**

### Finding 2: Dirty Streaming Events on 400 Fallback (P1)

When Anthropic endpoint returns 400 and dual-protocol retry succeeds on OpenAI, the `stream_event_handler` has already emitted `ContentBlockStarted`/`ContentBlockFinished`/`ToolCallFinished` events from the failed attempt. Text deltas are correctly buffered but content-block events leak.

### Finding 3: Legacy History Anthropic Projection Loses Metadata (P1)

`anthropic_history_content` only preserves tool_call IDs, not function names or arguments. Produces opaque strings like `assistant tool_calls [call_1, call_2]: text`.

### Finding 4: Dual reasoning_content Injection in Anthropic Path (P2)

DeepSeek Anthropic builder injects reasoning BOTH as a proper `thinking` content block AND as a non-standard `reasoning_content` message-level field. The latter is not part of the Anthropic Messages API spec.

### Finding 5: Orphan Tool Result Risk (P2)

`conversation_messages_from_event_log` emits tool result messages and tool call messages independently with no cross-validation. A `tool.result_recorded` without matching `tool.call_requested` produces orphaned tool messages → 400 error.

### Finding 6: Error Flag Uses Fragile Substring Match (P2)

`deepseek_tool_results_from_messages` detects `is_error` via `content.contains("\"is_error\":true")` — fragile substring matching.

### Finding 7: DeepSeek OpenAI reasoning_content Replay Correct (P0 ✓)

Correctly replays reasoning_content at message level, appears before tool_calls in JSON, truncated at 48K chars.

## doc39 Conflict

- **Yes** (§2.4): Compatible Anthropic path violates structured content block requirement
- **Yes** (§3.1): Dual-protocol fallback leaks events from failed attempt
- **Yes** (§4.2): Orphan tool result risk in conversation projection
- **No** (§2.3): reasoning_content replay correctly implemented

## Suggested Fix

1. Fix `compatible_provider.rs` Anthropic path to produce structured content blocks
2. Gate `stream_event_handler` on `live_stream_status_is_success`
3. Add tool_call_id cross-validation in conversation history projection
4. Remove non-standard `reasoning_content` field from Anthropic builder
5. Replace substring-based error detection with structured field

## Handoff Needed

- Agent 3 (Event Identity) — orphan tool result risk in conversation projection
- Agent 5 (DeepSeek Stream) — dual-protocol streaming events
