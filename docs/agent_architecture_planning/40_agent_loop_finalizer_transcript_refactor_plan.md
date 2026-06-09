# 40 Agent Loop / Finalizer / Transcript Refactor Plan

> Scope: fix the current "transition text becomes completion", hidden inter-tool
> narration, duplicated read plateau, and premature GUI completed-state problems
> without drifting away from doc39's DeepSeek-native architecture.
>
> Primary references:
>
> - `docs/agent_architecture_planning/38_agent_kernel_v2_claude_code_alignment.md`
> - `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md`
> - Local OpenClaudeCode source:
>   `Open-ClaudeCode-main/Open-ClaudeCode-main/src/query.ts`
>   `Open-ClaudeCode-main/Open-ClaudeCode-main/src/QueryEngine.ts`
>   `Open-ClaudeCode-main/Open-ClaudeCode-main/src/services/tools/toolExecution.ts`
>   `Open-ClaudeCode-main/Open-ClaudeCode-main/src/services/api/claude.ts`
>   `Open-ClaudeCode-main/Open-ClaudeCode-main/src/components/Message.tsx`

---

## 0. Decision Summary

The current failure mode is not one bug. It is a layering violation:

1. `visible_finalizer_answer` was introduced as a recovery path for "tool calls
   happened but no assistant text was visible".
2. That recovery path became an alternate terminal path for normal write tasks.
3. Duplicate-read plateau detection tries to close the turn by manufacturing
   visible text.
4. Tool manifest filtering started carrying workflow-state semantics.
5. The GUI treats some runtime terminal events as user-visible completion even
   when the model only emitted a transition sentence.

The corrected architecture is:

```text
Model-driven loop:

assistant text/tool_use stream
  -> if tool_use exists: execute tools, append tool_result, continue model loop
  -> if no tool_use: assistant text is the natural final answer, complete turn

Runtime may stop only for:
  - explicit budget exhaustion
  - user interruption
  - permission wait / plan wait
  - provider/tool hard failure
  - compaction boundary requiring retry
```

No generic `final_answer` tool and no generic `visible_finalizer_answer` should
be part of the normal interactive loop.

DeepSeek-specific robustness remains inside NativeProfile / TCML:

- streaming tool-call accumulation;
- `reasoning_content` replay;
- DSML/content tool-call extraction;
- alias and JSON repair;
- duplicate observation feedback;
- cache-prefix stability;
- DeepSeek/Qwen role and temperature policy.

It must not be implemented as many small "scenario manifests" such as
`reading`, `writing`, `testing`, `reviewing` that hide and re-expose tools inside
a single agent turn.

---

## 1. What OpenClaudeCode Actually Does

This section records the local-source facts that matter for this refactor.

### 1.1 Loop Exit Is Model-Natural

OpenClaudeCode `queryLoop()` runs a `while (true)` loop in `src/query.ts`.
The important invariant is stated directly in source comments around the
tool-use collection path:

- `stop_reason === 'tool_use'` is unreliable.
- The actual continue/exit signal is whether streamed assistant content includes
  `tool_use` blocks.
- If assistant content includes tool_use blocks, execute tools and recurse with
  `messages + assistantMessages + toolResults`.
- If no tool_use blocks are present, the turn completes naturally.

Relevant local-source anchors:

- `src/query.ts:553` comment: stop_reason is unreliable; tool_use block presence
  is the loop signal.
- `src/query.ts:829-834`: assistant tool_use blocks are collected.
- `src/query.ts:1380-1395`: tool results are collected as user/tool_result
  messages.
- `src/query.ts:1715-1724`: next state includes prior messages, assistant
  messages, and tool results.

There is an optional `maxTurns` guard, but it is a budget/error boundary, not
the normal completion mechanism.

### 1.2 Final Answer Is Not A Tool

OpenClaudeCode has no normal `final_answer` or `finalizer` tool in the main
interactive loop.

The final answer is ordinary assistant text. SDK/headless mode emits an
additional `result` control frame, but that frame is not the final answer in the
transcript. It is a completion/control message.

Relevant local-source anchors:

- `src/QueryEngine.ts:1124-1149`: result text is extracted from the last
  assistant text block.
- `src/remote/sdkMessageAdapter.ts:220`: remote adapter ignores success result
  as transcript content and uses it as lifecycle signal.
- `src/tools/SyntheticOutputTool/SyntheticOutputTool.ts`: structured output is
  a special non-interactive schema mechanism, not the general agent final answer.

### 1.3 Tool Surface Is Stable Enough; Permission Gates Execution

OpenClaudeCode does have dynamic tool availability by environment, MCP,
deferred ToolSearch, CLI allow/deny configuration, and feature flags. But it
does not hide read/list/search just because the current user task is "write".

Important source facts:

- Base tools include read and write tools together in `src/tools.ts`.
- Read/search tools remain available during coding workflows.
- Plan mode is not implemented by hiding write tools from the model; it is an
  execution-time permission/policy decision.
- Missing/unknown tools become model-readable tool_result errors.
- There is no general "manifest recovery" that maps a denied/unavailable tool
  to a different tool and executes that instead.

This matches doc38/doc39: "manifest 不切，权限切".

### 1.4 Inter-Tool Narration Is First-Class Assistant Content

OpenClaudeCode keeps assistant text and tool_use blocks in the same assistant
message/content stream.

Relevant source anchors:

- `src/services/api/claude.ts:1995`, `2087`, `2113`: text and tool args are
  accumulated from streaming deltas.
- `src/services/api/claude.ts:2192`: each completed content block is yielded as
  assistant content.
- `src/query.ts:823-851`: assistant messages are yielded before tool execution;
  tool_use blocks are collected after the assistant message is already in the
  message stream.
- `src/components/Message.tsx:483-507`: UI renders text and tool_use content
  blocks, not only final result frames.

This is the design target for the desktop transcript: text between tools is not
noise. It is transcript content. Only provider/tool-markup leakage is filtered.

---

## 2. Current Deep-Code Deviations

The current implementation contains several historically understandable but now
harmful compensations.

### 2.1 `visible_finalizer_answer` Is A Patch, Not A Loop Primitive

It was introduced to cover a real earlier bug: tool calls completed but no
assistant message was emitted, causing the GUI to appear silent or stuck.

The problem is that a finalizer model call with no tools available can only
produce text. If the text is a transition sentence such as:

```text
让我先看看项目结构...
现在开始创建...
我将读取以下路径...
```

the runtime has no remaining tool path. It marks the turn completed even though
the model meant to continue.

This is exactly the bug observed in GUI runs: a natural write request produced
`visible_finalizer_answer` content that was a pre-action sentence, then the
turn completed.

### 2.2 Duplicate Plateau Currently Competes With Natural Looping

Duplicate observation handling is necessary for DeepSeek. doc39 B12 explicitly
calls out DeepSeek's "tool failure doom loop" and says the kernel must use an
observation cache and duplicate-batch feedback, without disabling tools.

The current failure is that plateau handling sometimes escalates to a finalizer
instead of continuing the same model-driven loop with better tool_result
feedback or ending with a structured budget/failure event.

Correct behavior:

- Duplicate read/list/search should produce a tool_result saying the prior
  result is still current.
- The model should receive that tool_result and choose the next action.
- If the model repeats the same duplicate enough times, runtime may stop with a
  structured budget/no-progress error.
- Runtime should not call a no-tool visible finalizer and pretend the task is
  complete.

### 2.3 Workflow-State Manifest Filtering Is The Wrong Boundary

The recently tested `writing` manifest path proves the risk:

- It hides read/list/search to force a write.
- But after writing, a capable agent often needs to read back, test, diagnose,
  or run a command.
- Other recovery paths can still reintroduce read-like behavior, creating
  contradictions between manifest policy and recovery policy.

This conflicts with doc38 and doc39:

- doc38 replaces "small state-specific active tool sets" with a stable
  per-session manifest.
- doc39 Phase 2 explicitly says: delete turn-state manifest slicing and keep
  manifest full while permissions cut execution.

### 2.4 Manifest Recovery Violates Tool Mediation Semantics

If the model asks for a tool that is not in the manifest or not allowed, runtime
should return a model-readable tool_result error.

It should not silently choose another tool, execute it, and feed that result
back as if it were the model's requested operation.

Allowed recovery:

- alias resolution for equivalent names (`read_file` -> `file.read`);
- deferred ToolSearch/schema discovery;
- JSON/schema repair along issue paths.

Not allowed:

- permission-denied write -> read/list;
- hidden tool -> repo.map;
- failed shell/list command -> read/list/repo.map if current policy says that
  read-like surface is unavailable.

### 2.5 GUI Completion Should Follow Runtime Turn Lifecycle

The GUI should show completed only after a terminal runtime lifecycle event:

- model natural end with assistant text/no tool_use;
- structured result/control event;
- failed/blocked with visible reason;
- interrupted;
- pending approval/waiting state.

It should not infer completion from a stray assistant sentence, a finalizer
message, or a collapsed tool card.

---

## 3. Target Architecture

### 3.1 Main Loop Contract

```rust
enum AgentLoopStep {
    ModelStarted { call_id: String },
    AssistantDelta { stream_id: String, text: String },
    AssistantMessageCompleted { message_id: String, blocks: Vec<AssistantBlock> },
    ToolCallsAssembled { calls: Vec<AssembledToolCall> },
    ToolResultsCompleted { results: Vec<ToolResultMessage> },
    NeedsModelContinuation,
    NaturalAssistantEnd { message_id: String },
    WaitingForPermission { request: PermissionRequest },
    WaitingForPlanApproval { request: PlanApprovalRequest },
    BudgetStopped { reason: BudgetStopReason },
    Failed { reason: RuntimeFailureReason },
    Interrupted,
}
```

Loop rule:

```text
if assistant message contains one or more tool calls:
    execute tools
    append tool_result messages
    continue model loop
else:
    natural assistant end
    complete turn
```

Runtime stop rule:

```text
only budget/interruption/wait/failure may stop without natural assistant end
```

There is no generic final-answer tool in the interactive loop.

### 3.2 Tool Manifest Contract

The model-visible tool set is produced from:

- registered core tools;
- enabled MCP/deferred tools;
- user/session tool allow/deny configuration;
- provider capability/schema constraints;
- feature gates.

It is not produced from:

- prompt keyword classification;
- workflow state such as `writing`;
- whether the last call was duplicate;
- whether runtime thinks the model "should write now".

TurnRouter may choose:

- prompt scaffold;
- role model;
- temperature;
- budget;
- context policy;
- permission mode.

TurnRouter must not hide read/list/write/test tools to steer the model.

### 3.3 Permission Contract

Permission is checked at execution time.

```rust
enum PermissionDecision {
    Allow,
    Ask(PermissionRequest),
    Deny(ModelReadableToolError),
}
```

Plan mode:

- tool remains visible;
- write/shell produces a model-readable "plan approval required" or user-facing
  approval request;
- model can continue after tool_result/approval.

Manual review/default:

- write/shell/patch generate approval request;
- read/search/list generally execute directly;
- denial is returned as tool_result, not finalizer.

### 3.4 Duplicate Observation Contract

Duplicate observations are a tool-result class, not a finalizer trigger.

```text
File unchanged since the earlier <tool> call in this conversation.
The previous tool_result is still current; refer to it instead of re-reading.
If you have enough evidence to complete the task, proceed with the next
non-duplicate action.
```

For write/edit tasks, the hint may additionally say:

```text
The user requested implementation. If enough evidence is available, use a
write/edit/patch tool with a concrete relative path.
```

But it must remain feedback, not a hidden tool policy.

No-progress stop:

- If the same duplicate/error signature repeats beyond budget, emit
  `agent.no_progress_stopped` or `agent.loop_budget_exhausted`.
- Mark the turn `Failed` or `Blocked`, not `Completed`.
- Include last duplicate signatures and suggested next action.
- Do not call `visible_finalizer_answer`.

### 3.5 Assistant Text / Tool Block Contract

Runtime event model should represent assistant output as blocks:

```rust
enum AssistantBlock {
    Text {
        block_id: String,
        text: String,
        phase: AssistantTextPhase,
    },
    ToolUse {
        block_id: String,
        provider_tool_call_id: String,
        tool_id: String,
        arguments_json: String,
    },
    Reasoning {
        block_id: String,
        sanitized_preview: String,
    },
}

enum AssistantTextPhase {
    PreTool,
    InterTool,
    PostTool,
    NaturalFinal,
}
```

The GUI transcript should render text blocks as assistant messages. Tool cards
are grouped with their adjacent tool_use/tool_result, but they do not delete or
hide surrounding text.

Filtering is allowed only for:

- DSML/tool-call markup leakage;
- duplicate stream chunks;
- transient partial text superseded by final message.

Filtering is not allowed for normal inter-tool narration.

### 3.6 Completion Contract

Runtime emits explicit terminal lifecycle events:

```text
agent.turn.completed
agent.turn.failed
agent.turn.blocked
agent.turn.interrupted
agent.turn.awaiting_permission
agent.turn.awaiting_plan
```

The GUI composer state should derive from these events, not from transcript
content.

`assistant.message` is content, not lifecycle.

`result`-like frames are lifecycle/control, not final transcript text.

---

## 4. DeepSeek-Specific Adaptation Constraints

This section is the doc39 compatibility gate. Any implementation that violates
these points must stop and ask for approval.

### 4.1 Preserve `reasoning_content` Replay

Conflict risk:

- Removing finalizer paths changes how tool_result continuation is built.
- DeepSeek thinking mode may require prior `reasoning_content` attached to the
  previous assistant-with-tool-call message.

Required behavior:

- Do not flatten assistant/tool_result history into plain text.
- Do not synthesize a fake final assistant between assistant tool_use and
  tool_result.
- Keep `ReasoningReplayManager` ownership in NativeProfile.
- If a turn has tool calls and DeepSeek thinking reasoning, next provider call
  must replay raw reasoning as doc39 describes.

Notify user if:

- the current provider endpoint rejects reasoning replay shape;
- removing finalizer exposes a 400 from DeepSeek thinking mode;
- we need to choose between Anthropic-compatible and OpenAI-compatible message
  shapes for the same DeepSeek model.

### 4.2 Preserve DSML / Content Tool-Call Extraction

Conflict risk:

- Treating visible assistant text as first-class transcript content could
  accidentally show DSML `<tool_calls>` leakage.

Required behavior:

- DeepSeek stream processor still filters DSML markup from visible text.
- Extracted content tool calls remain candidates that go through TCML.
- Candidate state-changing tools still require permission.

Notify user if:

- DSML leakage cannot be reliably distinguished from user-facing code examples;
- a provider sends both structured tool_calls and DSML content for the same
  call and dedupe policy is ambiguous.

### 4.3 Stable Tool Catalog For Cache Prefix

Conflict risk:

- Removing workflow-state manifest filtering is good for cache stability.
- But changing tool order/schema or per-turn tool availability can still break
  DeepSeek prefix cache.

Required behavior:

- Sort tools by canonical id in prompt Zone A.
- Keep tool schema byte-stable per session whenever registered tools do not
  change.
- Put permission mode and plan state in Zone B/C guidance, not tool schema.

Notify user if:

- MCP/deferred tool discovery changes Zone A every turn;
- provider-specific schema differences force separate cache zones.

### 4.4 Duplicate Observation Is DeepSeek Guidance, Not Tool Hiding

Conflict risk:

- DeepSeek may ignore soft hints and repeat duplicate reads.
- Hiding tools is tempting but conflicts with doc38/doc39 and OpenClaudeCode.

Required behavior:

- Duplicate handling remains tool_result feedback.
- Repetition beyond budget stops as blocked/failed, not completed.
- Read tools stay visible for later validation after writes.

Notify user if:

- DeepSeek repeatedly fails natural write progression even with duplicate
  feedback;
- we need an eval-backed DeepSeek-specific "write nudge" message that is stronger
  than OpenClaudeCode's natural loop but still does not hide tools.

### 4.5 Qwen Must Not Inherit DeepSeek-Only Parser Rules Blindly

Conflict risk:

- This refactor touches NativeProfile boundaries. DeepSeek DSML, reasoning
  replay, and tool-call repair may not apply to Qwen.

Required behavior:

- Keep DeepSeek and Qwen parser/repair policies separated.
- Shared loop lifecycle is allowed.
- NativeProfile-specific stream parsing stays family-specific.

Notify user if:

- a shared event schema requires Qwen to emit DeepSeek-style reasoning fields;
- Qwen parser fixture regressions appear while fixing DeepSeek.

---

## 5. Required Refactor Work

### Phase A: Stop The Harmful Finalizer Path

Goal: make interactive turns complete naturally or stop structurally.

Changes:

1. Remove `agent.final_answer` from normal tool catalog if still present.
2. Remove `visible_finalizer_answer` as a completion path for interactive
   write/edit/tool turns.
3. Keep only a narrow diagnostic fallback for legacy non-interactive callers,
   gated behind explicit mode, not desktop interactive mode.
4. If no assistant text and no tool_use after a provider call:
   - if `stop_reason=end_turn`: complete with empty assistant result only as a
     control event, not a fabricated transcript answer;
   - otherwise emit provider failure/no-output event.
5. If plateau/no-progress fires:
   - emit blocked/failed with structured reason;
   - do not synthesize final user-facing answer.

Tests:

- natural text answer completes with assistant text;
- tool_use -> tool_result -> text completes naturally;
- tool_use -> duplicate plateau stops as blocked, not completed;
- transition sentence without tool_use does not become completed write task if
  task requires state-changing tool and no state change happened.

### Phase B: Revert Workflow-State Manifest Slicing

Goal: restore stable manifest semantics.

Changes:

1. Remove `writing` manifest state as a product behavior.
2. Remove `workflow_state_allows_tool()` from general manifest filtering, or
   limit it to explicit user/session tool policy modes only.
3. Make `FastAutoWrite` / `CodeEdit` expose read/search/list/write/edit/patch
   consistently.
4. PermissionPolicy decides execution, not visibility.
5. Recovery code must never bypass manifest/permission by choosing another
   non-equivalent tool.

Tests:

- write task manifest still includes read/list/search and write/edit/patch;
- plan/default/manual review permissions still block or ask at execution;
- unavailable tool returns model-readable error;
- no automatic `repo.map` fallback when a tool is not visible/allowed.

### Phase C: Rewrite Duplicate Plateau Handling

Goal: duplicate observation guides the model but does not final-answer the task.

Changes:

1. Duplicate read/list/search returns a clear `tool_result` stub.
2. Stub references prior tool_result and avoids "inspect narrower target" as the
   default nudge.
3. For implementation requests, add next-action hint: use write/edit/patch if
   enough evidence exists.
4. Track duplicate signatures in TurnState.
5. If duplicate signatures exceed threshold:
   - `agent.no_progress_stopped`;
   - state `Blocked` or `Failed`;
   - GUI displays retry/continue controls;
   - session memory does not persist fabricated fallback text.

Tests:

- duplicate read produces stub tool_result;
- repeated duplicate stop is not `Completed`;
- subsequent user follow-up is not polluted by "本轮已完成..." fallback memory;
- external file mtime change invalidates duplicate cache when applicable.

### Phase D: Transcript Block Model

Goal: text around tools is retained and rendered.

Changes:

1. Runtime emits assistant block events:
   - `assistant.block_started`
   - `assistant.text_delta`
   - `assistant.tool_call_delta`
   - `assistant.block_completed`
2. Preserve block order.
3. GUI consumes block events and renders text/tool cards in order.
4. Streaming partial text is reconciled with final assistant message, not
   deleted by tool cards.
5. `assistant.message` remains a content event, not lifecycle.

Tests:

- "I will read X" before a tool remains visible;
- text between two tool calls remains visible;
- post-tool final answer remains visible;
- DSML markup is filtered while normal code fences remain visible;
- no duplicate text after stream finalization.

### Phase E: Completion State Cleanup

Goal: GUI composer/status follows runtime lifecycle.

Changes:

1. Add/normalize terminal events:
   - `agent.turn.completed`
   - `agent.turn.failed`
   - `agent.turn.blocked`
   - `agent.turn.interrupted`
   - `agent.turn.awaiting_permission`
   - `agent.turn.awaiting_plan`
2. Composer badge uses lifecycle event, not last transcript row.
3. Tool cards may finish while turn remains executing.
4. Permission approval result can either:
   - resume model continuation if the loop needs it; or
   - complete structurally if the approved tool itself satisfied the turn.
5. Interruption sends cancellation to runtime and waits for `interrupted`, not
   only local UI reset.

Tests:

- long task with tools does not show completed while next model call pending;
- permission wait shows awaiting approval;
- approval resumes/settles deterministically;
- interruption cancels running sidecar/model wait and releases composer.

### Phase F: DeepSeek Native Regression Suite

Goal: prevent this refactor from regressing doc39.

Required fixtures:

1. DeepSeek Anthropic-compatible tool_use + tool_result loop.
2. DeepSeek OpenAI-compatible streaming `tool_calls.delta` accumulation.
3. `reasoning_content` replay after tool call.
4. DSML content leakage filtered from transcript but extracted as candidate.
5. Duplicate read loop stops as blocked after threshold.
6. Natural write request:
   - read/list allowed;
   - write permission requested;
   - after approval, file is written;
   - model may read/test after write.
7. Qwen parser fixture unchanged.

---

## 6. Files Likely Affected

Runtime:

- `crates/runtime/src/native_agent_loop.rs`
- `crates/runtime/src/native_agent_loop_prompt.rs`
- `crates/runtime/src/native_agent_loop_execution.rs`
- `crates/runtime/src/native_agent_loop_util.rs`
- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/agent_kernel/conversation_history.rs`
- `crates/runtime/src/agent_kernel/evidence_ledger.rs`
- `crates/runtime/src/tcml/manifest.rs`
- `crates/runtime/src/tcml/contract.rs`
- `crates/runtime/src/runtime_facade_impl.rs`
- `crates/runtime/src/local_api_server.rs`

Desktop:

- `desktop/src/hooks/useRuntimeEventSubscription.ts`
- `desktop/src/hooks/useStreamingTranscript.ts`
- `desktop/src/runtime/streamSanitizer.ts`
- `desktop/src/runtime/transcriptDedupe.ts`
- `desktop/src/components/Transcript.tsx`
- `desktop/src/components/Composer.tsx` or current status/composer owner
- `desktop/gui_three_round_smoke.mjs`

Tests:

- `crates/runtime/src/native_agent_loop_tests.rs`
- `crates/runtime/src/agent_kernel/turn_controller.rs` tests
- `crates/runtime/src/tcml/manifest.rs` tests
- `desktop/test_runtime_event_replay.mjs`
- `desktop/gui_three_round_smoke.mjs`

---

## 7. Conflicts / Must Notify User

This section is intentionally explicit. If any item below occurs during
implementation, stop and notify the user instead of silently choosing.

### C1. `visible_finalizer` For Read-Only Q&A

Potential conflict:

- Removing finalizer globally may reintroduce the old "tools completed but no
  visible reply" bug for read-only Q&A.

Default decision:

- Remove it from interactive write/tool turns first.
- For read-only Q&A, prefer natural loop. If provider returns no text and no
  tool_use, emit structured no-output error rather than a fabricated final
  answer.

Notify if:

- tests show common read-only Q&A now ends with empty output;
- a narrow read-only summarizer fallback is required.

### C2. DeepSeek Ignores Duplicate Feedback

Potential conflict:

- OpenClaudeCode trusts Claude to react to duplicate stubs; DeepSeek may keep
  repeating.

Default decision:

- Keep tools visible.
- Stop as blocked after no-progress budget.
- Do not hide tools.

Notify if:

- product requirement is to force progress even when DeepSeek ignores feedback;
- we need a DeepSeek-only prompt nudge/eval gate.

### C3. Plan Mode Semantics

Potential conflict:

- Current desktop plan approval UI may assume plan state is a workflow mode.
- doc38/doc39 say plan is permission semantics, not manifest slicing.

Default decision:

- Keep plan UI.
- Move enforcement to PermissionPolicy at execution time.

Notify if:

- existing plan approval flow cannot represent model-visible write tools that
  are execution-denied until plan approval.

### C4. Provider Protocol Split

Potential conflict:

- DeepSeek Anthropic-compatible path and OpenAI-compatible path may encode
  assistant/tool_result/reasoning differently.

Default decision:

- Shared loop semantics, provider-specific message projection.

Notify if:

- one protocol requires a fake assistant message or fake finalizer to remain
  valid.

### C5. GUI Transcript Storage Format

Potential conflict:

- Moving to block-ordered assistant transcript may require migration or replay
  compatibility logic.

Default decision:

- Event log remains source of truth.
- Add projection layer for old events.

Notify if:

- old sessions cannot be rendered faithfully without a migration.

---

## 8. Acceptance Criteria

### Architecture

- No generic `agent.final_answer` / `visible_finalizer_answer` in interactive
  normal loop.
- Tool manifest is not sliced by `writing` / `reading` / `testing` state.
- Permission is enforced at execution time.
- Duplicate plateau cannot mark a write task completed without a state-changing
  tool result.
- Assistant text before/between/after tools is preserved in transcript.

### Runtime Tests

- `cargo test -p researchcode-runtime native_loop_tool_acceptance_write_request_is_not_inventory`
- permission resume test still passes.
- new manifest stability tests pass.
- new duplicate plateau blocked-state tests pass.
- DeepSeek reasoning replay fixtures pass.
- Qwen parser fixtures pass or documented as unaffected.

### Desktop Tests

- runtime event replay passes.
- GUI real-runtime long task with:
  - greeting turn;
  - natural write request;
  - read/list exploration;
  - write approval;
  - file write;
  - optional read/test after write;
  - final assistant text.
- GUI shows no premature `已完成`.
- GUI does not hide inter-tool narration.
- interrupt and follow-up work after long task.

### Manual Smoke

Use the same real prompt that reproduced the incident:

```text
你好啊

来，这个文件夹中有一个vioce项目，我希望你能够写入其中的测试文件
```

Expected:

- first turn completes naturally;
- second turn may inspect project;
- duplicate inspections are stubbed, not repeated indefinitely;
- write request appears and can be approved;
- file is created;
- after approval, turn either naturally answers or continues validation;
- no `visible_finalizer_answer`;
- no `agent.final_answer`;
- no `Completed` before terminal lifecycle event.

---

## 9. Implementation Order

1. Freeze this document as the active plan.
2. Revert/remove `writing` manifest slicing introduced during debugging.
3. Normalize loop completion around natural assistant end and structured
   terminal events.
4. Remove interactive visible finalizer path.
5. Rewrite duplicate plateau to blocked/no-progress, not completed/finalized.
6. Remove manifest recovery that substitutes non-equivalent tools.
7. Add transcript block events/projection.
8. Fix GUI completion-state derivation.
9. Add DeepSeek/Qwen regression fixtures.
10. Run real GUI long-task smoke.

## 10. Implementation Evidence 2026-06-02

This plan is now implemented as a runtime/desktop contract, not just as a
design note.

### Phase Coverage

- Phase A: normal interactive loop no longer exposes or calls a generic
  `agent.final_answer` / `visible_finalizer_answer` completion path. Budget,
  empty-visible, max-tool, and plateau exits use structured stop/failure events.
- Phase B: workflow-state manifest slicing was removed for coding/write/test
  steering. Write/code-edit contexts keep read/list/search/validation surfaces
  visible; permission still gates execution.
- Phase C: duplicate observations remain model-visible tool-result feedback and
  no-progress plateaus stop as blocked/failed, not completed/finalized.
- Phase D: assistant text/tool-call transcript blocks are emitted as ordered
  events: `assistant.block_started`, `assistant.text_delta`,
  `assistant.tool_call_delta`, and `assistant.block_completed`.
- Phase E: desktop stream/composer state follows runtime lifecycle events and
  no longer treats inter-tool narration or tool-card settling as turn
  completion.
- Phase F: DeepSeek and Qwen regression fixtures cover reasoning replay,
  OpenAI-compatible streaming tool accumulation, DSML/content-tool fallback,
  Qwen parser/tool continuation, and native profile boundaries.

### Verified Gates

- `cargo fmt --all --check`
- `cargo test -p researchcode-runtime records_assistant_text_block_around_visible_stream_content --lib -- --nocapture`
- `cargo test -p researchcode-runtime records_assistant_tool_call_block_before_assembled_tool_call --lib -- --nocapture`
- Focused native-loop regressions:
  - `final_answer_tool_is_not_exposed_to_native_loop`
  - `native_agent_loop_v2_blocks_after_max_iterations_without_visible_finalizer`
  - `native_agent_loop_v2_blocks_after_max_tool_calls_without_visible_finalizer`
  - `native_agent_loop_v2_suppresses_duplicate_observation_calls`
  - `native_agent_loop_v2_visible_only_transition_after_tools_continues_loop`
  - `native_agent_loop_v2_legacy_final_answer_tool_call_does_not_stop_loop`
  - `qwen_native_agent_loop_v2_fastauto_write_executes_file_write`
- DeepSeek/Qwen preservation checks:
  - `native_agent_loop_v2_context_compaction_folds_reasoning_replay`
  - `native_tool_result_continuation_uses_provider_names_and_openai_call_ids`
  - `streaming_tool_assembler_completes_split_json_arguments`
  - `qwen_stream_processor_assembles_openai_tool_calls`
- Desktop checks:
  - `npm --prefix desktop run build`
  - `node desktop/test_runtime_event_replay.mjs`
  - `node desktop/test_desktop_polish_contract.mjs`
  - `node desktop/gui_conversation_quality_smoke.mjs`
  - `node desktop/gui_toolstorm_latency_smoke.mjs`
  - `node desktop/gui_permission_longtask_smoke.mjs`
  - `node desktop/gui_three_round_smoke.mjs`
- Broad gate:
  - `python3 scripts/check_all.py` passed end-to-end with local GUI port
    permissions enabled. Live provider checks were skipped by the offline
    `network_not_enabled` gate, while recorded/fixture/native event-log paths
    passed.

### Residual Keyword Policy

Remaining `visible_finalizer` / `agent.final_answer` string hits in `crates/`
are negative assertions or legacy-fixture inputs. They prove the old path is
not exposed or cannot terminate the loop. They are not production completion
paths.

---

## 11. Non-Goals

- Do not make Claude/OpenAI native optimization first-class.
- Do not genericize DeepSeek/Qwen NativeProfile into provider config.
- Do not remove duplicate observation handling entirely.
- Do not remove max-turn/max-budget boundaries entirely; they are structured
  safety/budget controls, not normal completion.
- Do not implement scenario-specific tool hiding as the final architecture.
- Do not make SDK/result/control frames visible as final transcript answers.

---

## 12. Short Version

OpenClaudeCode's core loop is simple and strong:

```text
assistant has tool_use -> run tools -> append tool_result -> continue
assistant has no tool_use -> natural final answer -> complete
```

Deep-code should adopt that loop discipline while keeping doc39's DeepSeek
native machinery around it.

The repair target is not "make a better finalizer". The target is:

- no finalizer in the normal interactive path;
- stable tools;
- execution-time permission;
- duplicate observations as tool_result feedback;
- structured blocked/failed stops;
- transcript text as first-class content;
- DeepSeek reasoning/DSML/cache adaptations kept in NativeProfile/TCML.
