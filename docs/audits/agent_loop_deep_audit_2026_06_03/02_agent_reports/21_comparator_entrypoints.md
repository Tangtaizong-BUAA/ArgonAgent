Now I have a thorough understanding of all the files. Here is the complete line-by-line review.

---

## File 1: `native_agent_loop_entrypoints.rs` (497 lines)

### Bugs Found

**Line 142 -- [#allow(dead_code)] on resume function**
- **Severity:** MEDIUM
- **Description:** The function `run_native_agent_loop_v2_deepseek_resume` (lines 142-497, ~355 lines) is marked `#[allow(dead_code)]` and never called anywhere in the codebase (confirmed by grep). This is a large duplicate of logic already handled by `run_native_agent_loop_v2_deepseek_inner` in `native_agent_loop.rs`. The actual resume path used is `resume_native_agent_loop_after_external_decision` in `native_agent_loop_resume.rs`.
- **Reference:** OpenClaudeCode has a single query loop; resume is handled within the main loop via state rehydration (query.ts lines 268-269).
- **Fix:** Remove this function or gate it behind a feature flag if it represents a planned migration path. Its continued presence adds maintenance burden (it must be kept in sync with the inner loop's changes).

**Line 223 -- event_sink shadowing causes parameter to be discarded**
- **Severity:** HIGH
- **Description:** Line 223 declares `let mut event_sink: Option<&mut dyn FnMut(&str)> = None;` which shadows the `event_sink: Option<&mut dyn FnMut(&str)>` parameter from line 135 (received via `run_native_agent_loop_v2_deepseek_with_interrupt`). The parameter is never used; all stream event emissions use the local `None` value. This means callers of `with_interrupt` that pass a real event sink will never see stream events.
- **Fix:** Delete line 223. The parameter binding at line 138 (`event_sink`) should flow through unchanged. But note this function is dead code, so the bug has no runtime impact currently.

**Lines 152-157 -- Guidance injected as stream delta with incorrect semantics**
- **Severity:** MEDIUM
- **Description:** User guidance is recorded as `record_model_stream_delta("user_guidance", "user", "input", ...)` which records it as a model stream delta from actor "user". The event type `model.stream_delta` implies model output, not user input. This will confuse downstream consumers parsing the event log.
- **Reference:** OpenClaudeCode uses `createUserInterruptionMessage` which yields a proper `UserMessage` type (query.ts line 1047).
- **Fix:** Use a proper user message event type, or record as a `runtime_event` with `Actor::User`.

**Lines 222-224 -- unused event_sink**
- **Severity:** LOW
- **Description:** Lines 222-224 declare local variables `emitted_event_count` and `event_sink` that shadow outer bindings. The `event_sink` on line 223 is set to `None`, discarding the parameter. In the dead code path this is harmless but indicates a copy-paste error from `run_native_agent_loop_v2_deepseek_inner`.

**Lines 230-239 -- Continuation request built with empty resume view**
- **Severity:** HIGH (in dead code)
- **Description:** Line 230 creates `empty_resume_view = ContinuationView::from_legacy_batch(Vec::new())`. Line 232 passes this to `build_native_tool_evidence_continuation_request`, which at line 115-117 of continuation.rs returns `Err("tool evidence continuation requires a non-empty tool batch")`. This would cause an immediate error in the resume flow. The initial resume continuation should use `build_deepseek_anthropic_request_with_tools` (or similar initial-request builder), not an evidence-based continuation.
- **Reference:** OpenClaudeCode's query loop uses `prependUserContext` and sends the system prompt + messages in the initial request, not a tool_evidence continuation (query.ts line 660).
- **Fix:** Replace lines 231-239 with a standard initial-request builder for the resume path.

### Missing Features

1. **No permission decision propagation in resume (lines 175-188):** The `resume_request` type (`NativeAgentLoopV2ResumeRequest`) does not carry `provided_permission_decisions`, so the resume path always uses `PermissionMode::Default` with no pre-approved decisions. OpenClaudeCode preserves the permission context across turns.

2. **No interrupt signal plumbing in resume (lines 260-300):** The resume path passes `&AtomicBool::new(false)` to the stream handler, meaning exit signals are never propagated to a resumed session. OpenClaudeCode uses `toolUseContext.abortController.signal` in query.ts.

3. **No tool manifest regeneration on turn classification change (lines 161-174):** If `TurnRouter::classify` returns a different route than expected, the manifest is not regenerated with the newly classified route.

### Dead Code

- **Lines 142-497:** Entire `run_native_agent_loop_v2_deepseek_resume` function (355 lines of dead code).
- **Lines 222-224:** Declaration of `emitted_event_count` and `event_sink` shadowed variables.

---

## File 2: `native_agent_loop_continuation.rs` (490 lines)

### Bugs Found

**Lines 115-117 -- Guard rejects empty tool batch**
- **Severity:** LOW
- **Description:** `build_native_tool_evidence_continuation_request` returns `Err` when the batch is empty. Callers must guarantee non-empty batches; this guard is correct but the error message could be more descriptive.
- **Fix:** Consider changing to a fallback behavior rather than an error, or document the precondition at the call sites.

**Lines 395-404 -- Error flag injected as JSON suffix rather than protocol-native field**
- **Severity:** MEDIUM
- **Description:** Tool result errors are appended as `\n{"is_error":true}` to the content string. For Anthropic-protocol paths, the API has a native `is_error` boolean field on `tool_result` blocks. This JSON suffix is parsed downstream (line 464 in `deepseek_tool_results_from_messages`) by string matching `content.contains("\"is_error\":true")`, which could false-positive if a tool result legitimately contains that substring.
- **Reference:** OpenClaudeCode uses the native `is_error: true` property on `ToolResultBlockParam` (query.ts line 141).
- **Fix:** Use the protocol-native `is_error` field where available, only falling back to JSON suffix injection for protocols that lack it.

**Lines 455-468 -- `deepseek_tool_results_from_messages` uses `content.contains` for error detection**
- **Severity:** LOW
- **Description:** `is_error: content.contains("\"is_error\":true")` is fragile; any legitimate tool output containing that JSON fragment would be wrongly classified as an error.
- **Fix:** Pass a structured error flag rather than checking content text.

### Design Observations

**Lines 152-175 -- `compacted_prompt_for_model` is a separate code path from normal continuation**
- **Severity:** INFO
- **Description:** This builds a compacted prompt for when context budget is exhausted. The OpenClaudeCode equivalent is `buildPostCompactMessages` in query.ts (line 528), which uses `compactionResult.summaryMessages` instead of constructing new messages from scratch. The Rust approach re-derives the prompt from the original task text, which is cleaner but loses structural fidelity.
- **Reference:** query.ts lines 470-535.

**Lines 344-406 -- `continuation_messages_for_provider_replay` is a complex transformation**
- **Severity:** INFO
- **Description:** This function handles three protocol variants (DeepSeek OpenAI, DeepSeek Anthropic, Qwen) in a single function with conditional logic. While correct, the branching on `openai_style_ids` makes the function harder to follow than separate per-protocol implementations would be.

---

## File 3: `native_agent_loop_util.rs` (1195 lines)

### Bugs Found

**Lines 181-188 -- `compact_text` always allocates**
- **Severity:** LOW
- **Description:** `value.to_string()` on line 183 always allocates a new String, even when no truncation occurs. The function is called on hot paths (stream handling, prompt construction).
- **Fix:** Use `Cow<str>` to avoid allocation in the common no-truncation case:
```rust
pub fn compact_text(value: &str, max_chars: usize) -> (Cow<str>, bool) {
    if value.chars().count() <= max_chars {
        return (Cow::Borrowed(value), false);
    }
    let mut output = value.chars().take(max_chars).collect::<String>();
    output.push_str("\n[truncated]");
    (Cow::Owned(output), true)
}
```

**Lines 386-399 -- `json_escape` allocates a Vec per escape character**
- **Severity:** LOW
- **Description:** Each escape character allocates a `Vec<char>` via `.collect::<Vec<_>>()`, then the outer `.flat_map()` collects all these into a final `String`. For a string with many escapes, this creates many intermediate allocations. A simpler `String::with_capacity` + `push_str` approach would be more efficient.
- **Fix:** Build the output String directly:
```rust
pub fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if other.is_control() => write!(out, "\\u{:04x}", other as u32).unwrap(),
            other => out.push(other),
        }
    }
    out
}
```

**Lines 758-786 -- Naive JSONL parsing with string search**
- **Severity:** MEDIUM
- **Description:** `aggregate_model_usage_from_jsonl` and `extract_json_u64_local` parse JSONL by string-matching `"event_type":"model.stream_completed"` and then extracting numeric values with `find("\"key\":")` patterns. This is fragile: if any string field value contains a matching pattern (e.g., a file preview containing `"prompt_tokens":`), the extraction will produce wrong results.
- **Reference:** OpenClaudeCode does not manually parse JSON in this way; it relies on structured deserialization.
- **Fix:** Parse each line as `serde_json::Value` and extract fields from the structured tree. This is slightly slower but correct.

**Lines 788-793 -- `live_deepseek_endpoint` sets `api_key_env = "PATH"`**
- **Severity:** MEDIUM
- **Description:** Setting `api_key_env = "PATH"` means the endpoint tries to read its API key from the PATH environment variable. This is a testing convenience but is incorrect -- PATH contains colon-separated directory paths, not API keys. If called in a non-test context, it would pass the PATH value as an API key.
- **Fix:** Use a dedicated env var name like `DEEPSEEK_API_KEY` or `RESEARCHCODE_DEEPSEEK_API_KEY`, or gate this behind `#[cfg(test)]`.

**Lines 796-800 -- `live_qwen_endpoint` hardcodes `localhost:8000`**
- **Severity:** LOW
- **Description:** The Qwen endpoint is hardcoded to `http://127.0.0.1:8000`, which is fine for local testing but would fail in CI. Same `api_key_env = "PATH"` issue as above.

### Dead Code / Unused Functions

**Lines 520-555:** `shell_list_command_to_repo_root` -- Checked by grep, only called from `shell_list_intent_to_repo_root` (line 560). Used, not dead.

**Lines 653-713:** `resolve_workspace_write_path` -- This function has no greppable callers in the reviewed files. Needs verification.

### Duplicates

**Lines 386-399 vs session.rs lines 1200-1212:** `json_escape` is duplicated between files. The session.rs version omits control character handling (only handles `\\`, `\"`, `\n`, `\r`, `\t`), while the util.rs version additionally escapes all control characters. This inconsistency could lead to different behavior depending on which function is called.
- **Fix:** Extract `json_escape` into a shared utility or make session.rs delegate to the util.rs version.

### Design Observations

**Lines 987-1076 -- `visible_text_looks_like_transition_statement` is a Rust-specific heuristic**
- **Severity:** INFO
- **Description:** This function implements heuristic detection of "preamble-only" model responses (e.g., "Let me check the codebase..."). OpenClaudeCode does not have this logic because its streaming tool executor naturally handles the transition. The Rust implementation needs this due to architectural differences in how tool calls are detected. The heuristic is well-tested with thorough positive and negative test cases (lines 1078-1195).

---

## File 4: `native_agent_loop_fixtures.rs` (840 lines)

### Bugs Found

**Lines 58, 134, 193, 246, 303, 361, 418, 464, 510, 553, 611, 682 -- Cleanup only on success path**
- **Severity:** MEDIUM
- **Description:** All fixtures use `let _ = fs::remove_dir_all(&root);` which runs unconditionally in the success path but will NOT run if an earlier `?` returns an error. This means failed fixtures leak temporary directories.
- **Fix:** Wrap the test body in a closure, use `std::panic::catch_unwind`, or use a `tempfile` crate to auto-clean.

**Lines 13-14 -- Manual JSON escaping for SSE bodies**
- **Severity:** LOW
- **Description:** Lines 14, 261-268, 318-326 construct SSE bodies by manually escaping HTML content with `.replace('\\', "\\\\").replace('"', "\\\"")`. This only handles backslash and double-quote escapes, not other JSON-required escapes (newlines, control characters). If the test HTML contains `\n`, the generated SSE would be invalid JSON.
- **Fix:** Use `serde_json::to_string` to properly escape the HTML content before embedding in the SSE format.

### Missing Test Scenarios

1. **No Qwen continuation fixture:** There are continuation fixtures for DeepSeek but none for Qwen.
2. **No interrupt/resume roundtrip fixture:** The external resume fixture (lines 616-683) tests the external decision resume, but there is no fixture for the user-interrupt- guidance resume flow.
3. **No concurrent tool execution fixture:** `concurrent_tool_execution: false` is set on ALL fixtures. The concurrent path (annotated as "D5" in code) is completely untested via fixtures.
4. **No compaction fixture:** None of the fixtures trigger the compaction guard path or `build_native_compacted_initial_request`.

### Dead Code

**Lines 198-246 -- `run_scripted_native_agent_loop_v2_tool_error_continuation_fixture`:** Sets `tool_exposure: ReadOnly` but the scripted response contains a `file_write` tool use. The tool would fail (not in manifest) but the fixture tests that specific recovery path. This is used in tests.

---

## File 5: `native_agent_loop_resume.rs` (190 lines)

### Bugs Found

**Lines 107-108 -- Counters initialized to zero, losing accumulated state**
- **Severity:** MEDIUM
- **Description:** `tool_call_count` is initialized to `0` and `model_call_count` to `0`, ignoring any accumulated counts from previous turns. The `NativeAgentLoopResumeRequest` struct does not carry accumulated counts, unlike `NativeAgentLoopV2ResumeRequest` which has `accumulated_tool_call_count` and `accumulated_model_call_count` (native_agent_loop.rs lines 189-190).
- **Reference:** OpenClaudeCode tracks `turnCount` and passes it through state (query.ts line 320).
- **Fix:** Add `accumulated_tool_call_count` and `accumulated_model_call_count` to `NativeAgentLoopResumeRequest`, or derive them from the event log.

**Lines 180-188 -- Loop ends after single tool execution, no continuation**
- **Severity:** HIGH
- **Description:** After executing the pending tool, the function immediately transitions to `Reviewing` then `Completed` and returns. Unlike OpenClaudeCode, which continues the multi-turn loop after tool execution (query.ts line 1715 transitions to `next_turn`), this Rust implementation stops after a single tool. If the model had additional tool calls queued, they are dropped.
- **Reference:** query.ts lines 1715-1727 -- after tool results are accumulated, the loop continues to `next_turn`.
- **Fix:** After executing the pending tool, return the result and let the caller continue the loop, or implement in-function continuation similar to `run_native_agent_loop_v2_deepseek_inner`.

**Lines 122-159 -- Completed-tool detection duplicates execution logic**
- **Severity:** LOW
- **Description:** The logic to check if a tool was "already completed" (`replayed_tool_completion_state`) duplicates what the inner loop's batch guard already handles. If a tool is replayed, the inner loop would detect the duplicate via `ObservationCache` or `ToolBatchGuardAction`.

---

## File 6: `session.rs` (1812 lines)

### Duplicates

**Lines 1200-1212 -- `json_escape` duplicate of util.rs lines 386-399**
- **Severity:** LOW
- **Description:** Same function name, same purpose, but different implementations (session.rs version misses control character escaping). This is a maintenance risk if only one version is updated.

**Lines 1311-1329 -- `json_string_local` and `escape_json_local` duplicate of util.rs `json_string` (lines 944-957)**
- **Severity:** LOW
- **Description:** These are private copies of the same JSON serialization logic used in event recording.

### Design Observations

**Lines 217-248 -- `begin_interactive_turn` bypasses `can_transition`**
- **Severity:** LOW
- **Description:** When the session is in `Completed`/`Failed`/`Cancelled` state, `begin_interactive_turn` manually sets `self.state = AgentState::Executing` and emits a `session.forced_transition` event, bypassing the state machine validator. While intentional (these are recovery paths), it means the transition rules in `state.rs` are not the sole authority on state changes.

---

## File 7: `state.rs` (147 lines)

No bugs found. The state transition matrix is comprehensive and well-defined. The `can_transition` function correctly encodes the FSM. Test coverage is adequate (6 tests covering expected paths, terminal states, cancellations, and safety boundaries).

---

## Summary

| Category | Count |
|---|---|
| HIGH severity bugs | 3 (event_sink shadowing, resume empty batch, loop stops after single tool) |
| MEDIUM severity bugs | 6 (naive JSON parsing, dup json_escape, counter reset, API key abuse, error flag injection, cleanup on success only) |
| LOW severity bugs | 6 (unnecessary allocation, Vec-per-escape, hardcoded URL, manual JSON escaping in fixtures, fragile error detection, transition bypass) |
| Missing features | 4 (permission propagation, interrupt plumbing, Qwen continuation fixture, concurrent execution fixture) |
| Dead code (lines) | ~355 lines (resume function in entrypoints) |
| Duplicate functions | 3 (`json_escape`, `json_string_local`, `escape_json_local`) |

### Overall Assessment per File

- **entrypoints.rs:** Structurally sound API surface (3 thin wrappers) but the dead `resume` function (~70% of the file) should be removed or gated. The `event_sink` shadowing bug is the most impactful issue.
- **continuation.rs:** Well-structured protocol multiplexing. The evidence-based fallback path vs. provider-native tool_result replay strategy is sound. Minor concerns with error flag injection.
- **util.rs:** Broad collection of helpers; most are used. `aggregate_model_usage_from_jsonl` parsing is the most impactful concern. String manipulation could be more allocation-efficient.
- **fixtures.rs:** Covers the main scenarios well but gaps exist in compaction, concurrent execution, and Qwen continuation. Temporary directory cleanup on failure is the main operational concern.
- **resume.rs:** The biggest architectural concern: it executes a single tool and stops, diverging from the OpenClaudeCode multi-turn continuation model. Accumulated counter reset is a correctness issue.
- **session.rs:** Solid event-log-based session management. Duplicate JSON utilities are a maintenance concern. State transition bypass is documented but worth monitoring.
- **state.rs:** Clean, well-tested FSM with no issues found.