# Doc39 AgentKernel Execution TODOs

本文是 `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md`
的执行清单。旧工作清单已废弃；本文件不再以 doc37/doc38/doc39 混合观点为准，
而是以 doc39 为架构源头，把 doc39 的每一个核心点拆成可执行 TODO。

原则：

- doc39 是目标架构；
- 本文件是施工合同；
- 不再把 DeepSeek 优化写成 generic kernel 外面的 adapter；
- 不抄 Anthropic wire format；
- 不抄 Claude 长 system prompt；
- 不把本项目降级成通用 multi-provider 适配框架；
- 不再采用 "按 turn/workflow 切掉 manifest" 的旧思路；
- 采用 doc39/doc38 对齐后的核心策略：**manifest 全开，权限切**；
- 每个 DeepSeek-specific primitive 都必须落实到模块、测试、事件和 eval gate；
- 工作不能在 P0/P1 中途停止，除非触碰 AGENTS.md stop condition。

## 0. Target

最终目标：

```text
DeepSeek-native agent kernel
= Claude-Code-grade discipline
  + DeepSeek-shaped primitives
  + Qwen native profile boundary
  + TCML不可绕过
  + event单一真相源
  + permission-gated execution
  + compaction/session/subagent/product observability
```

不可妥协的不变式：

- [ ] AgentKernel 是 turn 的唯一 orchestrator。
- [ ] RuntimeFacade 是外部 public API，但不直接拥有 loop policy。
- [ ] NativeProfile 是 DeepSeek/Qwen native 行为入口。
- [ ] AgentKernel 内不出现散落的 `if family == DeepSeek`。
- [ ] ToolManifest 尽量完整可见，不按普通 turn-state 任意切工具。
- [ ] PermissionPolicy/PermissionGate 决定工具是否能执行。
- [ ] TCML 是所有模型 tool call 的唯一 mediation 路径。
- [ ] StreamProcessor 是 DeepSeek-native heart。
- [ ] ReasoningReplayManager、CachePrefixPolicy、AliasRegistry、RepairCatalog、
      RoleSplit 是一等 kernel primitive。
- [ ] GUI/TUI/local API 消费结构化 event，而不是猜最终文本。
- [ ] DeepSeek/Qwen native promotion 必须由 eval gate 决定。

## 1. Task Contract

### Goal

从当前 runtime 状态开始，持续完成 doc39 指定的 8 个 implementation phases，
并补齐 doc39 §0-20 的所有组件、风险、eval、telemetry、file-level changes。

目标链路：

```text
RuntimeFacade
  -> AgentKernel
  -> NativeProfile(DeepSeek/Qwen)
  -> ProviderAdapter/StreamProcessor
  -> TCML
  -> PermissionGate
  -> ToolDispatcher
  -> ToolExecutors
  -> ResultFormatter
  -> ConversationHistory/EventLog
  -> Compactor/Subagent/ToolDoctor/GUI
```

### Do Not Stop Rule

- [ ] 不因 Phase 1 完成而停止。
- [ ] 不因某个 smoke test 通过而停止。
- [ ] 不把 doc39 的 Phase 7/8 或 telemetry/eval 重新降级成 deferred。
- [ ] 每完成一个 phase 必须更新 Progress Ledger，然后继续下一个未阻塞 phase。
- [ ] 如果某个 phase 被 blocker 阻塞，记录 blocker 并继续其它独立可执行项。

### Stop Conditions

Stop and report only if:

- [ ] 需要修改 root `AGENTS.md`。
- [ ] 需要读取 secrets/private keys/SSH files。
- [ ] 需要 destructive git operation 或 git history rewrite。
- [ ] 需要安装依赖或访问网络。
- [ ] 需要 breaking kernel/event/database/schema/security change 且无兼容路径。
- [ ] 需要改变 DeepSeek/Qwen native adapter core strategy 但没有 eval fixture。
- [ ] 测试暴露无法在当前 scope 内修复的架构矛盾。
- [ ] 用户显式要求暂停或改方向。

### Required Checks

Focused checks after each coherent slice:

```text
cargo test -p researchcode-runtime <focused_test_name>
cargo test -p researchcode-kernel <focused_test_name>
python3 scripts/claudecode_gap_check.py
```

Broad gate before final:

```text
python3 scripts/check_all.py
```

GUI/local API checks when runtime events are ready:

```text
python3 scripts/test_local_api_server.py
npm run tauri:dev
```

If a broad check fails:

- [ ] record exact command;
- [ ] record exact failing test;
- [ ] classify blocker vs unrelated existing failure;
- [ ] continue other unblocked work;
- [ ] never claim final completion while relevant failure remains.

## 2. Doc39 Coverage Matrix

| doc39 section | Required execution coverage |
|---|---|
| §0 Thesis | Target invariants in sections 0, 23 |
| §1 DeepSeek Behavior Catalog | B1-B13 checklist in section 3 |
| §2 Architecture Layers | module/path tasks in sections 4, 7, 8 |
| §3 Data Model | TurnState, ToolCallLifecycle, error, modes, roles in section 5 |
| §4 StreamProcessor | Phase 5 and section 10 |
| §5 ToolCallLifecycle | TCML pipeline in section 9 |
| §6 ReasoningReplayManager | Phase 5 and section 11 |
| §7 CachePrefixPolicy | Phase 5 and section 12 |
| §8 RepairCatalog | Phase 6 and section 13 |
| §9 AliasRegistry | Phase 6 and section 14 |
| §10 ProviderCapabilityMatrix | Phase 5 and section 15 |
| §11 RoleSplit + TemperatureSchedule | Phase 5 and section 16 |
| §12 Compactor | Phase 4 and section 17 |
| §13 ModelReadableToolError | Phase 6 and section 18 |
| §14 ToolResult Format | Phase 6 and section 19 |
| §15 Implementation Phases | sections 7-22 |
| §16 File-Level Changes | section 21 |
| §17 Risk Register | section 24 |
| §18 Eval Gates | section 25 |
| §19 Telemetry | section 26 |
| §20 Summary | final invariants in section 23 |
| TODO extension: GUI/TUI runtime integration | section 27 |

Acceptance:

- [ ] Every row above has completed tasks or a documented blocker.
- [ ] No doc39 section is treated as optional.
- [ ] Any deviation from doc39 is recorded as Architecture Deviation with reason,
      eval impact, and rollback condition.

## 3. DeepSeek Behavior Catalog Tasks

These tasks map doc39 B1-B13 into runtime behavior.

### B1 Reasoning Replay Required

- [ ] Implement capture of DeepSeek `reasoning_content` during thinking turns.
- [ ] Associate captured reasoning with `(session_id, turn_index, assistant_message_id)`.
- [ ] Detect when a tool-use chain requires reasoning replay in turn `t+1`.
- [ ] Inject raw reasoning into the last assistant message for the next DeepSeek request.
- [ ] Block before provider call with recoverable error if required replay is missing.
- [ ] Store raw replay content separately from GUI preview.
- [ ] Never sanitize `raw_reasoning` before provider replay.
- [ ] Only sanitize `sanitized_preview` for GUI display.
- [ ] Drop raw reasoning when the next turn has no tool-use chain requirement.
- [ ] During compaction, keep only the latest replay-required raw reasoning.
- [ ] Drop raw reasoning when session closes.

Acceptance:

- [ ] Thinking + tool-use + next turn request contains correct reasoning content.
- [ ] Missing replay is detected before HTTP/provider call.

### B2 Reasoning Can Be 10K+ Tokens

- [ ] Track reasoning tokens separately from visible output tokens.
- [ ] Budget reasoning separately in `TurnState`.
- [ ] Ensure compaction does not truncate required latest raw reasoning.
- [ ] Store sanitized preview separately from raw replay content.

Acceptance:

- [ ] Large reasoning content is preserved for required adjacent replay.
- [ ] GUI preview can be sanitized without modifying raw replay.
- [ ] Provider replay never receives sanitized reasoning content.

### B3 Pro/Flash Capability Gradient

- [ ] Implement RoleSplit model mapping.
- [ ] Default Executor to DeepSeek Pro/main chat endpoint.
- [ ] Default Compactor to DeepSeek Flash.
- [ ] Default Titler/Summarizer to DeepSeek Flash.
- [ ] Make role model mapping configurable with safe defaults.

Acceptance:

- [ ] Compaction uses Flash by default.
- [ ] Executor stays on stronger DeepSeek profile by default.

### B4 Temperature Sensitivity

- [ ] Implement TemperatureSchedule by role stage.
- [ ] Routing uses `0.0`.
- [ ] PlanDrafting uses `0.5`.
- [ ] Executing uses `0.2`.
- [ ] Reviewing uses `0.0`.
- [ ] Compacting uses `0.0`.
- [ ] NarrativeAnswer uses `0.7`.

Acceptance:

- [ ] Provider request carries stage-specific temperature.
- [ ] Tool-calling stages use low temperature.

### B5 Tool Calls Delta Across Chunks

- [ ] Implement indexed partial tool call accumulation.
- [ ] Merge `delta.tool_calls[i].function.arguments` by index.
- [ ] Parse only after args are complete or stream finalizes.
- [ ] Keep parallel indices separate.

Acceptance:

- [ ] Five chunks can assemble one valid JSON args object.
- [ ] Two indexed tool calls do not merge into one.

### B6 DSML Content Leakage

- [ ] Move existing DSML filter into `native_profile/deepseek/stream.rs`.
- [ ] Make DSML filter stateful across chunks.
- [ ] Suppress DSML/tool-call markup from visible output.
- [ ] Emit DSML leak telemetry.
- [ ] Add ContentToolCallExtractor candidate path.
- [ ] Ensure content-extracted calls never directly execute.

Acceptance:

- [ ] Cross-chunk DSML tags do not leak into visible stream.
- [ ] Extracted content tool call is candidate-only.

### B7 Strict Mode Beta Endpoint

- [ ] Add capability flag for strict JSON schema support.
- [ ] Probe or configure whether endpoint supports beta strict mode.
- [ ] If strict unsupported, keep runtime validator as source of truth.
- [ ] Surface strict capability in ToolDoctor.

Acceptance:

- [ ] Non-strict endpoints still validate through TCML.
- [ ] Strict mode is not assumed without capability evidence.

### B8 Provider Capability Differs By Endpoint

- [ ] Implement ProviderCapabilityMatrix.
- [ ] Probe simple tool calls.
- [ ] Probe streaming tool calls.
- [ ] Probe `tool_choice="required"`.
- [ ] Probe strict JSON schema support.
- [ ] Probe reasoning replay support for DeepSeek thinking.
- [ ] Detect vLLM tool parser flag where represented.
- [ ] Detect reasoning parser flag where represented.
- [ ] Cache capability results by endpoint hash with 24h ttl.
- [ ] Store capability cache at `~/.researchcode/capabilities/{endpoint_hash}.json`.
- [ ] Use cached capability result when ttl is valid.
- [ ] Add explicit override path for known endpoint/proxy quirks.

Acceptance:

- [ ] Session stores ToolCallingCapabilities.
- [ ] Kernel behavior reads capability matrix before assuming provider feature.

### B9 Tool Name Error Catalog

- [ ] Implement AliasRegistry.
- [ ] Add `file.read` aliases:
  - `read`
  - `Read`
  - `read_file`
  - `readFile`
  - `fileRead`
  - `read_source_code`
  - `ReadFile`
- [ ] Add `file.list_directory` aliases:
  - `ls`
  - `list`
  - `list_dir`
  - `listDir`
  - `ListDirectory`
  - `ListDir`
  - `list_files`
  - `list_directory`
- [ ] Add `search.ripgrep` aliases:
  - `grep`
  - `Grep`
  - `search`
  - `Search`
  - `rg`
  - `ripgrep`
  - `search_files`
  - `SearchFiles`
- [ ] Add `shell.command` aliases:
  - `bash`
  - `Bash`
  - `shell`
  - `Shell`
  - `exec`
  - `Exec`
  - `run`
  - `RunCommand`
  - `execute_command`
- [ ] Add `plan.enter` aliases:
  - `plan`
  - `Plan`
  - `plan_enter`
  - `enter_plan`
  - `EnterPlanMode`
- [ ] Add `file.write` aliases:
  - `write`
  - `Write`
  - `write_file`
  - `WriteFile`
  - `save`
  - `save_file`
- [ ] Add `file.edit` aliases:
  - `edit`
  - `Edit`
  - `edit_file`
  - `EditFile`
  - `modify`
  - `patch`
  - `PatchFile`
- [ ] Add `todo.write` aliases:
  - `todo`
  - `Todo`
  - `todo_write`
  - `TodoWrite`
  - `write_todo`
- [ ] Add `git.status` aliases:
  - `git_status`
  - `GitStatus`
  - `status`
- [ ] Add case-insensitive resolution.
- [ ] Add snake/dot conversion.
- [ ] Add fuzzy suggestion without unsafe auto-execution.

Acceptance:

- [ ] All aliases above resolve to canonical tools.
- [ ] Low confidence fuzzy match returns model-readable error/suggestion.

### B10 JSON Shape Error Catalog

- [ ] Implement optional null stripping.
- [ ] Implement stringified JSON array parsing.
- [ ] Implement empty object to array where schema expects array.
- [ ] Implement bare string to string array for safe list fields.
- [ ] Implement markdown link path unwrapping for path fields.
- [ ] Apply all repairs only after schema validation failure.
- [ ] Apply repairs only at issue paths.

Acceptance:

- [ ] Valid args are never mutated.
- [ ] Repair events include rule id and issue path.

### B11 Relational Invariants

- [ ] Implement RelationalInvariantResolver.
- [ ] Add safe defaults for missing `offset` where `limit` is present.
- [ ] Reject or repair `start > end` with model-readable guidance.
- [ ] Inject `base_hash` for file edit/write at runtime before dispatch.
- [ ] Keep base_hash injection outside model responsibility.

Acceptance:

- [ ] Model args without base_hash can dispatch with runtime-injected hash.
- [ ] Invalid ranges do not reach executor unmediated.

### B12 Tool Failure Doom Loop

- [ ] Preserve observation cache.
- [ ] Detect repeated same tool batch.
- [ ] Add "already observed" guidance instead of banning tools.
- [ ] Remove permanent tool-ban finalizer paths.
- [ ] Keep finalizer for budget/non-progress, not "disable useful tools forever".

Acceptance:

- [ ] Naming errors do not trigger tool ban.
- [ ] Repeated failed batch receives observation guidance.

### B13 Long Context Performance Bend

- [ ] Implement DeepSeek-aware compaction threshold:
  `min(192K, context_window * 0.75)`.
- [ ] Preserve recent 4 complete turns.
- [ ] Preserve latest raw reasoning needed for replay.
- [ ] Use Compactor role for compaction.
- [ ] Add `[compacted-context]` marker.

Acceptance:

- [ ] Prompt tokens above 192K trigger compaction.
- [ ] Compaction preserves latest replay-required reasoning.

## 4. Architecture Layer Tasks

### RuntimeFacade

- [ ] Keep RuntimeFacade as public API boundary.
- [ ] Route all agent turns into `AgentKernel::run_turn`.
- [ ] Emit event stream from RuntimeFacade.
- [ ] Remove direct policy ownership from RuntimeFacade.

### AgentKernel

- [ ] Create `crates/runtime/src/agent_kernel/`.
- [ ] Implement `AgentKernel`.
- [ ] Implement `TurnRequest`.
- [ ] Implement `TurnResult`.
- [ ] Implement turn orchestration:
  - route;
  - budget;
  - permission mode;
  - context manager;
  - evidence ledger;
  - compactor;
  - finalizer;
  - event log;
  - turn state.
- [ ] Ensure AgentKernel does not execute concrete tools directly.
- [ ] Ensure AgentKernel calls TCML for all model tool calls.

### NativeProfile

- [ ] Create `crates/runtime/src/native_profile/`.
- [ ] Implement `NativeProfile` trait.
- [ ] Implement factory for DeepSeek and Qwen.
- [ ] Add `DeepSeekProfile`.
- [ ] Add `QwenProfile`.
- [ ] Move DeepSeek-specific behavior into DeepSeekProfile.
- [ ] Add profile hooks:
  - prompt scaffold;
  - cache prefix;
  - tool schema policy;
  - role split;
  - temperature schedule;
  - reasoning replay;
  - stream processor.

### ProviderAdapter

- [ ] Keep HTTP/SSE transport separate from AgentKernel.
- [ ] Normalize provider stream chunks into StreamProcessor input.
- [ ] Add retry and timeout boundaries without hiding recoverable tool errors.
- [ ] Ensure provider wire format is not native model policy.

### TCML

- [ ] Create `crates/runtime/src/tcml/`.
- [ ] Implement TCML facade.
- [ ] Move alias, repair, relational, content extraction, error factory into TCML.
- [ ] Keep TCML unavoidable before PermissionGate/ToolDispatcher.

### PermissionGate

- [ ] Insert PermissionGate after TCML and before dispatch.
- [ ] Evaluate `PermissionMode`.
- [ ] Return Allow/Ask/Deny as structured event.
- [ ] Do not hide denied tools from manifest merely because they may be denied.

### ToolDispatcher

- [ ] Keep read-only tools parallelizable.
- [ ] Keep state-changing tools serialized.
- [ ] Preserve dispatch queue semantics.
- [ ] Attach tool lifecycle trace id to dispatch.

### ToolExecutors

- [ ] Keep concrete executor logic behind dispatcher.
- [ ] Return ToolExecutionResult into ResultFormatter.
- [ ] Do not return raw executor errors directly to model.

## 5. Data Model Tasks

### TurnState

- [ ] Define `TurnState`.
- [ ] Add identity fields:
  - `session_id`;
  - `turn_index`;
  - `started_at`.
- [ ] Add routing fields:
  - `route`;
  - `mode`;
  - `role`.
- [ ] Add budget fields:
  - `budget`;
  - `iterations`;
  - `tool_calls_used`;
  - `tokens_in`;
  - `tokens_out`;
  - `reasoning_tokens`.
- [ ] Add batch info:
  - `seen_tool_batches`;
  - `observation_cache`;
  - `last_tool_batch`.
- [ ] Add DeepSeek native fields:
  - `reasoning_replay`;
  - `stream_state`;
  - `provider_capabilities`.
- [ ] Add permission flow:
  - `awaiting_user`.
- [ ] Add emit tracking:
  - `emitted_event_count`.

Acceptance:

- [ ] TurnState replaces scattered loop variables.
- [ ] Reasoning tokens are tracked separately.

### ToolCallLifecycle

- [ ] Define `ToolCallPhase::Streaming`.
- [ ] Define `ToolCallPhase::Assembled`.
- [ ] Define `ToolCallPhase::NameResolved`.
- [ ] Define `ToolCallPhase::SchemaChecked`.
- [ ] Define `ToolCallPhase::Repaired`.
- [ ] Define `ToolCallPhase::PermissionEvaluated`.
- [ ] Define `ToolCallPhase::Dispatched`.
- [ ] Define `ToolCallPhase::Completed`.
- [ ] Define `ToolCallTrace`.
- [ ] Emit one structured event per phase transition.

Acceptance:

- [ ] GUI/TUI can render phase-by-phase lifecycle.
- [ ] ToolDoctor can inspect tool call traces.

### ModelReadableToolError

- [ ] Define fields:
  - `error_code`;
  - `tool_name`;
  - `short_message`;
  - `field_errors`;
  - `retry_hint`;
  - `retry_example`;
  - `retryable`;
  - `counts_against_budget`.
- [ ] Define error codes:
  - `UnknownTool`;
  - `PlanModeRequired`;
  - `PermissionDenied`;
  - `SafetyDenied`;
  - `SchemaValidationFailed`;
  - `MalformedJson`;
  - `RelationalInvariantFailed`;
  - `ToolExecutionFailed`;
  - `BudgetExhausted`.

Acceptance:

- [ ] Every code has standard hint and optional example.
- [ ] Naming/schema errors can be budget-free retryable errors.

### PermissionMode

- [ ] Implement `Default`.
- [ ] Implement `Plan`.
- [ ] Implement `AcceptEdits`.
- [ ] Implement `DontAsk`.
- [ ] Implement `BypassPermissions`.
- [ ] Ensure `BypassPermissions` is dev-only.

Acceptance:

- [ ] Default asks for write/shell.
- [ ] Plan denies write/shell with `plan.enter` guidance.

### AgentRole And RoleModelMap

- [ ] Implement `AgentRole::Executor`.
- [ ] Implement `AgentRole::Compactor`.
- [ ] Implement `AgentRole::Reviewer`.
- [ ] Implement `AgentRole::Titler`.
- [ ] Implement `AgentRole::Summarizer`.
- [ ] Implement `RoleModelMap`.

Acceptance:

- [ ] RoleSplit can choose model and temperature per role stage.

## 6. Baseline Phase

Before changing code:

- [ ] Run `python3 scripts/claudecode_gap_check.py`.
- [ ] Run relevant existing runtime tests.
- [ ] Inspect current `native_agent_loop.rs`.
- [ ] Inspect current `runtime_facade.rs`.
- [ ] Inspect current `tool_contract.rs`.
- [ ] Inspect current `tool_call_parser.rs`.
- [ ] Inspect current `deepseek_reasoning.rs`.
- [ ] Inspect current `thinking_chain.rs`.
- [ ] Inspect current `native_turn_controller.rs`.
- [ ] Inspect current `context_budget.rs`.
- [ ] Inspect current `prompt_assembler.rs`.
- [ ] Inspect current `tool_dispatcher.rs`.
- [ ] Inspect current `tool_execution.rs`.
- [ ] Record bypass paths.
- [ ] Record hardcoded schemas.
- [ ] Record current GUI/local API runtime bridge.
- [ ] Fill section 28 Baseline Notes.

Suggested commands:

```text
python3 scripts/claudecode_gap_check.py
rg -n "DeepSeek|reasoning_content|tool_calls|tool_exposure|non_progress|loop_guard|prompt-keyword|build_context_bundle" crates/runtime/src
rg -n "file.read|shell.command|search.ripgrep|native_readonly_provider_tool_schema_json|tool schema" crates/kernel/src crates/runtime/src crates/cli/src
rg -n "localRuntimeClient|local_api_server|tauri|adapter_not_connected|runtime:get-bootstrap" desktop scripts
```

Acceptance:

- [ ] Baseline notes are filled.
- [ ] No behavior change occurs before baseline is recorded.

## 7. Phase 1: Foundation Refactor

Goal: split monolithic `native_agent_loop.rs` into independently testable
components without changing external behavior.

### Files To Create

- [ ] `crates/runtime/src/agent_kernel/mod.rs`
- [ ] `crates/runtime/src/agent_kernel/kernel.rs`
- [ ] `crates/runtime/src/agent_kernel/turn_state.rs`
- [ ] `crates/runtime/src/agent_kernel/budget_policy.rs`
- [ ] `crates/runtime/src/agent_kernel/permission_policy.rs`
- [ ] `crates/runtime/src/agent_kernel/compactor.rs`
- [ ] `crates/runtime/src/native_profile/mod.rs`
- [ ] `crates/runtime/src/native_profile/deepseek/mod.rs`
- [ ] `crates/runtime/src/native_profile/deepseek/stream.rs`
- [ ] `crates/runtime/src/native_profile/deepseek/reasoning.rs`
- [ ] `crates/runtime/src/native_profile/deepseek/cache_prefix.rs`
- [ ] `crates/runtime/src/native_profile/deepseek/role_split.rs`
- [ ] `crates/runtime/src/native_profile/qwen/mod.rs`
- [ ] `crates/runtime/src/tcml/mod.rs`
- [ ] `crates/runtime/src/tcml/alias_registry.rs`
- [ ] `crates/runtime/src/tcml/repair_catalog.rs`
- [ ] `crates/runtime/src/tcml/relational_resolver.rs`
- [ ] `crates/runtime/src/tcml/content_extractor.rs`
- [ ] `crates/runtime/src/tcml/error_factory.rs`

### Move/Extract

- [ ] Extract AgentKernel struct and `run_turn`.
- [ ] Extract TurnState from scattered variables.
- [ ] Extract BudgetPolicy.
- [ ] Extract PermissionPolicy skeleton.
- [ ] Extract Compactor stub.
- [ ] Extract NativeProfile trait/factory.
- [ ] Extract DeepSeekProfile shell.
- [ ] Extract QwenProfile shell.
- [ ] Extract TCML pipeline shell.
- [ ] Keep behavior equivalent after each move.

Acceptance:

- [ ] Phase 1 does not intentionally change user-facing behavior.
- [ ] After each extraction, focused tests still pass.
- [ ] `native_agent_loop.rs` shrinks and delegates.

Checks:

```text
cargo test -p researchcode-runtime
python3 scripts/claudecode_gap_check.py
```

## 8. Phase 2: PermissionPolicy And Manifest Full-Open

Goal: align with ClaudeCode discipline: manifest stays complete; PermissionPolicy
decides execution.

TODO:

- [ ] Remove turn-state based manifest cutting.
- [ ] Remove prompt-keyword exposure control where it hides canonical tools.
- [ ] Ensure `shell.command` remains in manifest.
- [ ] Ensure write/edit tools remain in manifest when registered.
- [ ] Implement PermissionPolicy 5 modes:
  - `Default`;
  - `Plan`;
  - `AcceptEdits`;
  - `DontAsk`;
  - `BypassPermissions`.
- [ ] Default mode asks for write/shell.
- [ ] Plan mode denies write/shell with `PlanModeRequired` guidance.
- [ ] AcceptEdits allows file writes/edits but still asks for shell.
- [ ] DontAsk allows only allow-listed tools.
- [ ] BypassPermissions allows all only in dev/test explicitly.
- [ ] GUI approval path receives ask events similar to `plan_approval_pending`.
- [ ] Remove `non_progress_recovery_count` permanent tool-ban finalizer.
- [ ] Remove `loop_guard_recovery_count` permanent tool-ban finalizer.
- [ ] Replace permanent bans with observation guidance + budget finalizer.
- [ ] Treat the currently broken Phase 2 tests as explicit blockers:
  - `native_agent_loop_v2_aliases_shell_list_intent_to_repo_map_when_manifest_blocks_shell`;
  - `facade_deepseek_loop_reopens_completed_session_for_next_turn`;
  - `facade_generation_prompt_exposes_fastauto_write_tools`;
  - `facade_owns_deepseek_native_loop_session_events`;
  - `runtime_deepseek_token_budget_is_not_chat_sized_for_agent_work`.
- [ ] Update those five tests to assert permission/events/conversation-history
      behavior, not hidden-tool or old manifest-cut behavior.
- [ ] Mark `iteration_naming_error_results` as a transition patch only.
- [ ] Delete `iteration_naming_error_results` once TurnState observation cache +
      budget finalizer replaces the permanent-ban loop guard.
- [ ] Add a regression test proving naming errors are observation-guided and do
      not count as non-progress tool failure.

Acceptance:

- [ ] Tool name errors do not cause tool bans.
- [ ] Plan mode write is denied with retry guidance.
- [ ] Default mode write triggers ask.
- [ ] Manifest full-open behavior is tested.
- [ ] All five named broken tests are either passing or have a recorded blocker.
- [ ] No `iteration_naming_error_results` transition counter remains after the
      final Phase 2 architecture is complete.

Focused tests:

```text
naming_errors_do_not_trigger_tool_ban
iteration_naming_error_results_transition_removed
plan_mode_write_is_denied_with_retry_guidance
default_mode_write_triggers_ask
manifest_keeps_shell_visible_permission_denies_execution
native_agent_loop_v2_aliases_shell_list_intent_to_repo_map_when_manifest_blocks_shell
facade_deepseek_loop_reopens_completed_session_for_next_turn
facade_generation_prompt_exposes_fastauto_write_tools
facade_owns_deepseek_native_loop_session_events
runtime_deepseek_token_budget_is_not_chat_sized_for_agent_work
```

## 9. Phase 3: ConversationHistory Full Fidelity

Goal: preserve multi-turn tool reasoning and tool results exactly enough for
DeepSeek tool chains.

TODO:

- [ ] Implement `session.to_conversation_messages()`.
- [ ] Convert EventLog into OpenAI-compatible message format.
- [ ] Assistant message includes `tool_calls` when present.
- [ ] Assistant message includes `reasoning_content` when present.
- [ ] Tool message uses `tool_call_id`.
- [ ] Tool result is appended to conversation history.
- [ ] RuntimeFacade `build_context_bundle` keeps system/context construction only.
- [ ] RuntimeFacade no longer rebuilds lossy history.
- [ ] Preserve full turn 1 tool result for turn 2.
- [ ] Preserve reasoning replay requirements across history conversion.

Acceptance:

- [ ] Turn 2 sees turn 1 tool result.
- [ ] DeepSeek thinking tool turn can be serialized for next request.
- [ ] ConversationHistory is source of provider messages.

Focused tests:

```text
turn_two_sees_turn_one_tool_result
conversation_history_preserves_assistant_tool_calls
conversation_history_preserves_tool_call_id
conversation_history_preserves_deepseek_reasoning_content
```

## 10. Phase 4: Compactor And DeepSeek Threshold

Goal: implement DeepSeek-aware compaction instead of runaway context or crude
finalizer truncation.

TODO:

- [ ] Implement `agent_kernel/compactor.rs`.
- [ ] Trigger compaction when `tokens_in > min(192K, context_window * 0.75)`.
- [ ] Trigger compaction on explicit `/compact`.
- [ ] Preserve recent N pairs, with default N = 4 complete turns.
- [ ] Preserve latest raw reasoning required for replay.
- [ ] Call `ReasoningReplayManager.compact_old_reasoning`.
- [ ] Use `AgentRole::Compactor`.
- [ ] Use RoleSplit default Flash model for compactor.
- [ ] Mark summaries with `[compacted-context]`.
- [ ] Delete finalizer paths that existed only because compaction was missing.
- [ ] Add telemetry for compaction trigger and tokens freed.

Acceptance:

- [ ] Context > 192K triggers compaction.
- [ ] Latest required reasoning survives compaction.
- [ ] Compactor does not consume Executor budget.

Focused tests:

```text
context_above_192k_triggers_compaction
compaction_preserves_latest_reasoning_replay
compaction_uses_compactor_role
compacted_context_marker_is_inserted
```

## 11. Phase 5: NativeProfile Completion

Goal: make DeepSeek/Qwen native behavior live inside NativeProfile.

### DeepSeekProfile

- [ ] Move StreamProcessor into DeepSeekProfile.
- [ ] Move DsmlChunkFilter into DeepSeekProfile.
- [ ] Move ReasoningReplayManager into DeepSeekProfile.
- [ ] Move CachePrefixPolicy into DeepSeekProfile.
- [ ] Move RoleSplit into DeepSeekProfile.
- [ ] Add DeepSeek prompt scaffold.
- [ ] Add DeepSeek tool schema policy.
- [ ] Add DeepSeek budget policy.
- [ ] Add DeepSeek temperature schedule.
- [ ] Add DeepSeek content tool-call candidate extraction.
- [ ] Implement CachePrefixPolicy Zone A IMMUTABLE:
  - base system prompt for family;
  - tool catalog sorted by canonical id;
  - tool calling rules.
- [ ] Implement CachePrefixPolicy Zone B PER-SESSION:
  - PermissionMode and mode guidance;
  - project AGENTS.md / RESEARCHCODE.md;
  - workspace metadata/root path.
- [ ] Implement CachePrefixPolicy Zone C PER-TURN:
  - recent git status snapshot;
  - active plan/todo list;
  - conversation history.
- [ ] Sort Zone A tool catalog by `canonical_tool_id`.
- [ ] Sort Zone B project metadata by field name.
- [ ] Preserve AGENTS.md content exactly in Zone B.
- [ ] Emit `prompt_zone_a_hash`.
- [ ] Emit `prompt_zone_b_hash`.
- [ ] Emit `prompt_tokens_total`.
- [ ] Emit `prompt_tokens_cached_hint` when provider returns it.

### QwenProfile

- [ ] Implement QwenProfile shell from Phase 1.
- [ ] Add Qwen chat-template capability check.
- [ ] Add Qwen tool parser capability check.
- [ ] Add Qwen-specific prompt/tool policy.
- [ ] Ensure Qwen does not inherit DeepSeek S3/full scaffold by default.
- [ ] Ensure Qwen does not use DeepSeek reasoning replay blindly.

### Capability And Routing

- [ ] Implement ProviderCapabilityMatrix startup probe.
- [ ] Cache capability results by endpoint hash.
- [ ] Use capability matrix before selecting strict/tool/parser behavior.
- [ ] Expose capability status through ToolDoctor.

Acceptance:

- [ ] DeepSeek-specific code is not scattered across AgentKernel.
- [ ] Qwen has a separate native profile boundary.
- [ ] Provider capability is checked before feature assumptions.

Focused tests:

```text
deepseek_profile_owns_stream_processor
deepseek_profile_owns_reasoning_replay
qwen_profile_does_not_inherit_deepseek_scaffold
provider_capability_matrix_controls_strict_mode
```

## 12. Phase 6: ToolResult Format And Error Catalog

Goal: make tool outputs and errors directly readable by DeepSeek/Qwen.

### ResultFormatter

- [ ] Add ResultFormatter.
- [ ] Route every ToolExecutor output through ResultFormatter.
- [ ] Format `file.read` with path and line range header.
- [ ] Format `file.read` with numbered lines.
- [ ] Format `file.read` truncation hint with offset/limit guidance.
- [ ] Format `file.edit` with replacement count.
- [ ] Format `file.edit` with base_hash/new_hash.
- [ ] Format `file.edit` with diff.
- [ ] Format `shell.command` with command, exit code, duration.
- [ ] Format `shell.command` stdout last 80 lines.
- [ ] Format `shell.command` stderr even when empty.
- [ ] Format all errors uniformly.

### Error Catalog

- [ ] Implement `unknown_tool` template.
- [ ] Implement `plan_mode_required` template.
- [ ] Implement `schema_validation` template.
- [ ] Implement `permission_denied` template.
- [ ] Implement `safety_denied` template.
- [ ] Implement `malformed_json` template.
- [ ] Implement `relational_invariant_failed` template.
- [ ] Implement `tool_execution_failed` template.
- [ ] Implement `budget_exhausted` template.
- [ ] Implement `canonical_example_for(tool)`.
- [ ] Ensure retry examples are minimal valid args.

Acceptance:

- [ ] Every tool success has model-friendly preview.
- [ ] Every tool error has retry hint.
- [ ] DeepSeek can retry schema failure from standard template.

Focused tests:

```text
file_read_result_has_numbered_lines
file_edit_result_has_hashes_and_diff
shell_command_result_includes_empty_stderr
unknown_tool_error_has_retry_example
schema_error_has_canonical_minimal_args
```

## 13. Phase 7: Subagent `task.dispatch`

Goal: add ClaudeCode-style context isolation without making multi-agent the
default kernel design path.

TODO:

- [ ] Implement `task.dispatch` tool.
- [ ] Register `task.dispatch` in ToolManifest.
- [ ] Gate `task.dispatch` through PermissionPolicy if needed.
- [ ] Create child session isolated from parent context.
- [ ] Child session uses bounded task prompt.
- [ ] Child session can use RoleSplit::Compactor/Flash by default when appropriate.
- [ ] Parent receives summary string.
- [ ] Parent receives artifact refs.
- [ ] Parent does not inherit child raw context by default.
- [ ] EventLog links parent and child session ids.

Acceptance:

- [ ] Subagent has isolated context.
- [ ] Parent receives summary/artifacts only.
- [ ] Subagent dispatch is visible in GUI/TUI events.

Focused tests:

```text
task_dispatch_creates_child_session
task_dispatch_parent_receives_summary_and_artifacts
task_dispatch_does_not_leak_child_context_to_parent
```

## 14. Phase 8: Telemetry And ToolDoctor

Goal: make runtime behavior inspectable.

### Tool Lifecycle Events

- [ ] Emit `tool.call.streaming`.
- [ ] Emit `tool.call.assembled`.
- [ ] Emit `tool.name.resolved`.
- [ ] Emit `tool.schema.checked`.
- [ ] Emit `tool.input.repaired`.
- [ ] Emit `tool.permission.evaluated`.
- [ ] Emit `tool.dispatched`.
- [ ] Emit `tool.completed`.
- [ ] Emit `tool.error.model_readable`.

### ToolDoctor Commands

- [ ] Add `tool-doctor cache-status`.
- [ ] Add `tool-doctor alias-stats`.
- [ ] Add `tool-doctor repair-rate`.
- [ ] Add `tool-doctor unknown-tool-history`.
- [ ] Add `tool-doctor capabilities`.
- [ ] Add `tool-doctor manifest`.
- [ ] Add `tool-doctor stream`.

### GUI Diagnostics

- [ ] GUI displays tool lifecycle phases.
- [ ] GUI displays active permission mode.
- [ ] GUI displays active DeepSeek/Qwen profile.
- [ ] GUI displays cache prefix status.
- [ ] GUI displays capability matrix summary.
- [ ] GUI displays repair and alias stats.

Acceptance:

- [ ] ToolDoctor can explain why a tool did/did not execute.
- [ ] GUI can debug a failed tool call without reading raw logs.

Focused tests:

```text
tool_lifecycle_events_are_emitted_in_order
tool_doctor_reports_cache_status
tool_doctor_reports_alias_stats
tool_doctor_reports_unknown_tool_history
```

## 15. StreamProcessor Detail Tasks

- [ ] Define `StreamProcessor`.
- [ ] Define `StreamProcessorState`.
- [ ] Define `StreamEvent::VisibleDelta`.
- [ ] Define `StreamEvent::ReasoningDelta`.
- [ ] Define `StreamEvent::ToolCallPartial`.
- [ ] Define `StreamEvent::ToolCallAssembled`.
- [ ] Define `StreamEvent::ContentToolCallCandidate`.
- [ ] Define `StreamEvent::StreamCompleted`.
- [ ] Ingest `delta.reasoning_content`.
- [ ] Append reasoning delta to reasoning buffer.
- [ ] Ingest `delta.tool_calls[i]`.
- [ ] Merge tool-call deltas into accumulator.
- [ ] Emit partial event before completion.
- [ ] Emit assembled event when complete.
- [ ] Ingest `delta.content`.
- [ ] Filter DSML from visible text.
- [ ] Append filtered text to visible buffer.
- [ ] On finish_reason, finalize stream.
- [ ] If finish_reason=stop and tool_calls empty, scan content for candidates.

Acceptance:

- [ ] Visible stream never includes filtered DSML content.
- [ ] Reasoning stream is separate from visible stream.
- [ ] Tool call stream can be assembled incrementally.

## 16. DSML Filter Detail Tasks

- [ ] Implement cross-chunk `inside` state.
- [ ] Recognize start markers:
  - `<｜｜DSML｜｜tool_calls>`;
  - `<tool_call>`;
  - `<|tool_calls_section_begin|>`.
- [ ] Recognize end markers:
  - `</｜｜DSML｜｜tool_calls>`;
  - `</tool_call>`;
  - `<|tool_calls_section_end|>`.
- [ ] Filter content while inside DSML section.
- [ ] Preserve non-DSML visible text around markers.
- [ ] Emit leak count telemetry.
- [ ] Emit recovered count telemetry.

Acceptance:

- [ ] DSML beginning in chunk N and ending in chunk N+5 is fully hidden.
- [ ] Non-DSML text before/after markers remains visible.

## 17. ToolCallAccumulator Detail Tasks

- [ ] Define `PartialToolCall`.
- [ ] Store partials by `index`.
- [ ] Store id if present.
- [ ] Store name if present.
- [ ] Append args buffer.
- [ ] Track started_at.
- [ ] Parse only when args buffer is closed JSON.
- [ ] Finalize all at stream end.
- [ ] Convert unclosed args to ModelReadableToolError.

Acceptance:

- [ ] One index with fragmented args assembles.
- [ ] Two indices remain separate.
- [ ] Invalid final JSON becomes recoverable error.

## 18. ContentToolCallExtractor Detail Tasks

- [ ] Define `ExtractedContentCall`.
- [ ] Store raw text.
- [ ] Store tool name.
- [ ] Store args text.
- [ ] Store confidence.
- [ ] Store extraction pattern.
- [ ] Scan DSML tags.
- [ ] Scan `<tool_calls>` style tags.
- [ ] Scan `<|tool_call|>` style markers.
- [ ] Require JSON-like args.
- [ ] Return candidate only.
- [ ] If route expects tools and structured tool_calls are empty, pass candidate into TCML.
- [ ] If candidate is state-changing, force PermissionMode.Default ask path.
- [ ] Otherwise emit event and ignore.

Acceptance:

- [ ] State-changing content candidate never dispatches directly.
- [ ] Candidate event is visible to ToolDoctor.

## 19. TCML Pipeline Detail Tasks

Implement this exact path:

- [ ] Input: `StreamEvent::ToolCallAssembled`.
- [ ] Step 1: `AliasRegistry.resolve(name)`.
- [ ] Step 1 failure: `UnknownTool`.
- [ ] Step 2: `SchemaValidator.validate(args)`.
- [ ] Step 3 if issues: `IssueGuidedRepairer.repair(args, issues)`.
- [ ] Step 4: validate repaired args again.
- [ ] Step 4 failure: `SchemaValidationFailed`.
- [ ] Step 5: `RelationalInvariantResolver.resolve(args)`.
- [ ] Step 6: `ProviderCapabilityMatrix.check_strict_required`.
- [ ] Step 7: `PermissionPolicy.evaluate(call, mode)`.
- [ ] Step 8 if allow: `ToolDispatcher.dispatch_or_queue`.
- [ ] Step 9: `ToolExecutor.execute`.
- [ ] Step 10: `ResultFormatter.format`.
- [ ] Step 11: append tool result to ConversationHistory.
- [ ] No shortcut path allowed.
- [ ] base_hash injection occurs in Step 5 before dispatch.

Acceptance:

- [ ] Every accepted call has lifecycle phases.
- [ ] Every rejected call has ModelReadableToolError.
- [ ] No executor receives unmediated args.

## 20. RepairCatalog Detail Tasks

- [ ] Define `RepairRule`.
- [ ] Add `strip_optional_null`.
- [ ] Add `parse_stringified_array`.
- [ ] Add `wrap_bare_string_to_array`.
- [ ] Add `unwrap_markdown_link_path`.
- [ ] Add `empty_object_to_array`.
- [ ] Add `never_apply_to`.
- [ ] Add never repair `file.write.content`.
- [ ] Add never repair `shell.command.command`.
- [ ] Run schema validation before repair.
- [ ] Collect validation issues.
- [ ] Apply at most issue-local repair.
- [ ] Validate repaired args again.
- [ ] Return ModelReadableToolError if still invalid.

Acceptance:

- [ ] `file.write.content` is never modified.
- [ ] `shell.command.command` is never modified.
- [ ] Valid input is unchanged.

## 21. File-Level Change Checklist

These are doc39 §16 file-level changes.

- [ ] `crates/runtime/src/native_agent_loop.rs`
  - [ ] split into `agent_kernel/` files;
  - [ ] leave final kernel entry under 400 lines where practical;
  - [ ] remove scattered DeepSeek-specific branches.
- [ ] `crates/runtime/src/runtime_facade.rs`
  - [ ] remove prompt-keyword exposure logic;
  - [ ] remove lossy history part of `build_context_bundle`;
  - [ ] route through AgentKernel.
- [ ] `crates/runtime/src/tool_contract.rs`
  - [ ] split alias/repair/relational into `tcml/`;
  - [ ] preserve `mediate_tool_call` or equivalent TCML entry.
- [ ] `crates/runtime/src/tool_call_parser.rs`
  - [ ] move or wrap under `tcml/`;
  - [ ] integrate with AliasRegistry and StreamProcessor.
- [ ] `crates/runtime/src/deepseek_reasoning.rs`
  - [ ] move into `native_profile/deepseek/reasoning.rs`.
- [ ] `crates/runtime/src/thinking_chain.rs`
  - [ ] move into `native_profile/deepseek/stream.rs`.
- [ ] `crates/runtime/src/native_turn_controller.rs`
  - [ ] merge into `agent_kernel/turn_state.rs`.
- [ ] `crates/runtime/src/context_budget.rs`
  - [ ] move DeepSeek budget logic into `native_profile/deepseek/budget.rs`;
  - [ ] add Qwen budget path.
- [ ] `crates/runtime/src/prompt_assembler.rs`
  - [ ] move cache prefix logic into NativeProfile;
  - [ ] move conversation history conversion into `agent_kernel/conversation_history.rs`.
- [ ] `crates/runtime/src/tool_dispatcher.rs`
  - [ ] preserve dispatcher;
  - [ ] add PermissionGate integration.
- [ ] `crates/runtime/src/tool_execution.rs`
  - [ ] add ResultFormatter integration.
- [ ] `crates/kernel/src/tool.rs`
  - [ ] add full alias list;
  - [ ] preserve canonical tool definitions.
- [ ] `crates/runtime/src/observation_cache.rs`
  - [ ] preserve observation cache;
  - [ ] ensure repeated calls are guided, not banned.

## 22. Qwen Native Boundary

doc39 is DeepSeek-first but includes Qwen as native profile. This section ensures
Qwen is not lost.

- [ ] QwenProfile exists as first-class NativeProfile.
- [ ] QwenProfile has chat-template capability check.
- [ ] QwenProfile has parser capability check.
- [ ] QwenProfile has dedicated tool prompt/schema policy.
- [ ] QwenProfile does not receive DeepSeek S3/full scaffold by default.
- [ ] QwenProfile does not use DeepSeek reasoning replay behavior unless a Qwen
      capability explicitly requires equivalent handling.
- [ ] Qwen eval fixtures are separate from DeepSeek eval fixtures.
- [ ] Qwen compatible endpoint remains compatible-only unless native capability
      evidence exists.

Acceptance:

- [ ] Qwen native boundary is implemented, even if DeepSeek has more mature
      primitive coverage first.

## 23. Final Architectural Invariants

These must be true at final completion:

1. [ ] DeepSeek-native kernel = Claude-Code-grade discipline + DeepSeek-shaped
       primitives.
2. [ ] ToolManifest is not casually cut by turn state.
3. [ ] PermissionPolicy decides execution permission.
4. [ ] TCML cannot be bypassed.
5. [ ] EventLog is the single truth source.
6. [ ] Automatic compaction exists.
7. [ ] Subagent dispatch has context isolation.
8. [ ] StreamProcessor handles reasoning, visible content, tool deltas, and DSML.
9. [ ] ReasoningReplayManager preserves raw replay content when required.
10. [ ] CachePrefixPolicy uses three zones.
11. [ ] AliasRegistry covers DeepSeek high-frequency wrong names.
12. [ ] RepairCatalog covers DeepSeek/Qwen real JSON shape failures.
13. [ ] RoleSplit maps Pro/Flash by role.
14. [ ] TemperatureSchedule maps temperature by stage.
15. [ ] ProviderCapabilityMatrix gates strict/tool/parser assumptions.
16. [ ] Compactor triggers around 192K or 75% context window.
17. [ ] ToolResult format is DeepSeek-friendly.
18. [ ] ModelReadableToolError has standard templates.
19. [ ] GUI/TUI can inspect lifecycle events.
20. [ ] Eval gates block native promotion on wrong-tool/replay/repair failures.

## 24. Risk Register Tasks

For each doc39 risk:

### native_agent_loop Refactor Regression

- [ ] Strictly keep Phase 1 behavior-preserving.
- [ ] Run tests after each move.
- [ ] Record any behavior change explicitly.

### Reasoning Replay Loss

- [ ] Add unit test for thinking + tool_use + next turn.
- [ ] Add test for raw reasoning not sanitized before replay.

### CachePrefixPolicy Regression

- [ ] Treat old cache as naturally expiring.
- [ ] Add zone hash tests.

### Flash Compactor Quality

- [ ] Make Compactor model configurable.
- [ ] Add ability to switch compactor to Pro.
- [ ] Add eval or A/B fixture for summary quality where possible.

### AliasRegistry False Mapping

- [ ] Add confidence on fuzzy suggestions.
- [ ] Low confidence must return ModelReadableToolError, not auto-execute.

### ContentToolCallExtractor False Positive

- [ ] Trigger only when structured tool_calls are empty and finish_reason=stop.
- [ ] Apply high threshold.
- [ ] State-changing candidates go through ask path.

### Compactor Summary Loss

- [ ] Preserve recent 4 full turns.
- [ ] Write strict compactor prompt.
- [ ] Add regression fixture.

### Strict Probe Misclassification

- [ ] Cache probe result with ttl.
- [ ] Allow explicit override.

### Phase Ordering Breaks Usability

- [ ] After Phase 1-3, run a full user scenario.
- [ ] Do not pile all 8 phases without intermediate verification.

## 25. Eval Gates

These are doc39 R1-R10. Native promotion cannot pass unless they pass.

- [ ] R1 reasoning_content replay:
  - [ ] simulate thinking + tool_use three turns;
  - [ ] fourth turn request body contains correct reasoning_content.
- [ ] R2 DSML cross-chunk visible suppression:
  - [ ] DSML starts in chunk N;
  - [ ] DSML ends in chunk N+5;
  - [ ] hidden middle content never appears in visible buffer.
- [ ] R3 tool_calls.delta accumulation:
  - [ ] five chunks produce one finalized closed JSON args object.
- [ ] R4 ContentToolCallExtractor state-changing safety:
  - [ ] content containing `<tool_call>file.write {...}</tool_call>` yields candidate;
  - [ ] candidate does not dispatch directly.
- [ ] R5 AliasRegistry coverage:
  - [ ] 50+ DeepSeek high-frequency aliases resolve to canonical ids.
- [ ] R6 RepairCatalog dangerous-field safety:
  - [ ] `file.write.content` containing stringified array is not changed;
  - [ ] `file.write.content` containing null is not changed;
  - [ ] `file.write.content` containing markdown link is not changed.
- [ ] R7 CachePrefixPolicy stable ordering:
  - [ ] same manifest with different input ordering yields same zone_a hash.
- [ ] R8 RoleSplit compactor Flash:
  - [ ] compaction request uses Flash endpoint by default.
- [ ] R9 192K compaction:
  - [ ] prompt_tokens=193K emits `compaction.completed` before next request.
- [ ] R10 base_hash runtime injection:
  - [ ] model args omit base_hash;
  - [ ] dispatch-time args include runtime-computed base_hash.

## 26. Telemetry Tasks

Implement or record these metrics:

- [ ] `deepseek.cache.zone_a_hit_rate`
- [ ] `deepseek.cache.zone_b_hit_rate`
- [ ] `deepseek.reasoning.tokens_per_turn`
- [ ] `deepseek.reasoning.replay_count`
- [ ] `deepseek.reasoning.replay_size_kb`
- [ ] `deepseek.dsml.leak_chunks_count`
- [ ] `deepseek.dsml.leak_recovered`
- [ ] `deepseek.tool_call.partial_chunks_avg`
- [ ] `deepseek.tool_call.assembly_latency_ms`
- [ ] `deepseek.alias.resolution_count_by_alias`
- [ ] `deepseek.repair.rule_applied_count_by_rule`
- [ ] `deepseek.repair.success_rate`
- [ ] `deepseek.compaction.triggers_count`
- [ ] `deepseek.compaction.tokens_freed`
- [ ] `deepseek.role_split.executor_calls`
- [ ] `deepseek.role_split.compactor_calls`
- [ ] `deepseek.role_split.flash_savings_estimate_usd`
- [ ] Write daily aggregate to `~/.researchcode/telemetry/{date}.jsonl` if local
      telemetry storage is available and allowed.
- [ ] ToolDoctor reads telemetry.

Acceptance:

- [ ] ToolDoctor can report every metric family or explain unavailable source.

## 27. GUI/TUI Integration Tasks

doc39 references GUI/TUI rendering through structured events. Make it real.

- [ ] TUI consumes AgentKernel event stream.
- [ ] GUI consumes same event stream.
- [ ] GUI displays VisibleDelta separately from ReasoningDelta.
- [ ] GUI can collapse/preview reasoning via sanitized preview.
- [ ] GUI displays tool lifecycle phase transitions.
- [ ] GUI displays PermissionMode and permission asks.
- [ ] GUI displays ProviderCapabilityMatrix summary.
- [ ] GUI displays cache prefix status.
- [ ] GUI displays compaction events.
- [ ] GUI displays subagent parent/child relation.
- [ ] GUI displays ToolDoctor diagnostics.
- [ ] local API returns explicit adapter/runtime errors, never fabricated assistant text.

Acceptance:

- [ ] `npm run tauri:dev` path reaches real RuntimeFacade/AgentKernel.
- [ ] Tool failure appears as structured recoverable event.

## 28. Baseline Notes

Fill before implementation.

```text
Date: 2026-05-10
Branch: spike/next-step

Baseline commands:
- python3 scripts/claudecode_gap_check.py: passed; all listed implemented/partial checks reported ok.
- cargo test -p researchcode-runtime --lib: failed; 317 passed, 2 failed.
- python3 scripts/check_all.py: not run yet in this baseline slice.

Current bypass paths:
- Current native loop still contains direct shell/list recovery branches and direct finalizer/recovery counters in `crates/runtime/src/native_agent_loop.rs`.

Hardcoded schema paths:
- `crates/kernel/src/tool.rs` still owns provider schema serialization including `native_readonly_provider_tool_schema_json`.
- `crates/cli/src/main.rs` contains TUI/provider schema smoke references to `file_read` and `search_ripgrep`.

Current DeepSeek scattered logic:
- `crates/runtime/src/native_agent_loop.rs`: native loop still owns most execution
  orchestration, but Phase 2 transition counters `loop_guard_recovery_count`,
  `non_progress_recovery_count`, and `iteration_naming_error_results` have been
  removed from the active runtime path.
- `crates/runtime/src/runtime_facade.rs`: `deepseek_runtime_max_tokens_for_prompt`, `deepseek_runtime_tool_exposure_for_prompt`.
- `crates/runtime/src/prompt_assembler.rs`, `context_budget.rs`, `provider_response_adapter.rs`, `deepseek_stream.rs`, `deepseek_reasoning.rs`.

Current DSML/reasoning files:
- `crates/runtime/src/deepseek_stream.rs`
- `crates/runtime/src/deepseek_reasoning.rs`
- `crates/runtime/src/thinking_chain.rs`
- `crates/runtime/src/tool_call_parser.rs`

Current GUI/local API bridge:
- `desktop/` and `scripts/local_api_server.py` exist in the checkout; GUI verification not run yet.

Known blockers:
- `native_agent_loop_v2_aliases_shell_list_intent_to_repo_map_when_manifest_blocks_shell`: old assertion expects `source=manifest_recovery`; current manifest-full path recovers through permission handling.
- `runtime_deepseek_token_budget_is_not_chat_sized_for_agent_work`: old assertion expects `8192` for a short DeepSeek prompt; current runtime uses `16384` minimum agent budget.
```

## 29. Progress Ledger

Update after every coherent slice.

```text
Phase: Phase 0 baseline + Phase 2 blocker cleanup
Status: completed
Changed files:
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/runtime_facade.rs
Tests:
- python3 scripts/claudecode_gap_check.py: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_aliases_shell_list_intent_to_repo_map_when_manifest_blocks_shell -- --exact: passed
- cargo test -p researchcode-runtime runtime_facade::tests::runtime_deepseek_token_budget_is_not_chat_sized_for_agent_work -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 319/319 before Phase 1
Doc39 sections covered:
- §15 Phase 2 blocker test update
- §17 risk register: phase ordering and regression control
Blockers:
- none for this slice
Next:
- Phase 1 foundation refactor module skeleton
```

```text
Phase: Phase 1 foundation refactor module skeleton
Status: completed
Changed files:
- crates/runtime/src/lib.rs
- crates/runtime/src/agent_kernel/*
- crates/runtime/src/native_profile/*
- crates/runtime/src/tcml/*
- crates/runtime/src/tool_contract.rs
Tests:
- cargo fmt: passed
- cargo test -p researchcode-runtime agent_kernel --lib: passed, 6/6
- cargo test -p researchcode-runtime native_profile --lib: passed, 7/7
- cargo test -p researchcode-runtime tcml --lib: passed, 2/2
- cargo test -p researchcode-runtime --lib: passed, 333/333
Doc39 sections covered:
- §2 architecture layers
- §3 data model scaffolding
- §4 StreamProcessor shell and DSML filter
- §6 ReasoningReplayManager shell
- §7 CachePrefixPolicy shell
- §8 RepairCatalog shell
- §9 AliasRegistry shell
- §11 RoleSplit + TemperatureSchedule shell
- §15 Phase 1 foundation refactor
- §16 file-level changes, first extraction targets
Blockers:
- none for this slice
Next:
- Start integrating these modules into the existing runtime path without changing behavior.
```

```text
Phase: Phase 1 runtime integration - TCML aliases, DeepSeek stream/reasoning/cache/role split
Status: completed
Changed files:
- crates/runtime/src/tool_call_parser.rs
- crates/runtime/src/tool_contract.rs
- crates/runtime/src/provider_response_adapter.rs
- crates/runtime/src/model_transcript.rs
- crates/runtime/src/native_profile/deepseek/stream.rs
- crates/runtime/src/native_profile/deepseek/reasoning.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/model_adapter.rs
- crates/runtime/src/prompt_assembler.rs
Tests:
- cargo fmt: passed
- cargo test -p researchcode-runtime tool_call_parser::tests::normalizes_patch_propose_to_apply -- --exact: passed
- cargo test -p researchcode-runtime tool_contract::tests::resolves_alias_to_canonical_tool -- --exact: passed
- cargo test -p researchcode-runtime tcml::alias_registry::tests::resolves_doc39_high_frequency_aliases -- --exact: passed
- cargo test -p researchcode-runtime native_profile::deepseek::stream::tests::dsml_filter_buffers_split_markers -- --exact: passed
- cargo test -p researchcode-runtime provider_response_adapter::tests::deepseek_live_deltas_hide_cross_chunk_dsml_tool_markup -- --exact: passed
- cargo test -p researchcode-runtime native_profile::deepseek::reasoning::tests::capture_raw_response_sanitizes_preview_only -- --exact: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_tool_result_continuation_uses_provider_names_and_openai_call_ids -- --exact: passed
- cargo test -p researchcode-runtime live_model_request::tests::deepseek_openai_tool_result_request_replays_reasoning_content -- --exact: passed
- cargo test -p researchcode-runtime model_adapter::tests::deepseek_native_plan_preserves_native_invariants -- --exact: passed
- cargo test -p researchcode-runtime prompt_assembler::tests::deepseek_prompt_preserves_reasoning_and_prefix_cache_rules -- --exact: passed
- cargo test -p researchcode-runtime prompt_assembler::tests::qwen_prompt_preserves_qwen_native_rules -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 336/336
Doc39 sections covered:
- §4 StreamProcessor DSML filter moved into DeepSeek native profile and used by provider adapter visible deltas.
- §6 ReasoningReplayManager now owns DeepSeek raw reasoning replay state in native loop.
- §7 CachePrefixPolicy now shapes DeepSeek prompt A/B cache zones.
- §9 AliasRegistry now mediates parser + tool contract alias resolution.
- §11 RoleSplit now feeds DeepSeek native model plan role model and temperature.
Blockers:
- Provider request body still does not carry role temperature as a first-class JSON field; current slice records it in PlannedModelCall and prompt metadata.
  Resolved later in "Phase 5 DeepSeek stage temperature in provider request body".
Next:
- Continue Phase 2 by replacing local observation cache with TurnState-owned observation state and keep budget finalizers as the only hard tool-disable path.
```

```text
Phase: Phase 2 loop guard transition counter removal
Status: completed
Changed files:
- crates/runtime/src/native_agent_loop.rs
Tests:
- cargo fmt: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_recovers_from_repeated_tool_batch -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 336/336
Doc39 sections covered:
- §8 Remove non_progress_recovery_count permanent tool-ban finalizer.
- §8 Remove loop_guard_recovery_count permanent tool-ban finalizer.
- §8 Delete iteration_naming_error_results transition counter.
Blockers:
- TurnState now owns seen tool batch signatures, but the read-only observation cache itself is still local to native_agent_loop.rs.
Next:
- Extract ToolObservationCache into AgentKernel/TurnState ownership and expose replayable telemetry for GUI/TUI.
```

```text
Phase: Phase 2 observation cache extraction + TCML repair catalog integration
Status: completed
Changed files:
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/observation_cache.rs
- crates/runtime/src/agent_kernel/turn_state.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/tcml/repair_catalog.rs
- crates/runtime/src/tool_contract.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo fmt: passed
- cargo test -p researchcode-runtime agent_kernel::observation_cache::tests::duplicate_read_observation_is_detected_by_stable_key -- --exact: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_suppresses_duplicate_observation_calls -- --exact: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_recovers_from_repeated_tool_batch -- --exact: passed
- cargo test -p researchcode-runtime tcml::repair_catalog::tests::dangerous_fields_are_in_never_repair_lists -- --exact: passed
- cargo test -p researchcode-runtime tool_contract::tests::file_write_content_is_not_repaired -- --exact: passed
- cargo test -p researchcode-runtime tool_contract::tests::shell_command_is_not_repaired -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 337/337
- python3 scripts/claudecode_gap_check.py: passed
Doc39 sections covered:
- §3 TurnState now owns ObservationCache.
- §8 Phase 2 duplicate observation suppression uses kernel state and budget finalizer path.
- §8 RepairCatalog is now used by tool_contract never-repair enforcement.
- §13 telemetry/replay remains append-only through existing events.
Blockers:
- GUI/TUI-specific rendering of observation-cache telemetry still needs explicit local API contract verification.
Next:
- Generate/replay event logs through RuntimeFacade/local API and verify GUI-consumable event deltas for permission/tool/model events.
```

```text
Phase: Provider schema, GUI/local API contract, replay validation, broad harness
Status: completed
Changed files:
- crates/kernel/src/tool.rs
- crates/cli/src/main.rs
- scripts/local_api_server.py
- scripts/test_local_api_server.py
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo test -p researchcode-kernel --lib: passed, 19/19
- cargo test -p researchcode-runtime --lib: passed, 337/337
- cargo test --workspace: passed
- python3 -m unittest scripts/test_local_api_server.py: passed, 23/23
- python3 -m unittest scripts/test_local_api_http.py: passed, skipped=1
- node apps/desktop/test_local_api_client.mjs: passed
- node apps/desktop/test_static_mock_contract.mjs: passed
- cargo run -q -p researchcode-cli -- provider-tool-schema-smoke: passed
- cargo run -q -p researchcode-cli -- event-replay-smoke: passed
- cargo run -q -p researchcode-cli -- native-agent-loop-v2-smoke: passed
- cargo run -q -p researchcode-cli -- tool-contract-mediation-smoke: passed
- python3 scripts/claudecode_gap_check.py: passed
- python3 scripts/check_all.py: passed
Doc39 sections covered:
- §10 ProviderCapabilityMatrix boundary: provider schema now exposes only stable provider-visible tools and does not leak gated worktree tools.
- §14 ToolResult/event contract: replay/event validation remains GUI-consumable.
- §18 Eval gates: native profile promotion gate and parser/stream/scaffold checks passed.
- §20 GUI/TUI extension: local API and desktop static contracts passed against runtime-shaped events.
Blockers:
- Live provider health/live calls are still skipped by default because network is disabled by policy.
Next:
- Continue Phase 5 by making DeepSeek role temperature a first-class provider request field.
```

```text
Phase: Phase 5 DeepSeek stage temperature in provider request body
Status: completed
Changed files:
- crates/runtime/src/live_model_request.rs
- crates/runtime/src/live_model_executor.rs
Tests:
- rustfmt crates/runtime/src/live_model_request.rs crates/runtime/src/live_model_executor.rs: passed
- cargo test -p researchcode-runtime live_model_executor::tests::preflight_puts_deepseek_stage_temperature_in_provider_body: passed
- cargo test -p researchcode-runtime live_model_request::tests::builds_deepseek_openai_request_with_native_tools: passed
- cargo test -p researchcode-runtime --lib: passed, 338/338
- cargo run -q -p researchcode-cli -- live-model-preflight-smoke: passed
- cargo run -q -p researchcode-cli -- deepseek-request-builder-smoke: passed
- cargo run -q -p researchcode-cli -- deepseek-tool-result-continuation-smoke: passed
Doc39 sections covered:
- §1 B4 Temperature Sensitivity: DeepSeek execution/tool-calling request body now carries scheduled temperature.
- §11 RoleSplit + TemperatureSchedule: provider request construction reads DeepSeek schedule instead of leaving it only in prompt metadata.
Blockers:
- Role model name is still carried in planning metadata; endpoint/model switching itself remains a later RoleSplit routing slice.
Next:
- Continue with ProviderCapabilityMatrix/ToolDoctor or ConversationHistory full-fidelity, whichever gives the next largest kernel correctness gain.
```

```text
Phase: Phase 3 ConversationHistory event-log projection + ToolCallLifecycle assembled args
Status: completed
Changed files:
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/conversation_history.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/session.rs
- crates/runtime/src/runtime_facade.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- rustfmt crates/runtime/src/session.rs crates/runtime/src/runtime_facade.rs crates/runtime/src/native_agent_loop.rs crates/runtime/src/agent_kernel/conversation_history.rs: passed
- cargo test -p researchcode-runtime agent_kernel::conversation_history::tests::event_log_projects_tool_turn_into_openai_messages: passed
- cargo test -p researchcode-runtime session::tests::session_conversation_history_preserves_tool_result_pairing -- --exact: passed
- cargo test -p researchcode-runtime runtime_facade::tests::facade_projects_session_events_to_conversation_messages -- --exact: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_continues_after_tool_results -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 341/341
Doc39 sections covered:
- §3 Data Model: ConversationMessage and ConversationToolCall now exist under agent_kernel.
- §5 ToolCallLifecycle: `tool.call.assembled` records mediated tool id, argument hash, replayability, and safe replay arguments before `tool.call_requested`.
- §15 Phase 3: AgentSession can project EventLog into OpenAI-compatible user/assistant/tool messages.
- §20 GUI/TUI extension: RuntimeFacade exposes conversation messages from the same event source used by GUI event streaming.
Blockers:
- EventLog intentionally stores sanitized reasoning previews only; raw DeepSeek reasoning replay remains owned by ReasoningReplayManager and is not reconstructed from EventLog.
- Read-only tool arguments are now replayed from `tool.call.assembled`; write/shell/research side-effect arguments remain hash-only in EventLog until a secure non-read replay policy exists.
Next:
- Continue with ProviderCapabilityMatrix/ToolDoctor or secure side-effect argument artifact policy, whichever gives the next largest kernel correctness gain.
```

```text
Phase: Phase 5/8 ProviderCapabilityMatrix + ToolDoctor capability diagnostics
Status: completed
Changed files:
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/provider_capability.rs
- crates/runtime/src/live_model_executor.rs
- crates/cli/src/main.rs
- scripts/check_all.py
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- rustfmt crates/runtime/src/agent_kernel/mod.rs crates/runtime/src/agent_kernel/provider_capability.rs crates/runtime/src/live_model_executor.rs crates/cli/src/main.rs: passed
- cargo test -p researchcode-runtime agent_kernel::provider_capability -- --nocapture: passed, 5/5
- cargo test -p researchcode-runtime live_model_executor::tests::preflight_blocks_tool_request_when_capability_matrix_disables_tools -- --exact: passed
- cargo run -q -p researchcode-cli -- tool-doctor-capabilities-smoke: passed
- cargo run -q -p researchcode-cli -- agent-tui-ui-smoke: passed
- cargo test -p researchcode-runtime --lib: passed, 347/347
Doc39 sections covered:
- §10 ProviderCapabilityMatrix: native endpoints now produce deterministic capability records for tools, streaming tools, parallel tool calls, strict schema, reasoning replay, and parser identity.
- §14 Phase 8 Telemetry/ToolDoctor: CLI and TUI Doctor can report capability status without inspecting raw logs.
- §19 TCML Pipeline Step 6: live model preflight now checks provider capabilities before building a tool-enabled request.
Blockers:
- The current probe is offline and deterministic; network startup probe plus 24h persisted cache remains a later slice.
- `tool_choice=required`, tool_choice-specific, and strict JSON schema are intentionally false until proven by probe/eval.
Next:
- Continue with full lifecycle telemetry ordering or secure side-effect argument artifact policy.
```

```text
Phase: Phase 8 Tool lifecycle telemetry ordering
Status: completed
Changed files:
- crates/runtime/src/tool_contract.rs
- crates/runtime/src/session.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/runtime_facade.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- rustfmt crates/runtime/src/tool_contract.rs crates/runtime/src/session.rs crates/runtime/src/native_agent_loop.rs crates/runtime/src/runtime_facade.rs: passed
- cargo test -p researchcode-runtime session::tests::tool_lifecycle_events_are_emitted_in_order -- --exact: passed
- cargo test -p researchcode-runtime tool_contract::tests -- --nocapture: passed, 16/16
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_continues_after_tool_results -- --exact: passed
- cargo test -p researchcode-runtime runtime_facade::tests::facade_projects_session_events_to_conversation_messages -- --exact: passed
- cargo test -p researchcode-runtime --lib: passed, 348/348
- cargo run -q -p researchcode-cli -- tool-contract-mediation-smoke: passed
- cargo run -q -p researchcode-cli -- event-invariant-smoke: passed
- cargo run -q -p researchcode-cli -- native-loop-v2-tool-error-continuation-smoke: passed
Doc39 sections covered:
- §5 ToolCallLifecycle: ordered assembled -> permission evaluated -> requested -> dispatched -> completed lifecycle is now expressible in EventLog.
- §14 Phase 8 Telemetry/ToolDoctor: model-readable TCML events now include `tool.name.resolved`, `tool.schema.checked`, and `tool.input.repaired`.
- §19 TCML Pipeline: native loop and RuntimeFacade principal execution paths emit permission/dispatched/completed phases for ToolDoctor inspection.
Blockers:
- Full GUI rendering of the new lifecycle events is not implemented yet; current GUI can consume the EventLog but does not display every phase as a dedicated visual row.
- Some synthetic/governance paths intentionally do not emit `tool.dispatched` when no executor dispatch happens, such as plan approval waits and policy blocks.
Next:
- Continue with secure side-effect argument artifact policy or GUI lifecycle visualization.
```

```text
Phase: Secure side-effect argument replay policy
Status: completed
Changed files:
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/tool_argument_policy.rs
- crates/runtime/src/session.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- rustfmt crates/runtime/src/agent_kernel/mod.rs crates/runtime/src/agent_kernel/tool_argument_policy.rs crates/runtime/src/session.rs: passed
- cargo test -p researchcode-runtime agent_kernel::tool_argument_policy -- --nocapture: passed, 2/2
- cargo test -p researchcode-runtime session::tests::side_effect_tool_arguments_are_summary_only_in_event_log -- --exact: passed
- cargo test -p researchcode-runtime conversation_history -- --nocapture: passed, 2/2
- cargo test -p researchcode-runtime --lib: passed, 351/351
Doc39 sections covered:
- §5 ToolCallLifecycle: `tool.call.assembled` now carries full args only for replayable read-only calls and summary-only metadata for side-effect calls.
- §15 ConversationHistory: provider replay still ignores non-replayable side-effect arguments, preserving the safe event-log projection boundary.
- §24 Security/permissions invariant: command/content/edit payloads are redacted from side-effect argument summaries.
Blockers:
- Side-effect summaries are diagnostic-only, not reversible artifacts; this is intentional until a separate encrypted/local-only artifact policy is defined.
Next:
- Continue with GUI lifecycle visualization or persistent ProviderCapabilityMatrix probe cache.
```

```text
Phase: GUI lifecycle visualization contract
Status: completed
Changed files:
- desktop/local_api_client.mjs
- apps/desktop/local_api_client.mjs
- desktop/test_local_api_client.mjs
- apps/desktop/test_local_api_client.mjs
- desktop/src/components/AppShell.tsx
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- node desktop/test_local_api_client.mjs: passed
- node apps/desktop/test_local_api_client.mjs: passed
- npm run build (desktop): passed
Doc39 sections covered:
- §14 Phase 8 GUI Diagnostics: workflow panels now preserve per-tool lifecycle phases and argument replayability/summary metadata.
- §20 GUI/TUI extension: React runtime event handling surfaces assembled, permission evaluated, dispatched, and completed phases in progress state.
Blockers:
- The GUI still renders lifecycle phases as compact progress items, not a dedicated inspector timeline component.
Next:
- Continue with persistent ProviderCapabilityMatrix probe cache or final broad harness.
```

```text
Phase: Persistent ProviderCapabilityMatrix cache API
Status: completed
Changed files:
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/provider_capability.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- rustfmt crates/runtime/src/agent_kernel/mod.rs crates/runtime/src/agent_kernel/provider_capability.rs: passed
- cargo test -p researchcode-runtime agent_kernel::provider_capability -- --nocapture: passed, 8/8
- cargo test -p researchcode-runtime --lib: passed, 354/354
Doc39 sections covered:
- §10 ProviderCapabilityMatrix: cache file path, write, read, TTL expiry, and matrix cache-priority lookup now exist.
Blockers:
- Live network startup probe is still not enabled; the cache API is ready, but current runtime still uses offline deterministic probing unless an explicit cache root is provided.
Next:
- Run final broad harness and record remaining blockers.
```

```text
Phase: Final broad harness
Status: completed
Changed files:
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- python3 scripts/check_all.py: passed; final line `all checks passed`
Doc39 sections covered:
- End-to-end validation across parser, stream, native profiles, RuntimeFacade, tool contract, ToolDoctor, GUI/local API contract, event replay, provider sidecar gates, native loop recovery, and persisted event fixtures.
Blockers:
- No harness blocker remains after this pass.
Next:
- Move from architecture convergence into live-provider/GUI product hardening.
```

```text
Phase: shell.command usability correction
Status: completed
Changed files:
- crates/runtime/src/permission.rs
- crates/runtime/src/command.rs
- crates/runtime/src/tool_execution.rs
- crates/runtime/src/tool_harness.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo test -p researchcode-runtime command::tests -- --nocapture: passed, 11/11
- cargo test -p researchcode-runtime permission::tests -- --nocapture: passed, 6/6
- cargo test -p researchcode-runtime tool_execution::tests::apply_mode_executes_shell_only_with_permission -- --exact --nocapture: passed
- cargo test -p researchcode-runtime tool_execution::tests::shell_nonzero_exit_is_observation_not_tool_failure -- --exact --nocapture: passed
- cargo test -p researchcode-runtime tool_execution::tests::shell_hard_deny_is_not_reported_as_permission_prompt -- --exact --nocapture: passed
- cargo run -q -p researchcode-cli -- classify-command 'rg token . | head': ask
- cargo run -q -p researchcode-cli -- classify-command 'cd crates/runtime && cargo test -p researchcode-runtime command::tests': ask
- cargo run -q -p researchcode-cli -- classify-command 'echo ok; curl https://example.com': deny
- cargo run -q -p researchcode-cli -- tool-harness-smoke: passed, 58/58
- cargo test -p researchcode-runtime --lib: passed, 358/358
- python3 scripts/check_all.py: passed; final line `all checks passed`
Doc39 sections covered:
- §14.3 shell.command format and tool observation semantics.
- §23 final architectural invariants: shell is now permission-gated execution feedback, not a fake argv-only executor.
Blockers:
- No harness blocker remains.
Next:
- Add live GUI permission-card QA for shell commands with pipelines, cwd changes, nonzero test exits, and timeout cancellation.
```

```text
Phase: file.read recoverable observation correction
Status: completed
Changed files:
- crates/runtime/src/tool_execution.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/runtime_facade.rs
- crates/cli/src/main.rs
- scripts/local_api_server.py
- scripts/test_local_api_server.py
- scripts/test_local_api_http.py
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo fmt -- crates/runtime/src/tool_execution.rs crates/runtime/src/native_agent_loop.rs crates/runtime/src/runtime_facade.rs crates/cli/src/main.rs: passed
- cargo test -p researchcode-runtime tool_execution::tests::file_read_directory_error_suggests_directory_tool -- --exact --nocapture: passed
- cargo test -p researchcode-runtime tool_execution::tests::file_read_missing_path_is_recoverable_observation -- --exact --nocapture: passed
- cargo run -q -p researchcode-cli -- loop-recovery-directory-smoke: passed
- python3 -m unittest scripts/test_local_api_server.py scripts/test_local_api_http.py: passed, 25 tests, 1 skipped
- cargo test -p researchcode-runtime --lib: passed, 359/359
- python3 scripts/check_all.py: passed; final line `all checks passed`
Doc39 sections covered:
- Tool observation semantics: recoverable file.read path mistakes are returned as model-readable observations, not failed tool lifecycle events.
- RuntimeFacade memory: recoverable path observations still update path-correction state even when lifecycle ok=true.
- GUI/local API bridge: directory, missing, and non-file read previews now return structured recoverable observations instead of throwing bridge errors.
- Native loop recovery: file.read directory observations still trigger directory auto-recovery after ok=true normalization.
Blockers:
- No harness blocker remains.
Next:
- Run a live GUI stress replay and confirm directory/missing-path read cards render as recoverable observations instead of red failures.
```

```text
Phase: GUI shell permission approval surfacing
Status: completed
Changed files:
- desktop/src/components/AppShell.tsx
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- npm run build: passed in desktop/
- python3 -m unittest scripts/test_local_api_server.py scripts/test_local_api_http.py: passed, 25 tests, 1 skipped
- cargo test -p researchcode-runtime runtime_facade::tests::facade_fast_auto_runs_safe_command_but_blocks_hard_deny -- --exact --nocapture: passed
- node test_local_api_client.mjs: passed in desktop/
Doc39 sections covered:
- GUI/TUI permission boundary: the desktop GUI now starts runtime sessions in conservative mode so shell.command requires explicit user approval instead of safe-command FastAutoApply.
- GUI approval UX: pending permission requests now surface as a main-pane approval bar even if the right inspector is closed.
Blockers:
- No focused test blocker remains.
Next:
- Run npm run tauri:dev and verify a live /run or model-triggered shell.command produces the approval bar before execution.
```

```text
Phase: Provider tool surface and retryable tool observation correction
Status: completed
Changed files:
- crates/runtime/src/tool_contract.rs
- crates/runtime/src/native_agent_loop.rs
- crates/kernel/src/tool.rs
- desktop/src/components/AppShell.tsx
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo fmt -- crates/kernel/src/tool.rs crates/runtime/src/tool_contract.rs crates/runtime/src/native_agent_loop.rs: passed
- npm run build: passed in desktop/
- cargo test -p researchcode-kernel tool::tests -- --nocapture: passed, 8/8
- cargo test -p researchcode-runtime tool_contract::tests -- --nocapture: passed, 17/17
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_converts_tool_error_to_tool_result -- --exact --nocapture: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_routes_plan_enter_to_plan_approval -- --exact --nocapture: passed
- node test_local_api_client.mjs: passed in desktop/
- cargo test -p researchcode-runtime --lib: passed, 360/360
- python3 -m unittest scripts/test_local_api_server.py scripts/test_local_api_http.py: passed, 25 tests, 1 skipped
- cargo run -q -p researchcode-cli -- tool-manifest-doctor-smoke: passed, provider_names=13
- cargo run -q -p researchcode-cli -- provider-tool-schema-smoke: passed
- cargo run -q -p researchcode-cli -- native-agent-loop-v2-smoke: passed
- python3 scripts/check_all.py: passed; final line `all checks passed`
Doc39 sections covered:
- Provider-visible tool manifest surface: preview-only and gated tools are no longer advertised to native model provider manifests.
- Research workflow boundary: research tools stay available only when the workflow state explicitly enters research mode.
- Retryable tool errors as observations: schema/manifest mistakes now produce recoverable model-readable observations instead of hard failed tool lifecycle completions.
- Plan approval path: plan.enter without a model-supplied plan payload routes to the plan approval interaction instead of schema rejection.
- GUI model-readable error display: retryable model-readable errors are shown as observations fed back to the model, not generic red tool errors.
Blockers:
- No harness blocker remains.
Next:
- Run a live GUI stress replay with the same prompts/screenshots and compare red failure counts against the exported event log.
```

```text
Phase: Uncapped tool use with progress-based convergence
Status: completed
Changed files:
- desktop/src-tauri/src/main.rs
- crates/runtime/src/agent_kernel/mod.rs
- crates/runtime/src/agent_kernel/turn_state.rs
- crates/runtime/src/native_agent_loop.rs
- crates/runtime/src/live_http_transport.rs
- docs/implementation/agent_kernel_tool_contract_long_task_todos.md
Tests:
- cargo fmt -- crates/runtime/src/agent_kernel/mod.rs crates/runtime/src/agent_kernel/turn_state.rs crates/runtime/src/native_agent_loop.rs crates/runtime/src/live_http_transport.rs: passed
- cargo fmt --manifest-path desktop/src-tauri/Cargo.toml: passed
- cargo test -p researchcode-runtime agent_kernel::turn_state::tests -- --nocapture: passed, 2/2
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_empty_visible_after_tools_finalizer_keeps_tool_evidence -- --exact --nocapture: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_suppresses_duplicate_observation_calls -- --exact --nocapture: passed
- cargo test -p researchcode-runtime native_agent_loop::tests::native_agent_loop_v2_survives_long_deepseek_tool_run_under_context_guard -- --exact --nocapture: passed
- cargo test --manifest-path desktop/src-tauri/Cargo.toml tests::desktop_agent_loop_keeps_tool_budget_uncapped -- --exact --nocapture: passed
- npm run build: passed in desktop/
- cargo test -p researchcode-runtime --lib: passed, 361/361
- cargo test --manifest-path desktop/src-tauri/Cargo.toml: passed, 1/1
- python3 scripts/check_all.py: passed; final line `all checks passed`
Doc39 sections covered:
- Tool pressure policy: desktop Tauri runtime keeps tool calls uncapped (`max_tool_calls=0`); convergence is controlled by progress quality, not an arbitrary GUI count.
- Progress controller: `TurnState` now tracks new evidence, recoveries, duplicate observations, errors, and consecutive no-progress/duplicate plateaus.
- Precision guard: native loop continues indefinitely while tool calls add new evidence, but switches to no-tool synthesis after repeated duplicate/no-progress iterations.
- Visible answer invariant: after tool execution, an empty/hidden continuation now carries the just-executed tool evidence into the visibility finalizer instead of losing it.
- GUI product safety: long GUI runs should terminate in a user-visible answer, approval pending state, or explicit blocked state when progress plateaus, without sacrificing useful deep tool exploration.
Blockers:
- No harness blocker remains.
Next:
- Run npm run tauri:dev and replay the screenshot stress prompt; verify the GUI reaches visible synthesis only after evidence stops improving, not after a fixed tool count.
```

```text
Phase:
Status: pending | in_progress | completed | blocked
Changed files:
- ...
Tests:
- ...
Doc39 sections covered:
- ...
Blockers:
- ...
Next:
- ...
```

## 30. Final Report Format

Final response after full completion must include:

- changed files;
- completed doc39 sections;
- completed phases;
- architectural invariants now enforced;
- DeepSeek-specific primitives implemented;
- Qwen native boundary implemented;
- tests/checks run;
- failed checks and exact blockers;
- eval gates passed;
- telemetry/tooldoctor status;
- risks remaining;
- unresolved questions;
- next recommended task.

## 31. One-Sentence Rule

不要让 DeepSeek 通过"更听话"来变强；要让 kernel 从 stream、reasoning、
cache、alias、repair、permission、compaction、telemetry 到 eval 都按 DeepSeek
的真实行为塑形。
