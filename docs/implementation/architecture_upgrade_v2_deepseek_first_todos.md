# Architecture Upgrade V2 DeepSeek-First: Executable TODOs

This TODO plan is derived from:

- `.claude/worktrees/gracious-dubinsky-f08f39/docs/engineering/architecture_upgrade_v2_deepseek_first.md`
- `docs/engineering/architecture_upgrade_from_claude_code.md`
- `docs/implementation/architecture_upgrade_from_claude_code_todos.md`
- `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md`
- current main workspace runtime and GUI state

The source V2 document is directionally correct: V1 makes the runtime less
fragile; V2 makes DeepSeek/Qwen native behavior part of the agent kernel instead
of a provider-side wrapper. This file converts that architecture into an
implementation backlog.

## 0. Execution Contract

Goal:

```text
Upgrade the current Claude-Code-aligned runtime into a DeepSeek/Qwen-first agent
kernel where message structure, hooks, transcript, tool identity, reasoning,
DSML fallback, context caching, retry policy, and GUI-visible events are coherent
kernel/runtime contracts.
```

Allowed scope:

- `crates/kernel`
- `crates/runtime`
- `crates/cli`
- root `desktop/`
- `scripts`
- `docs`
- focused tests, fixtures, and replay harnesses

Denied without explicit approval:

- adding async dependencies such as `tokio`, `tokio-util`, `futures`, or
  `parking_lot`;
- changing root `AGENTS.md`;
- destructive git operations;
- network/package-install actions;
- reading secrets, SSH keys, `.env`, or private credentials;
- genericizing DeepSeek/Qwen native behavior into compatible-provider config.

Dependency rule:

- Kernel type additions are safe to implement first.
- Runtime wiring must keep existing sync bounded-blocking architecture unless
  the async ADR is explicitly reopened and approved.
- Provider wire-format changes must preserve current compatible-provider
  behavior as baseline, but compatible providers cannot override native
  DeepSeek/Qwen prompts, parsers, context policy, tool policy, or eval gates.

Completion gate:

- focused tests for every coherent slice;
- `cargo fmt --all`;
- `cargo test -p researchcode-kernel --lib` when kernel changes;
- `cargo test -p researchcode-runtime --lib` when runtime changes;
- `cargo check --manifest-path desktop/src-tauri/Cargo.toml` when Tauri changes;
- `npm run build` in `desktop/` when GUI changes;
- `python3 scripts/check_all.py` before claiming the whole V2 upgrade complete.

Stop conditions:

- a denied path/tool is required;
- new dependency approval is required;
- a native DeepSeek/Qwen optimization conflicts with an existing authoritative
  planning document;
- an eval gate shows DeepSeek/Qwen wrong-tool execution regression;
- a schema/event contract change would break GUI replay without migration.

Current implementation status, 2026-05-10:

- [x] V2 DeepSeek-first kernel/runtime backlog is implemented through Phase 12.
- [x] Kernel message, hook, transcript, subagent, native model capability, and
      tool-schema contracts are present and tested.
- [x] Runtime integration includes native profile routing, bounded hook
      dispatcher, transcript store, DeepSeek reasoning/DSML policy events,
      cache planner events, bounded live HTTP retry/fallback/repair, permission
      mode/rule parity, and isolated subagent events.
- [x] GUI/TUI observability renders recovery, reasoning, cache, subagent, and
      permission-resume lifecycle events without exposing raw reasoning.
- [x] Final verification passed:
  - `cargo fmt --all`
  - `cargo test -p researchcode-kernel --lib`
  - `cargo test -p researchcode-runtime --lib`
  - `cargo test -p researchcode-cli`
  - `cargo check --manifest-path desktop/src-tauri/Cargo.toml`
  - `npm run build` in `desktop/`
  - `npm run tauri:dev` startup smoke in `desktop/`
  - `python3 scripts/claudecode_gap_check.py`
  - `python3 scripts/check_all.py`

## 1. Source Corrections And Non-Negotiable Invariants

The V2 source document corrects several earlier overstatements. These are
implementation invariants, not optional notes.

- [x] Preserve the existing provider-specific block evidence in
      `crates/runtime/src/live_model_request.rs`; do not claim message support is
      absent. The real gap is no provider-agnostic kernel `Message`.
- [x] Preserve existing `DeepSeekAnthropicToolUseBlock`,
      `DeepSeekAnthropicToolResultBlock`, `QwenOpenAiToolCallBlock`,
      `DeepSeekOpenAiToolCallBlock`, and streaming provider id assembly until
      the unified kernel message path fully replaces them.
- [x] Preserve DeepSeek reasoning dual-track semantics:
      sanitized reasoning may persist; raw volatile reasoning must not persist
      across sessions.
- [x] Preserve DSML fallback as a DeepSeek-native recovery path, not as a generic
      fallback for all providers.
- [x] Keep root `desktop/` as the product GUI path.
- [x] Keep V1 approval, provider-id, timeout, GUI feedback, and event-replay
      fixes as prerequisites for V2.

Acceptance:

- [x] No V2 task deletes a DeepSeek/Qwen native feature before an equivalent
      kernel-level replacement exists.
- [x] Every migration step has rollback notes and focused tests.

## 2. Coverage Matrix

Every row must have implementation tasks, tests, and acceptance evidence before
the V2 plan can be called done.

| Source Section | Target Phase | Status |
|---|---|---|
| title and V2 positioning: beyond Open-ClaudeCode, DeepSeek first-class | Execution contract, engineering entry | implemented |
| 0.1 Message model exists but lacks provider-agnostic skeleton | Phase 3 | implemented |
| 0.2 Hooks do not exist | Phase 4 | implemented |
| 0.3 DeepSeek first-class features already exist | Phase 1, 6, 7, 8, 9 | implemented |
| 1 V2 upgrade goals: L1 kernel, L2 runtime, L3 DeepSeek first-class | Phase 2-12 | implemented |
| 2.1 `kernel/message.rs` | Phase 3 | implemented |
| 2.2 `kernel/hooks.rs` | Phase 4 | implemented |
| 2.3 `kernel/transcript.rs` | Phase 5 | implemented |
| 2.4 `kernel/subagent.rs` | Phase 10 | implemented |
| 3 L2 runtime integration based on V1 plus V2 increments | V1 dependency handoff, Phase 4-8 | implemented |
| 3.1 Runtime unified message wiring | Phase 6 | implemented |
| 3.2 Hook dispatcher wiring | Phase 4, 7 | implemented |
| 3.3 Transcript persistence | Phase 5 | implemented |
| 3.4 Subagent isolation | Phase 10 | implemented |
| 4.1 DeepSeek variant routing | Phase 2 | implemented |
| 4.2 Protocol fallback | Phase 8 | implemented |
| 4.3 Reasoning budget and folding | Phase 7 | implemented |
| 4.4 Context caching breakpoints | Phase 9 | implemented |
| 4.5 DSML/native smart switching | Phase 7 | implemented |
| 4.6 DeepSeek error retry table | Phase 8 | implemented |
| 5 Open-ClaudeCode parallel features | Phase 11 | implemented |
| 6 Existing advantages to preserve | Phase 1, final gate | implemented |
| 7 roadmap and sequencing | V1 dependency handoff, implementation order | implemented |
| 8 V2 acceptance checklist | Final verification matrix | implemented |
| 9 one-sentence architecture summary | Execution contract, engineering entry | implemented |
| appendix A source evidence | Phase 1 audit and evidence refresh | implemented |

### V1 Dependency Handoff Gate

The V2 source roadmap references V1 work such as async migration, facade
`RwLock`, push channels, timeout, error classification, approval fixes, and
tool-use id preservation. In the current main workspace those are not all the
same kind of work:

- V1 approval, provider id, GUI feedback, timeout, and event-replay fixes are
  prerequisites for V2 runtime rewrites.
- Full async migration, `RwLock`, and `Notify`/push-channel changes are governed
  by `docs/engineering/adr_sync_runtime_bounded_blocking_vs_tokio.md` and must
  not be added silently inside V2.
- V2 phases should use the current bounded-blocking runtime unless the async ADR
  is explicitly reopened and approved.

Tasks:

- [x] Before Phase 6 runtime message wiring, verify the V1 TODO progress ledger
      in `docs/implementation/architecture_upgrade_from_claude_code_todos.md`.
- [x] Block Phase 6 if provider tool-use id preservation or true approval resume
      has regressed.
- [x] Block Phase 8 retry/protocol fallback if the existing error/event taxonomy
      cannot represent bounded retry attempts.
- [x] If async/RwLock/Notify is required for a V2 task, stop and update the ADR
      instead of sneaking dependencies into the implementation.

Acceptance:

- [x] The V2 plan does not depend on unverified V1 claims.
- [x] Async-related source-roadmap items are either explicitly approved or
      explicitly deferred.

## 3. Phase 1: Baseline Audit And Preservation Fixtures

Purpose:

Before introducing new kernel abstractions, freeze the features we must not
lose: DeepSeek reasoning, DSML fallback, streaming tool id assembly,
artifact-backed tool output, context budget, and ToolSpec richness.

Tasks:

- [x] Add or update an audit document section listing current source anchors for:
  - DeepSeek reasoning sanitized/raw split;
  - reasoning replay decision;
  - prompt cache stats parsing;
  - DSML fallback parser;
  - streaming tool call assembler provider id;
  - artifact store tool-result references;
  - context budget guard;
  - ToolSpec metadata fields.
- [x] Refresh appendix-style evidence against current main because the V2 source
      line numbers came from a worktree snapshot and may drift.
- [x] Add preservation tests where missing:
  - sanitized reasoning persists while raw volatile reasoning does not;
  - DSML fallback parses a tool call when native tool call is absent;
  - streaming partial JSON preserves provider tool id;
  - artifact-backed large tool result is referenced, not dumped into context.
- [x] Add a fixture named around the V2 promise, for example
      `deepseek_first_preserves_native_primitives`.
- [x] Mark each preservation test as a blocker for later migration phases.

Tests:

- [x] `cargo test -p researchcode-runtime --lib deepseek`
- [x] focused DSML parser tests
- [x] focused streaming assembler tests

Acceptance:

- [x] Existing DeepSeek advantages are executable tests, not just document
      claims.
- [x] Later phases can refactor through these tests without guessing.

## 4. Phase 2: Native Model Variant And Capability Matrix

Purpose:

Stop treating `DeepSeek` as one flat family. Kernel/runtime policy must know the
difference between V3, V3.1, V3.2-Exp, R1, and Coder-class models.

Tasks:

- [x] Extend the kernel model layer with native variants:
  - `DeepSeekVariant::{V3, V31, V32Exp, R1, CoderV2}`;
  - `QwenVariant` placeholders for the existing Qwen/Qwen3.6 line.
- [x] Add `DeepSeekCapabilities`:
  - native tool-calling reliability;
  - reasoning support;
  - max context tokens;
  - preferred protocol;
  - context caching support;
  - fill-in-the-middle support.
- [x] Add `ToolCallingReliability::{Stable, Unstable, NotRecommended}` with a
      DSML fallback flag for unstable variants.
- [x] Update `ProviderConfig` / `ModelAliasMapping` resolution so aliases can
      map to a native variant without making compatible providers native.
- [x] Thread capabilities into runtime request planning without changing provider
      HTTP serialization yet.
- [x] Add events for resolved native profile:
  - `model.native_profile.resolved`;
  - `model.native_capabilities.resolved`.

Tests:

- [x] alias resolution maps a DeepSeek model string to the expected variant;
- [x] compatible OpenAI/Claude provider remains compatible-only;
- [x] R1 capability enables reasoning and discourages native tool calling;
- [x] V3 unstable capability enables DSML fallback policy.

Acceptance:

- [x] Runtime can choose policy from native capabilities rather than string
      matching scattered through the loop.
- [x] Compatible providers cannot enter native eval promotion accidentally.

## 5. Phase 3: Kernel Message Model

Purpose:

Replace string-only message assembly with a provider-agnostic message skeleton
that can carry text, reasoning, tool use, tool result, images, and cache control.

Tasks:

- [x] Add `crates/kernel/src/message.rs`.
- [x] Define:
  - `MessageRole::{System, User, Assistant, Tool}`;
  - `ContentBlock::{Text, Reasoning, ToolUse, ToolResult, Image, CacheControl}`;
  - `Message`;
  - `StopReason::{EndTurn, ToolUse, MaxTokens, StopSequence, Refusal,
    ReasoningExhausted}`.
- [x] Make `Reasoning` content explicitly carry:
  - sanitized text;
  - optional raw volatile text;
  - optional token count;
  - optional signature.
- [x] Make `ToolUse.id` and `ToolResult.tool_use_id` use provider ids when
      provided.
- [x] Add conversion helpers from current runtime provider-specific blocks into
      kernel messages.
- [x] Add non-lossy serializer tests for:
  - DeepSeek Anthropic-compatible;
  - DeepSeek OpenAI-compatible;
  - Qwen OpenAI-compatible.
- [x] Keep old provider-specific block structs until runtime is fully migrated.

Tests:

- [x] `cargo test -p researchcode-kernel --lib message`
- [x] runtime serializer fixture: text + reasoning + tool use + tool result
      round-trips without losing ids or reasoning metadata.

Acceptance:

- [x] No provider-visible tool result uses a fabricated id when provider id
      exists.
- [x] Raw volatile reasoning is representable but not persisted by default.
- [x] Cache control is representable as a message block instead of ad hoc request
      metadata only.

## 6. Phase 4: Kernel Hooks And Permission-Safe Dispatcher

Purpose:

Add Open-ClaudeCode-style lifecycle hooks while keeping PermissionGate as the
only authority for user approval and hard denial.

Tasks:

- [x] Add `crates/kernel/src/hooks.rs`.
- [x] Define hook events:
  - `SessionStart`;
  - `UserPromptSubmit`;
  - `PreToolUse`;
  - `PostToolUse`;
  - `PostToolUseFailure`;
  - `PreCompact`;
  - `PostCompact`;
  - `Stop`;
  - `ReasoningChainCompleted`;
  - `DsmlFallbackTriggered`.
- [x] Define hook decisions:
  - `Allow`;
  - `Modify`;
  - `Deny`;
  - `Warn`.
- [x] Add a runtime hook dispatcher that is sync bounded-blocking by default.
- [x] Add timeout policy:
  - timeout emits warning;
  - timeout cannot deny;
  - timeout cannot bypass permission;
  - default fallback is allow-with-warning for observability hooks.
- [x] Insert hook dispatch points in the native loop:
  - before tool schema validation for `PreToolUse`;
  - after tool execution for `PostToolUse`;
  - on tool failure for `PostToolUseFailure`;
  - around compaction;
  - when DeepSeek reasoning chain completes;
  - when DSML fallback triggers.
- [x] Ensure hook `Modify` re-enters schema validation and permission
      evaluation.
- [x] Emit hook lifecycle events:
  - `hook.dispatch.started`;
  - `hook.dispatch.completed`;
  - `hook.dispatch.timeout`;
  - `hook.dispatch.failed`.

Tests:

- [x] `PreToolUse` deny blocks dispatch and returns model-readable tool error;
- [x] `PreToolUse` modify changes args before schema validation;
- [x] hook timeout allows with warning;
- [x] hook cannot allow a hard-denied shell command;
- [x] DSML fallback emits `DsmlFallbackTriggered`;
- [x] reasoning completion emits `ReasoningChainCompleted`.

Acceptance:

- [x] Hooks are lifecycle/control points, not permission bypasses.
- [x] GUI/TUI can replay hook outcomes from events.

## 7. Phase 5: Transcript Store As Kernel/Runtime Contract

Purpose:

Make transcript the durable source of conversation, tool, reasoning, compaction,
cache, and subagent boundaries.

Tasks:

- [x] Add `crates/kernel/src/transcript.rs`.
- [x] Define `TranscriptEntry`:
  - entry id;
  - sequence;
  - timestamp;
  - kind;
  - optional `Message`;
  - optional kernel/runtime event;
  - optional cache breakpoint.
- [x] Define `TranscriptKind`:
  - user message;
  - assistant message;
  - tool use;
  - tool result;
  - reasoning chain;
  - compaction marker;
  - subagent boundary;
  - system note.
- [x] Add `crates/runtime/src/transcript_store.rs`.
- [x] Store session transcript as JSONL with sequence validation.
- [x] Add fsync or explicit durability policy for product sessions.
- [x] Add rotation policy for large transcripts.
- [x] Add `fork_from(parent_session_id, until_sequence)` with independent child
      append after fork.
- [x] Ensure raw volatile reasoning is never written to transcript.
- [x] Add cache breakpoint projection for DeepSeek caching planner.

Tests:

- [x] transcript append/reopen preserves sequence;
- [x] transcript fork copies prefix and diverges safely;
- [x] raw volatile reasoning is not serialized;
- [x] compaction marker and cache breakpoint survive replay;
- [x] large transcript rotation preserves validation.

Acceptance:

- [x] Event log and transcript have a clear relationship:
      transcript is model/conversation truth, event log is runtime lifecycle
      truth.
- [x] GUI can rebuild visible conversation without parsing provider-specific
      request strings.

## 8. Phase 6: Runtime Unified Message Wiring

Purpose:

Move `native_agent_loop` and provider request builders from string concatenation
to `kernel::Message` while preserving existing provider behavior.

Tasks:

- [x] Add an adapter layer:
  - `serialize_messages_for(family, endpoint, messages, capabilities)`;
  - provider-specific serializers behind strategy tables.
- [x] Replace tool-result string formatting with
      `ContentBlock::ToolResult { tool_use_id, content, is_error }`.
- [x] Replace reasoning string injection with
      `ContentBlock::Reasoning { sanitized, raw_volatile, .. }`.
- [x] Preserve provider-specific output for:
  - DeepSeek Anthropic-compatible;
  - DeepSeek OpenAI-compatible;
  - Qwen OpenAI-compatible.
- [x] Update conversation history projection to store kernel messages first,
      provider HTTP payload second.
- [x] Update event payloads to include provider id and internal id consistently.
- [x] Add migration helpers so old tests can still assert payload snapshots.

Tests:

- [x] old provider request snapshot tests still pass or have intentional snapshot
      updates;
- [x] three-tool-call replay keeps exact provider ids;
- [x] DeepSeek reasoning + tool result replay uses correct native field;
- [x] compatible provider baseline remains unchanged.

Acceptance:

- [x] Adding a provider no longer requires copying six message block types.
- [x] DeepSeek/Qwen native behavior is driven by capability policy, not generic
      provider config.

## 9. Phase 7: DeepSeek Reasoning Budget, DSML Policy, And Metrics

Purpose:

Turn DeepSeek-specific behavior from scattered fallback logic into explicit
session policy with observability.

Tasks:

- [x] Add `ReasoningBudget`:
  - max reasoning tokens per turn;
  - max total reasoning tokens;
  - auto-fold threshold.
- [x] Add `ReasoningFoldPolicy`:
  - keep raw for current adjacent replay only;
  - summarize after N turns;
  - drop older raw volatile state by time/turn window.
- [x] Add runtime budget checks at turn start and before provider replay.
- [x] Emit:
  - `reasoning.budget.checked`;
  - `reasoning.fold.started`;
  - `reasoning.fold.completed`;
  - `reasoning.replay.required`;
  - `reasoning.replay.injected`;
  - `reasoning.replay.missing`.
- [x] Add `ToolCallProtocolPolicy`:
  - preferred protocol;
  - fallback allowed;
  - DSML fallback warning threshold;
  - auto-disable DSML prompt guidance when native succeeds consistently.
- [x] Track per-session:
  - native tool-call success rate;
  - DSML fallback rate;
  - parser repair rate;
  - wrong-tool prevention rate.
- [x] Emit `tool.protocol.policy.updated` when policy changes.
- [x] Emit `DsmlFallbackTriggered` hook when threshold is exceeded.

Tests:

- [x] R1 long reasoning fixture folds old chains before context blow-up;
- [x] missing required reasoning replay blocks before HTTP call;
- [x] DSML fallback threshold emits hook/event;
- [x] consecutive native successes remove DSML guidance only for eligible
      DeepSeek variants;
- [x] raw reasoning never appears in GUI-visible events.

Acceptance:

- [x] DeepSeek reasoning is budgeted and replay-safe.
- [x] DSML is a measured recovery path, not a hidden parser accident.

## 10. Phase 8: DeepSeek Protocol Fallback And Error Classification

Purpose:

Make DeepSeek production failures recoverable according to DeepSeek-specific
semantics instead of generic `Err(String)`.

Tasks:

- [x] Add `DeepSeekProtocolStrategy`:
  - primary protocol;
  - optional fallback protocol;
  - fallback triggers.
- [x] Add fallback triggers:
  - HTTP 429/502/504;
  - invalid tool-use id;
  - native tool call unexpectedly empty;
  - context length exceeded.
- [x] Add `crates/runtime/src/deepseek_error.rs`.
- [x] Define `DeepSeekErrorClass`:
  - insufficient balance;
  - rate limited;
  - context length exceeded;
  - invalid tool calls;
  - upstream gateway;
  - stream truncated;
  - fatal.
- [x] Define `RetryStrategy`:
  - no retry;
  - immediate retry once;
  - exponential backoff;
  - compact then retry;
  - switch protocol then retry;
  - request model repair.
- [x] Integrate classification into provider response handling.
  - Implemented in `live_http_transport`: DeepSeek HTTP failures now produce
    `LiveHttpFailurePolicy`, `model.call_recovery_planned`, and enriched
    `model.call_blocked` gates.
  - `native_agent_loop` now uses bounded recovery for DeepSeek live HTTP calls
    and counts retry attempts.
- [x] Emit:
  - `deepseek.error.classified`;
  - `deepseek.retry.scheduled`;
  - `deepseek.protocol_fallback.started`;
  - `deepseek.protocol_fallback.completed`;
  - `deepseek.protocol_fallback.failed`.
- [x] Emit production recovery events through the current event taxonomy:
  - `model.call_recovery_planned`;
  - `model.call_recovery_attempted`;
  - enriched `model.call_blocked` gate, e.g.
    `http_status_400_invalid_tool_calls`.
- [x] Ensure fallback preserves provider tool ids and transcript sequence for
      bounded live HTTP retry/fallback attempts.

Tests:

- [x] mock 502 on OpenAI-compatible endpoint retries once within bounded time;
- [x] mock invalid tool call on Anthropic-compatible endpoint switches to
      OpenAI-compatible fallback within bounded time;
- [x] context length error triggers compaction retry;
- [x] invalid tool-use id triggers protocol fallback or model-readable repair;
- [x] insufficient balance is fatal and not retried;
- [x] stream truncation with partial tool input preserves partial assembly and
      asks model to repair.

Acceptance:

- [x] DeepSeek-specific error handling is centralized and tested.
- [x] Retry loops have max attempts and cannot spin forever for live HTTP
      gateway/protocol fallback recovery.
- [x] Context-length compaction retry and stream-truncation repair are integrated
      into the live HTTP path with bounded recovery tests.

## 11. Phase 9: DeepSeek Context Cache Planner

Purpose:

Make DeepSeek context caching automatic by deriving cache breakpoints from
transcript/message structure.

Tasks:

- [x] Add `crates/runtime/src/deepseek_cache_planner.rs`.
- [x] Define:
  - `CachePlan`;
  - `CacheBreakpoint`;
  - cache eligibility policy.
- [x] Plan breakpoints from kernel messages:
  - system prompt + repo instructions + tool schema prefix;
  - stable transcript prefix;
  - exclude most recent turns from caching.
- [x] Map `ContentBlock::CacheControl` to DeepSeek-compatible request payloads.
- [x] Use parsed prompt cache stats from stream responses to update telemetry.
- [x] Emit:
  - `deepseek.cache_plan.created`;
  - `deepseek.cache_plan.applied`;
  - `deepseek.cache_stats.recorded`;
  - `deepseek.cache_plan.skipped`.
- [x] Add GUI/TUI diagnostics for cache hit/miss without exposing raw prompt.

Tests:

- [x] long system prompt creates a breakpoint;
- [x] recent four turns are not cached;
- [x] cache stats parser updates session telemetry;
- [x] cache control serializes correctly for supported protocols;
- [x] unsupported variant skips with a structured reason.

Acceptance:

- [x] DeepSeek users get cache behavior without hand-writing request metadata.
- [x] Cache planner never stores secrets or raw volatile reasoning.

## 12. Phase 10: Subagent Kernel Spec And Runtime Isolation

Purpose:

Keep subagents useful without letting them share parent mutable state or bypass
project multi-agent policy.

Tasks:

- [x] Add `crates/kernel/src/subagent.rs`.
- [x] Define:
  - `SubagentSpec`;
  - `SubagentBudget`;
  - `SubagentResult`.
- [x] Move budget/spec validation into kernel tests.
- [x] Runtime subagents get:
  - isolated session id;
  - isolated transcript/event log;
  - explicit allowed tool list;
  - budget counters;
  - timeout;
  - parent merge event.
- [x] Preserve AGENTS.md banned multi-agent core-path rules:
  - no product kernel design delegation;
  - no permission manager core delegation;
  - no model router core delegation;
  - no DeepSeek/Qwen adapter core strategy delegation.
- [x] Add parent events:
  - `subagent.started`;
  - `subagent.result_received`;
  - `subagent.timeout`;
  - `subagent.failed`;
  - `subagent.merged`.

Tests:

- [x] explorer read-only subagent cannot write;
- [x] worker write-scope enforcement;
- [x] timed-out subagent does not freeze parent;
- [x] parent transcript includes summary/result, not child raw log by default;
- [x] subagent cannot consume parent budget silently.

Acceptance:

- [x] Subagent failure cannot freeze parent runtime.
- [x] Subagent isolation is a kernel/runtime invariant, not UI convention.

## 13. Phase 11: Open-ClaudeCode Parity Features Without Losing Native Edge

Purpose:

Align the missing parallel features from Open-ClaudeCode while preserving
DeepSeek-first design.

Tasks:

- [x] Add `PermissionMode` parity:
  - default ask;
  - accept edits;
  - plan;
  - bypass permissions;
  - do not ask.
- [x] Add `PermissionRule` with source/behavior/value.
- [x] Support layered permission sources:
  - project;
  - user;
  - session;
  - command-specific ephemeral approval.
- [x] Ensure permission rules cannot override hard-deny security boundaries.
- [x] Add slash command system only after hooks and transcript exist.
- [x] Defer plugin lifecycle until hook API is stable.

Tests:

- [x] permission mode maps to expected shell/edit behavior;
- [x] project rule and session rule precedence is deterministic;
- [x] hard-deny command remains blocked under permissive modes;
- [x] slash command dispatch cannot call hidden tools.

Acceptance:

- [x] We match the useful ClaudeCode/OpenClaudeCode control surface.
- [x] DeepSeek/Qwen native optimization remains kernel-level, not a plugin.

## 14. Phase 12: GUI/TUI Product Observability

Purpose:

Expose V2 kernel/runtime semantics to the user so long tasks are understandable:
approval, reasoning, DSML fallback, hook warnings, protocol fallback, cache, and
compaction should be visible as lifecycle signals.

Tasks:

- [x] Add GUI event renderers for:
  - reasoning folded;
  - DSML fallback triggered;
  - protocol fallback started/completed;
  - cache plan applied/stats recorded;
  - hook warning/denial;
  - transcript fork/subagent boundary.
- [x] Keep raw reasoning hidden.
- [x] Keep side-effect tool arguments redacted unless explicitly safe.
- [x] Add TUI equivalents through `RuntimeFacade -> AgentEvent`.
- [x] Add replay fixture for a long DeepSeek task showing:
  - reasoning;
  - tool use;
  - approval;
  - DSML fallback;
  - protocol fallback;
  - compaction/cache.

Tests:

- [x] `npm run build` in `desktop/`;
- [x] Tauri `cargo check`;
- [x] GUI replay fixture renders all lifecycle cards;
- [x] TUI smoke prints structured event summaries.

Acceptance:

- [x] User can tell why the model is waiting, retrying, compacting, or switching
      protocol.
- [x] No natural assistant text is filtered out as fake tool JSON.

## 15. Final Verification Matrix

Kernel:

- [x] `cargo fmt --all`
- [x] `cargo test -p researchcode-kernel --lib`
- [x] kernel message tests pass
- [x] kernel hooks tests pass
- [x] kernel transcript tests pass
- [x] kernel subagent tests pass

Runtime:

- [x] `cargo test -p researchcode-runtime --lib`
- [x] `cargo test -p researchcode-runtime live_http_transport --lib`
- [x] `cargo test -p researchcode-runtime native_agent_loop --lib`
- [x] `cargo test -p researchcode-runtime facade_owns_deepseek_native_loop_session_events --lib`
- [x] `cargo test -p researchcode-runtime facade_approval_decision_executes_pending_native_shell_tool --lib`
- [x] DeepSeek preservation fixtures pass
- [x] provider id replay fixtures pass
- [x] reasoning budget fixtures pass
- [x] DSML policy fixtures pass
- [x] protocol fallback fixtures pass
- [x] cache planner fixtures pass
- [x] transcript persistence fixtures pass

GUI/TUI:

- [x] `cargo check --manifest-path desktop/src-tauri/Cargo.toml`
- [x] `npm run build` in `desktop/`
- [x] manual `npm run tauri:dev` startup smoke
- [x] approval card appears for normal shell command
- [x] hard-deny shell command shows structured blocked state
- [x] long multi-tool DeepSeek run reaches final answer

Harness:

- [x] `python3 scripts/claudecode_gap_check.py`
- [x] `python3 scripts/check_all.py`

DeepSeek-first acceptance:

- [x] R1 reasoning budget folds old reasoning chains before total budget blow-up.
- [x] DSML fallback threshold emits a warning hook/event.
- [x] protocol fallback retries boundedly and preserves transcript/provider ids.
- [x] context cache planner produces cache hit telemetry when supported.
- [x] compatible providers remain compatible-only baselines.

## 16. Implementation Order For One Long Task

Recommended order:

1. Phase 1 preservation fixtures.
2. Phase 2 native model capabilities.
3. Phase 3 kernel message model.
4. Phase 5 transcript type/store skeleton.
5. Phase 6 runtime unified message wiring.
6. Phase 4 hooks and dispatcher.
7. Phase 7 reasoning and DSML policy.
8. Phase 8 DeepSeek retry/protocol fallback.
9. Phase 9 cache planner.
10. Phase 10 subagent isolation.
11. Phase 11 permission parity features.
12. Phase 12 GUI/TUI observability.
13. Final verification matrix.

This order keeps the kernel data model ahead of runtime rewrites, avoids adding
async dependencies prematurely, and makes DeepSeek-specific behavior testable
before it is exposed as product UI.

## 17. Progress Ledger

Use this section during implementation. Do not mark a phase complete unless its
tests and acceptance criteria pass.

| Phase | Status | Evidence |
|---|---|---|
| Phase 1 preservation fixtures | implemented baseline | Added `deepseek_first_preserves_native_primitives`; existing DSML/provider-id/artifact/context tests remain covered; verified by `cargo test -p researchcode-runtime --lib`. |
| Phase 2 native model capability matrix | implemented baseline | Added `DeepSeekVariant`, `QwenVariant`, `DeepSeekCapabilities`, `ToolCallingReliability`; provider capability probe now derives DeepSeek variant from model name and discourages R1 native tool calling; verified by kernel/runtime tests. |
| Phase 3 kernel message model | implemented baseline | Added `crates/kernel/src/message.rs`; request layer has kernel-message serializers for DeepSeek Anthropic, DeepSeek OpenAI, and Qwen OpenAI preserving reasoning/tool ids/cache control; verified by `live_model_request` tests. |
| Phase 4 hooks and dispatcher | implemented baseline | Added `crates/kernel/src/hooks.rs` and sync bounded `crates/runtime/src/hook_dispatcher.rs`; timeout returns allow-with-warning shape and PermissionGate remains authoritative; verified by hook tests. |
| Phase 5 transcript store | implemented baseline | Added `crates/kernel/src/transcript.rs` and `crates/runtime/src/transcript_store.rs`; append/reopen/fork skeleton rejects raw volatile reasoning persistence; verified by transcript tests. |
| Phase 6 runtime unified message wiring | implemented | Kernel-message provider serializers exist for DeepSeek Anthropic, DeepSeek OpenAI, and Qwen OpenAI; provider-id replay and compatible-provider baselines are covered by runtime tests. |
| Phase 7 reasoning/DSML policy | implemented | Added `ReasoningBudget`, fold policy, tool protocol metrics, DSML fallback warning threshold, native-stability heuristic, and live runtime events; verified by runtime policy and live HTTP tests. |
| Phase 8 protocol fallback/error classification | implemented | Added `deepseek_error.rs`, bounded retry strategies, `DeepSeekProtocolStrategy`, explicit DeepSeek recovery events, context-length compaction retry, fatal no-retry, and stream-truncation repair; verified by `live_http_transport` and full runtime tests. |
| Phase 9 context cache planner | implemented | Added `deepseek_cache_planner.rs` with cache breakpoints, `CacheControl` insertion, cache stats telemetry, and GUI/TUI diagnostics; verified by cache planner/runtime/desktop checks. |
| Phase 10 subagent isolation | implemented | Added kernel subagent spec plus runtime start/result/timeout/failure/merge events, core-path write guards, and summary-only parent merge tests. |
| Phase 11 Open-ClaudeCode parity features | implemented | Permission mode matrix, layered session/project rules, hard-deny precedence, permission resume, and slash palette parity are tested. |
| Phase 12 GUI/TUI observability | implemented | Desktop progress renderer and TUI cards cover recovery, reasoning, cache, subagent, and permission-resume lifecycle events. |
| final verification | implemented | `cargo fmt --all`, kernel/runtime/CLI tests, Tauri cargo check, desktop build, `npm run tauri:dev` startup smoke, `claudecode_gap_check`, and `check_all` pass. |
