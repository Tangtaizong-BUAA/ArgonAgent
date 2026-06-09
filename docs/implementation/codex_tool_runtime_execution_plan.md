# Codex Tool Runtime Execution Plan

## Summary

This is the merged execution contract for:

- `tool-usage/deepseek_qwen_tool_calling_harness_engineering_mega_plan.md`
- `/Users/gongyuxuan/Documents/codex_tool_runtime_todos_and_execution_plan.md`

Primary objective: complete all tool-runtime hardening work for DeepSeek/Qwen native modes to engineering-grade reliability.

Completion rule:

- Do not stop at partial slices.
- Continue until all unblocked phase tasks are done.
- Only stop on explicit hard blockers (forbidden permission/dependency/network/secrets/history rewrite).

## Phase List (19)

### Phase 01: Preflight Inventory and Traceability
- Create and maintain:
  - `docs/implementation/tool_runtime_progress.md`
  - `docs/implementation/tool_runtime_inventory.md`
- Map mega-plan sections 0-31 to implementation tasks with status: `missing/partial/implemented/blocked`.

Acceptance:
- Inventory and progress files exist and are updated.
- Every mega-plan section has a traceable implementation target.

### Phase 02: Dynamic ToolManifest Single Source of Truth
- Build model-visible manifest only from ToolRegistry and runtime filters.
- Inputs: model family, provider capability, workflow state, task contract, permission summary, tool exposure mode.
- Record `tool.manifest.generated` and `manifest_hash`.

Acceptance:
- No prompt-only tools.
- Manifest/registry consistency passes ToolDoctor checks.

### Phase 03: Canonical Naming and Alias Registry
- Enforce canonical tool names for execution.
- Required aliases:
  - `read_source_code/read_file/readFile -> file.read`
  - `search_source_code/file.search/file.grep -> search.ripgrep`
  - `list_source_files/list_files/list_dir -> repo.map`
  - `shellCommand/shell.run -> shell.command`
  - `writeFile -> file.write`

Acceptance:
- Alias events emitted.
- Unknown alias no longer causes fatal runtime crash.

### Phase 04: UnknownTool Recovery
- Convert unknown tool path to recoverable `ModelReadableToolError`.
- Emit retry guidance and available-tool summary.
- Auto-trigger ToolDoctor snapshot for diagnosis.

Acceptance:
- Unknown tool never terminates runtime loop.
- TUI shows recovery card.

### Phase 05: Streaming Tool Call Accumulator
- Assemble streamed tool deltas before execution.
- Do not execute incomplete/malformed arguments.
- Emit tool-call delta lifecycle events.

Acceptance:
- Chunked tool arguments are correctly assembled.
- Incomplete JSON cannot execute.

### Phase 06: Validate-Then-Repair Core
- Validate raw input first.
- Repair only on validator issue paths.
- Revalidate after repair.
- Valid inputs must remain unchanged.

Acceptance:
- No global greedy preprocessor behavior.
- Repair telemetry recorded per issue path.

### Phase 07: P0 Repair Catalogue
- Implement safe repairs:
  - optional null strip
  - stringified array parse
  - empty object placeholder handling
  - bare string to array<string>
  - markdown path auto-link unwrap (path-only)
  - line-range defaults via relational resolver

Acceptance:
- All P0 repair smoke cases pass.

### Phase 08: Schema Primitive Layer
- Implement model-aware primitives:
  - `pathString`
  - `pathArray`
  - `fileContentString`
  - `commandString`
  - `lineRange`

Acceptance:
- Path repairs never mutate non-path fields.
- Content/command primitives remain non-repairable.

### Phase 09: RelationalInvariantResolver
- Handle semantic constraints:
  - offset/limit pairing
  - base-hash requirements
  - path-root/protected-path checks
  - mutually exclusive args

Acceptance:
- Defaults include model-visible notes.
- Semantic failures produce structured tool errors.

### Phase 10: ModelReadableToolError Standardization
- Replace raw validator blobs with structured, retryable tool errors.
- Include field-level hints and retry examples.

Acceptance:
- Model receives actionable retry format.
- Validator internals are not exposed as raw-only output.

### Phase 11: ToolCallLedger Exactly-Once Guarantees
- Track proposed vs completed tool results.
- Enforce exactly-one tool result per call.
- Emit missing/duplicate result diagnostics.

Acceptance:
- Ledger exactly-once smoke passes.

### Phase 12: DeepSeek Content Tool Fallback
- Parse DSML/XML/plain-text tool-call candidates.
- Execute only after full mediation pipeline and policy checks.

Acceptance:
- Content fallback cannot bypass validation/permission/FSM.

### Phase 13: Provider Capability Matrix
- Encode mode/capability checks for DeepSeek/Qwen providers.
- Validate parser/template/reasoning replay support on startup.

Acceptance:
- Capability mismatch is explicit and blocks unsafe native path.

### Phase 14: DeepSeek Reasoning Replay Manager
- Persist and replay reasoning metadata for tool continuation turns.
- Never replay reasoning as normal user-visible message.
- Detect missing replay before API call.

Acceptance:
- Reasoning replay continuity checks pass in loop tests.

### Phase 15: Qwen3.6-27B Native Path Hardening
- Keep Qwen parser/template/capability path isolated from DeepSeek.
- Enforce canonical native target checks.

Acceptance:
- Qwen continuation fixtures pass.
- Non-canonical native target is rejected in native mode.

### Phase 16: Workflow FSM Tool Exposure and Parallel Policy
- Expose minimal tool subsets per workflow state.
- Allow read-only parallel batches.
- Serialize state-changing tools.

Acceptance:
- Scheduler policy and runtime behavior agree.

### Phase 17: TUI Event Lifecycle Rendering
- Render key runtime lifecycle events:
  - manifest
  - alias/unknown
  - validation/repair
  - execution
  - ledger
  - retry/recovery
- Avoid silent `Doing...` periods when tool events exist.

Acceptance:
- TUI shows actionable lifecycle cards for failure/recovery paths.

### Phase 18: ToolDoctor + Telemetry + Eval Gates
- ToolDoctor checks:
  - registry/manifest mismatch
  - prompt/docs stale tool names
  - permission/task/workflow visibility mismatch
  - provider capability mismatch
- Telemetry events:
  - repair/unknown/alias/missing-result/replay-missing/fallback/mismatch
- Enforce release-blocking eval rules.

Acceptance:
- ToolDoctor smoke and eval gates pass.

### Phase 19: UltraPlan/UltraReview/AgentTeams Integration and Final Go/No-Go
- UltraPlan emits tool contract requirements for new/changed tools.
- UltraReview adds tool reliability summary and evidence guardrails.
- AgentTeams scheduler consumes reliability diagnostics.
- Update rules/docs and final go/no-go checklist.

Acceptance:
- Reliability failures cannot be misclassified as verified code findings.
- Final report includes completed tasks, files changed, tests, risks, blockers.

## Required Test Commands

- `cargo test --workspace`
- `cargo run -q -p researchcode-cli -- tool-manifest-doctor-smoke`
- `cargo run -q -p researchcode-cli -- tool-contract-mediation-smoke`
- `cargo run -q -p researchcode-cli -- unknown-tool-recovery-smoke`
- `cargo run -q -p researchcode-cli -- tool-input-repair-smoke`
- `cargo run -q -p researchcode-cli -- tool-ledger-exactly-once-smoke`
- `cargo run -q -p researchcode-cli -- deepseek-content-tool-fallback-smoke`
- `cargo run -q -p researchcode-cli -- qwen-tool-mediation-fixture-smoke`
- `cargo run -q -p researchcode-cli -- deepseek-multi-tool-continuation-smoke`
- `cargo run -q -p researchcode-cli -- qwen-tool-continuation-fixture-smoke`
- `cargo run -q -p researchcode-cli -- agent-tui-tool-chain-smoke`
- `cargo run -q -p researchcode-cli -- agent-tui-error-boundary-smoke`
- `cargo run -q -p researchcode-cli -- ultraplan-fixture-smoke`
- `cargo run -q -p researchcode-cli -- ultrareview-fixture-smoke`
- `cargo run -q -p researchcode-cli -- agentteam-smoke`
- `cargo run -q -p researchcode-cli -- evidence-ledger-smoke`
- `python3 scripts/check_all.py`

## Release-Blocking Rules

Block completion if any of the following occurs:

- UnknownTool causes fatal runtime error.
- Model-visible tool is not registry-backed.
- Valid inputs are mutated.
- `file.write.content` is transformed by repair.
- `shell.command.command` is auto-repaired.
- DeepSeek reasoning replay is dropped after tool call.
- TUI hides tool lifecycle during active tool loop.
- Tool result ledger has missing/duplicate entries.
- UltraReview classifies harness failure as verified code bug.

