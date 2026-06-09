I now have a thorough understanding of all the files. Let me compile the architecture review report.

---

# Architecture Review Report: Model I/O, Streaming, and Prompt Construction Pipeline

## 1. Streaming Architecture

### OpenClaudeCode Pattern
- Single `callModel()` function returns `AsyncGenerator<StreamEvent>` (query.ts line 659)
- The Anthropic SDK handles all stream parsing into typed `BetaContentBlock` objects
- `StreamingToolExecutor` class executes tools **in parallel during streaming** (query.ts line 562-568)
- Content blocks are yielded as SDK-typed events (`tool_use`, `text`, `thinking`)
- No raw SSE parsing anywhere; the SDK is the single normalization boundary

### deep-code Pattern
- Two separate stream processors: `StreamProcessor` (DeepSeek, stream_processor.rs) and `QwenStreamProcessor` (qwen stream_processor.rs)
- Raw SSE string parsing via hand-written JSON extractors (`parse_deepseek_sse_line_all` in stream.rs, `parse_qwen_sse_line_all` in qwen_stream.rs)
- `LiveHttpStreamEvent` enum (live_http_transport.rs:33-62) serves as the intermediate normalization format
- Event flow: raw SSE lines -> `LiveHttpStreamEvent` -> `StreamProcessor.ingest()` -> `StreamProcessorOutput`
- Tool calls are completed only after the full stream finishes, not during (model_io.rs:610, `stream_processor.complete_stream()`)

### Critical Gap: The StreamProcessor and QwenStreamProcessor are nearly identical

**File**: `native_profile/deepseek/stream_processor.rs` (lines 1-514) and `native_profile/qwen/stream_processor.rs` (lines 1-264)

Both have:
- A `ToolCallPipeline` field
- `ingest_chunk()` accepting SSE lines
- `take_pending_content()` / `take_pending_reasoning_chars()`
- `complete_stream()` scanning for content tool call candidates
- Almost identical event enums (`StreamProcessorEvent` vs `QwenStreamProcessorEvent`)

The DeepSeek version adds DSML filtering and content block tracking that Qwen does not need, but the overall architecture is duplicated. The `ingest_visible_delta()`, `ingest_tool_event()`, and output handling logic should be a shared trait/struct with provider-specific plugins for filtering.

**Severity: HIGH** -- maintenance burden, testing duplication, risk of asymmetric bug fixes.

### Critical Gap: No streaming tool execution

OpenClaudeCode's `StreamingToolExecutor` (query.ts line 562) lets tools run **while** the model is still streaming tokens. deep-code collects all tool calls first, then executes them after `complete_stream()` (model_io.rs:610). This means deep-code cannot overlap tool execution with model response delivery. For slow tools (shell commands, network calls), this adds sequential latency of `stream_time + tool_time` instead of `max(stream_time, tool_time)`.

**Severity: HIGH** -- impacts latency for multi-tool turns.

---

## 2. Prompt Construction

### OpenClaudeCode Pattern
- `buildEffectiveSystemPrompt()` (systemPrompt.ts:41-123): layered priority model with 7 levels (override > coordinator > agent > custom > default + proactive + append)
- System prompts are arrays of strings, composed via `asSystemPrompt()` tagged type
- `appendSystemContext()` and `prependUserContext()` add environment context
- Separate concern for each prompt layer

### deep-code Pattern
- `native_loop_system_prompt()` (prompt.rs:231-300): single function, selects between DeepSeek/Qwen templates
- `native_loop_prompt_with_turn_directives()` (prompt.rs:302-322): appends runtime directives
- Uses keyword-based intent detection strings (prompt.rs:335-464) for file generation, write intent, tool inventory
- DeepSeek cache prefix zones embedded in the prompt function (prompt.rs:282-298)

### Critical Gap: No prompt layering system

deep-code has a flat prompt: one system prompt string per family, with directives bolted on. There is no equivalent to OpenClaudeCode's:
- Override system prompt (user-provided replacement)
- Agent-specific prompts (mainThreadAgentDefinition)
- Proactive mode prompt variant
- Append system prompt

**Severity: MEDIUM** -- the layered approach enables agent composition and custom instructions. Without it, extending to coordinator/agent mode would require refactoring `native_loop_system_prompt`.

### Key Difference: Intent detection via keyword matching

deep-code uses hardcoded Chinese/English keyword lists (prompt.rs:337-463) to detect user intent (file generation, write requests, tool inventory). OpenClaudeCode does not do this -- it relies on the model's understanding plus the `TurnRoute` enum. This keyword-based approach is fragile and won't scale to new languages or phrasings.

**Severity: MEDIUM** -- fragile, but functional for the current scope.

---

## 3. Provider Abstraction

### OpenClaudeCode Pattern
- Single provider (Anthropic) with Bedrock/Vertex variants handled via SDK configuration
- `getAPIProvider()` returns `'anthropic' | 'bedrock' | 'vertex'`
- Provider differences isolated to header configuration in `claude.ts`

### deep-code Pattern
- `NativeModelFamily` enum: `DeepSeek`, `Qwen`
- `NativeProviderEndpoint` struct with `protocol` field: `"anthropic_compatible"`, `"openai_compatible"`, `"custom"`
- Six request builder functions in `live_model_request.rs`:
  - `build_deepseek_anthropic_request[_with_tools]`
  - `build_deepseek_openai_request[_with_tools]`
  - `build_qwen_openai_request[_with_tools]`
  - Plus 3 tool_result variants
- `ModelAdapter` trait with per-family implementations

### Critical Gap: No unified provider abstraction trait

The request builders are free functions, not methods on a trait. Each builder has its own validation logic, URL construction, and body formatting. Adding a third provider would require N x M new builder functions. OpenClaudeCode's SDK-based approach handles provider differences through configuration objects.

**Severity: HIGH** -- combinatorial explosion of builder functions. Consider: each new protocol (e.g., Gemini-compatible) adds 2+ new builders, and each new result-type variant mutliplies that.

### Gap: Manual JSON body construction instead of serde

**File**: `live_model_request.rs`, lines 352-370 (`build_deepseek_anthropic_multi_tool_result_request_with_thinking`)

```rust
let body_json = format!(
    "{{\"model\":\"{}\",\"max_tokens\":{},\"stream\":{},\"system\":\"{}\",...
```

Request bodies are built with `format!()` and manual `escape()` calls throughout (lines 644-654, 713-722, 732-773, etc.). This is error-prone (missing commas produce invalid JSON at runtime) and bypasses Rust's type safety. OpenClaudeCode uses typed SDK objects that serialize safely.

**Severity: HIGH** -- security risk (JSON injection via unescaped user content) and maintainability.

### Over-engineering: Dual-protocol fallback (DualProtocolFallback)

**File**: `native_agent_loop_model_io.rs`, lines 692-707

deep-code implements automatic fallback from Anthropic-style to OpenAI-style protocol when a 400 error occurs. While this is a DeepSeek-specific concern (the API sometimes rejects Anthropic-format requests), the fallback logic is embedded directly in the streaming send function (`send_with_live_visible_stream_events`, line 309). This adds ~50 lines of retry-loop logic mixed with streaming observer setup, making the function 440 lines long.

**Severity: MEDIUM** -- protocol fallback belongs in a middleware or adapter layer, not the core send function.

---

## 4. Response Adapter

### OpenClaudeCode Pattern
- `normalizeContentFromAPI()` converts SDK-typed content blocks into internal `Message` types
- `createAssistantAPIErrorMessage()` for streaming errors
- Thinking blocks handled natively by the SDK

### deep-code Pattern
- `provider_response_adapter.rs`: two input types, two output paths
  - `NativeProviderStreamInput` for SSE stream recording (line 219)
  - `NativeProviderResponseInput` for non-stream response recording (line 41)
- `NativeProviderStreamResult` carries both visible content and `volatile_reasoning_content` (line 73)
- Per-provider branching in `record_native_provider_stream_inner` (line 100)

### Critical Gap: Volatile reasoning is passed as raw strings through the result

**File**: `provider_response_adapter.rs`, line 73 and line 156-157

```rust
pub volatile_reasoning_content: Option<String>,
```

The reasoning content is explicitly marked "volatile" in comments (must not be logged) but there is no type-system enforcement. It's a plain `String` that could accidentally be serialized. OpenClaudeCode keeps thinking blocks inside typed `AssistantMessage.content` arrays where the SDK enforces the contract.

**Severity: MEDIUM** -- a newtype wrapper (e.g., `VolatileReasoning(String)`) with `Debug`/`Display` suppression would prevent accidental logging.

---

## 5. HTTP Transport

### OpenClaudeCode Pattern
- Single transport: Anthropic SDK HTTP client, created via `getAnthropicClient()` (claude.ts)
- SDK handles retries, timeouts, authentication
- `dumpPromptsFetch` wrapper for request logging (query.ts line 588)

### deep-code Pattern
- `LiveHttpTransport` trait (live_http_transport.rs:64-75) with `send()` and `send_with_stream_observer()`
- Three implementations:
  1. `RecordedLiveHttpTransport` -- test fixture
  2. `ScriptedLiveHttpTransport` -- test fixture with SSE simulation
  3. `PythonSidecarLiveHttpTransport` -- real HTTP via subprocess
- Two separate modules: `live_http_transport.rs` (834 lines) and `sidecar_http_transport.rs` (643 lines)

### Gap: Why does deep-code need BOTH live_http_transport.rs AND sidecar_http_transport.rs?

**Answer**: `live_http_transport.rs` defines the trait and provides the `run_live_model_http_once()` orchestration function (line 270). `sidecar_http_transport.rs` implements the trait using Python subprocess (line 101). The separation is architecturally clean -- trait + orchestration layer vs implementation. However, the `run_live_model_http_once_inner()` function at live_http_transport.rs:289 has its own retry/error logic that partially duplicates `send_with_live_visible_stream_events()` in model_io.rs:309.

**Severity: LOW** -- the separation is correct, but there is retry logic duplication between the two orchestration entry points.

### Architectural Question: Python sidecar for HTTP

The design delegates actual HTTP I/O to a Python subprocess (`provider_http_sidecar.py`) to keep the Rust kernel free of network concerns. This adds:
- Subprocess management overhead (spawn, stdin/stdout pipe, temp files for response bodies)
- Streaming via stdout line-by-line JSON events with idle/total timeout management (sidecar_http_transport.rs:261-443)
- Temp directory cleanup in error paths

OpenClaudeCode makes HTTP calls in-process via the SDK. The sidecar pattern is an intentional architectural choice (security boundary), not over-engineering.

**Severity: N/A** -- this is an intentional design decision for security isolation.

---

## 6. Token Management

### OpenClaudeCode Pattern
- `countTokensWithAPI()` (tokenEstimation.ts:124) -- calls Anthropic's `countTokens` API endpoint
- `tokenCountWithEstimation()` in tokens.ts -- uses API when available, falls back to estimation
- `calculateTokenWarningState()` for blocking/warning thresholds
- `checkTokenBudget()` for per-turn token budget with augmentation messages (query.ts:1308)
- `ESCALATED_MAX_TOKENS` cap escalation from 8k to 64k on `max_output_tokens` errors (query.ts:1194)
- Compaction tracking: `AutoCompactTrackingState`, `snipTokensFreed`

### deep-code Pattern
- `estimate_tokens()` (native_turn_controller.rs line reference) -- character-based estimation only
- `ContextBudget` (context_budget.rs) with scaffold/dynamic/protected reserve tiers
- `DEEPSEEK_REASONING_REPLAY_BUDGET_TOKENS` constant for reasoning replay
- `guard_native_loop_prepared_request()` checks budget before sending (model_io.rs:819)
- No API-based token counting

### Critical Gap: No API-based token counting

deep-code estimates tokens via simple heuristics (likely char/4 or similar). OpenClaudeCode's `countTokens` API provides exact token counts from the provider's tokenizer. For DeepSeek models with non-standard tokenization (especially for Chinese text), character-based estimates can be off by 30-50%. This means context budget guards can be dangerously inaccurate.

**Severity: HIGH** -- DeepSeek and Qwen both have token counting endpoints (since they're OpenAI-compatible at the API level). Not using them means context overflow errors (400/413) will happen at runtime instead of being caught during budget guarding.

### Critical Gap: No max_tokens escalation pattern

OpenClaudeCode has a 3-tier max_tokens recovery:
1. First hit: escalate from ~8k to 64k automatically (query.ts:1199)
2. Second hit: inject "continue" message and retry (max 3 times, query.ts:1223)
3. Exhausted: surface error to user

deep-code has `ErrorRecoveryState` (model_io.rs:324, 404, 577) with `max_tokens.escalate()` and `max_tokens.record_retry()`, but the escalation logic itself (what value to escalate to, how many times) is opaque. The `MAX_TOKENS_LIMIT` in live_model_request.rs:12 is a hard cap (128k), not an escalation target.

**Severity: HIGH** -- without proper escalation, long tool-result continuations will hit `max_tokens` limits and produce truncated responses with no recovery path.

### Gap: No token budget continuation system

OpenClaudeCode has `checkTokenBudget()` (query.ts:1308) that injects nudge messages when approaching budget limits. deep-code has no equivalent.

**Severity: MEDIUM** -- important for long-running autonomous agent sessions, but may not be needed in the current single-turn-tool-loop design.

---

## 7. Model Request Building

### Comparison Summary

| Aspect | OpenClaudeCode | deep-code |
|--------|---------------|-----------|
| Pattern | SDK typed params | Manual JSON strings |
| Validation | SDK type system | Custom `validate_*` functions |
| Tool schemas | `toolToAPISchema()` | Raw JSON strings passed through |
| Message history | SDK handles | Manual `project_*_history_messages()` |
| Cache control | SDK-managed ephemeral | Cache prefix zones injected into system prompt |
| Thinking config | `thinkingConfig` object | Inline `thinking`/`reasoning_effort` fields |

### Critical Gap: History projection relies on runtime text markers

**File**: `live_model_request.rs`, lines 973-986 (`split_legacy_openai_history_section`)

deep-code parses conversation history from a specially-marked section in the user prompt text:

```rust
let marker = "\n\n# Conversation History (OpenAI JSON)\n";
```

This is a string-based protocol between prompt construction and request building. If the marker format changes or the user sends text containing that marker, it breaks. OpenClaudeCode keeps history as typed `Message[]` arrays throughout, never relying on text markers.

**Severity: HIGH** -- fragile coupling between prompt construction and request building.

---

## 8. Error Handling

### OpenClaudeCode Pattern
- SDK-provided error types: `APIError`, `APIConnectionTimeoutError`, `APIUserAbortError`
- `FallbackTriggeredError` for model fallback to backup model (query.ts:893)
- Rate limit detection via `extractQuotaStatusFromHeaders()`
- Multi-tier recovery: collapse drain -> reactive compact -> model fallback -> surface error
- `isPromptTooLongMessage()` detection and withholding

### deep-code Pattern
- `ErrorRecoveryState` struct (model_io.rs)
- Retry on transient HTTP statuses: 408, 409, 429, 500, 502, 503, 504 (max 6, model_io.rs:753)
- Jitter-based delay: `native_loop_retry_delay_ms()` (model_io.rs:758)
- `DualProtocolFallback` on 400 (switch Anthropic->OpenAI protocol)
- No rate limit header parsing
- No prompt-too-long recovery

### Critical Gap: No prompt-too-long recovery loop

OpenClaudeCode has a sophisticated recovery chain for 413/context-overflow errors:
1. Context collapse drain (release queued collapses)
2. Reactive compact (force compaction)
3. Model fallback (switch to fallback model)
4. Surface error to user

deep-code's only response to context overflow is the `guard_native_loop_prepared_request()` check before sending (model_io.rs:819). If the guard passes but the API still rejects with 400/413, there is no recovery -- the error surfaces immediately.

**Severity: CRITICAL** -- for long conversations, this means the agent will fail irrecoverably once context is exhausted.

### Critical Gap: No rate limit handling beyond retry

deep-code retries on 429 but does not:
- Parse `Retry-After` headers to respect server-specified backoff
- Parse quota headers to detect hitting limits vs transient throttling
- Have exponential backoff (base delay doubles but is constant per-attempt)
- Emit rate limit budget events for monitoring

**Severity: CRITICAL** -- in production, undifferentiated retry on 429 without respecting Retry-After will cause thundering-herd retries and potentially get the API key rate-limited harder.

### Critical Gap: No model fallback

OpenClaudeCode can switch to a fallback model when the primary model returns errors (claude.ts `FallbackTriggeredError` -> query.ts line 893). deep-code only switches protocols (Anthropic <-> OpenAI) for the same model family, never switches to a different model.

**Severity: MEDIUM** -- less critical for single-model scenarios but blocks multi-model routing.

---

## 9. Summary of All Gaps by Severity

### CRITICAL
1. **No prompt-too-long recovery loop** (model_io.rs) -- agent fails irrecoverably on context overflow
2. **No Retry-After/quota header parsing** (model_io.rs:753) -- 429 retry without server guidance

### HIGH
3. **No API-based token counting** (native_turn_controller.rs) -- estimates can be 30-50% off for non-English text
4. **No max_tokens escalation pattern** (model_io.rs) -- truncated responses with no recovery
5. **Manual JSON body construction** (live_model_request.rs) -- security risk, maintainability
6. **No unified provider abstraction trait** (live_model_request.rs) -- combinatorial builder explosion
7. **No streaming tool execution** (model_io.rs:610) -- sequential tool latency
8. **Duplicate stream processors** (stream_processor.rs + qwen stream_processor.rs) -- maintenance burden
9. **Fragile history projection via text markers** (live_model_request.rs:973)

### MEDIUM
10. **No prompt layering system** (prompt.rs) -- flat prompt vs layered priority
11. **Fragile keyword-based intent detection** (prompt.rs:335-464)
12. **Volatile reasoning lacks type-safety** (provider_response_adapter.rs:73)
13. **Dual-protocol fallback embedded in send function** (model_io.rs:309)
14. **No token budget continuation system** (no equivalent to `checkTokenBudget`)
15. **Retry logic duplicated between orchestration layers** (model_io.rs:309 vs live_http_transport.rs:289)

### LOW
16. **No model fallback to different model** (only protocol switching)
17. **Python sidecar adds deployment complexity** (intentional design choice)

---

## 10. Specific Recommendations

### R1: Unify stream processors (stream_processor.rs, qwen/stream_processor.rs)
Create a shared `StreamProcessorCore` that handles common state (pending content, thinking chars, tool pipeline). Provider-specific adapters (DSML filter, Anthropic content block tracking) become plugins. Target: delete `QwenStreamProcessor` and make `StreamProcessor` work for both via a provider mode enum.

### R2: Use serde for request body construction (live_model_request.rs)
Replace all `format!(...)` JSON string building with serde structs:
```rust
#[derive(Serialize)]
struct AnthropicRequestBody { model, max_tokens, stream, system, tools, messages }
```
This eliminates JSON injection risks and makes the builders testable via `serde_json::from_str`.

### R3: Add API token counting (native_turn_controller.rs)
Call DeepSeek/Qwen token counting endpoints (OpenAI-compatible `/chat/completions` with `max_tokens=0` or their dedicated endpoints). Cache results per message hash. Use as primary source, fall back to estimation.

### R4: Implement prompt-too-long recovery (model_io.rs)
Add a recovery loop that on 400/413:
1. Triggers compaction (drop old tool results, summarize)
2. If compaction fails, escalate `max_tokens` downward
3. Surface error as last resort

### R5: Add Retry-After header handling (model_io.rs, sidecar_http_transport.rs)
Parse `Retry-After` from 429 response headers. Use the server-specified delay if present, fall back to jittered exponential backoff. The sidecar already receives status -- the header just needs to be forwarded to Rust.

### R6: Create a provider trait (live_model_request.rs)
```rust
trait NativeModelRequestBuilder {
    fn build_request(&self, messages: &[Message], opts: &RequestOpts) -> PreparedModelHttpRequest;
    fn build_tool_result_request(&self, ...) -> PreparedModelHttpRequest;
}
```
Implement for `DeepSeekAnthropic`, `DeepSeekOpenAI`, `QwenOpenAI`.

### R7: Add reactive compaction equivalent
When the model approaches context limits (detected via guard or API 400), automatically trigger a compaction step: drop old tool result content, summarize conversation, retry. This is OpenClaudeCode's `reactiveCompact` pattern.

### R8: Remove text-marker-based history projection (live_model_request.rs)
Replace `split_legacy_openai_history_section()` with typed `ConversationMessage` arrays passed directly to request builders. The prompt construction layer should never encode structured data in text markers.

### R9: Consider removing Python sidecar for production (sidecar_http_transport.rs)
For production deployment, implement a native Rust HTTP transport using `reqwest` (the crate is already listed as a dependency per the recent commit `chore(runtime): add tokio reqwest transport deps`). Keep the sidecar as a test/development fallback.