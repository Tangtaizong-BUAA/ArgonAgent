# Tool Runtime Inventory

## 1. Repository structure summary

- Workspace root: `/Users/gongyuxuan/Documents/deep-code`
- Rust crates:
  - `crates/kernel`
  - `crates/runtime`
  - `crates/cli`
- Implementation target decision: modify existing runtime/kernel/cli crates (no parallel spike tree).

## 2. Existing tool runtime files

- Tool contract mediation (primary):
  - `crates/runtime/src/tool_contract.rs`
  - Contains:
    - `StreamingToolCallAccumulator`
    - `ToolCallLedger`
    - `ModelReadableToolError`
    - `mediate_tool_call`
    - `build_tool_manifest`
    - `run_tool_manifest_doctor`
- Tool parser:
  - `crates/runtime/src/tool_call_parser.rs`
  - Supports DSML/XML/JSON/function-call extraction and normalization.
- Tool scheduler/dispatcher:
  - `crates/runtime/src/tool_dispatcher.rs`
  - Current scheduler validates `find_tool_spec` and batches by concurrency safety.
- Tool execution:
  - `crates/runtime/src/tool_execution.rs`
  - `crates/runtime/src/executor.rs`

## 3. Existing kernel/spec files

- Tool spec source:
  - `crates/kernel/src/tool.rs`
  - Contains:
    - `core_tool_specs`
    - `provider_aliases`
    - provider schema builders
    - `tool_catalog_hash`
- Task contract:
  - `crates/kernel/src/task.rs`
  - Contains bounded autonomy checks and violations.

## 4. Existing state/workflow files

- Agent state machine:
  - `crates/runtime/src/state.rs`
  - Contains lifecycle and `can_transition`.
- Native loop runtime:
  - `crates/runtime/src/native_agent_loop.rs`
  - Contains loop v2 execution, tool-result continuation, recovery/finalizer paths, reasoning metadata handling.
- Runtime facade:
  - `crates/runtime/src/runtime_facade.rs`
  - Shared boundary for TUI/runtime state and loop invocation.

## 5. Existing provider/model adapter files

- Model adapter:
  - `crates/runtime/src/model_adapter.rs`
  - Contains native adapter planning and capability hint usage.
- Native provider and stream/normalizer:
  - `crates/runtime/src/native_provider.rs`
  - `crates/runtime/src/deepseek_stream.rs`
  - `crates/runtime/src/qwen_stream.rs`
  - `crates/runtime/src/native_response_normalizer.rs`

## 6. Existing TUI event rendering files

- TUI entry and event cards:
  - `crates/cli/src/main.rs`
  - Existing cards include:
    - `UnknownToolRecoveryCard`
    - `ToolValidationCard`
    - `ToolRepairCard`
    - `ToolLedgerCard`
    - `ManifestCard`
- Rendering currently driven by event-type string matching.

## 7. Existing tests and smokes (relevant subset)

- Tool mediation and recovery:
  - `tool-contract-mediation-smoke`
  - `tool-manifest-doctor-smoke`
  - `unknown-tool-recovery-smoke`
  - `tool-input-repair-smoke`
  - `tool-ledger-exactly-once-smoke`
- Native loops:
  - `deepseek-multi-tool-continuation-smoke`
  - `qwen-tool-continuation-fixture-smoke`
  - `deepseek-content-tool-fallback-smoke`
- TUI:
  - `agent-tui-tool-chain-smoke`
  - `agent-tui-error-boundary-smoke`

## 8. Stale tool names / compatibility names found

- `read_source_code` handling exists in parser/tool contract tests.
- Legacy names present in code/tests:
  - `read_file`
  - `readFile`
  - `search_source_code`
  - `list_source_files`
  - `shellCommand`
  - `writeFile`

## 9. Current maturity snapshot

- Strong partial implementation exists for phases:
  - UnknownTool recovery
  - basic repair and relational defaults
  - TUI recovery cards
  - native loop continuation/finalizer
- Completed in current iteration:
  - Context-aware dynamic manifest filtering by workflow/exposure/protocol
  - UnknownTool recovery enriched with `tool.doctor.snapshot` telemetry
  - Expanded canonical alias coverage for model-observed compatibility names
  - Workflow-state exposure constraints (`planning/reading/editing/testing/review`)
- Remaining improvement tracks (non-blocking to current phase completion):
  - Live provider HTTP 400 diagnostics on real DeepSeek/Qwen endpoints
  - TUI interaction polish for full ClaudeCode-style streaming UX parity

## 10. Risks and open questions

- Existing behavior is partially live; broad refactors in `native_agent_loop.rs` can regress passing smokes if not done incrementally.
- Tool naming policy keeps canonical runtime ids and expands alias ingestion; future docs should keep this explicit to avoid parser/prompt drift.
- Some release-blocking checks are documented but may still rely on convention rather than a single hard gate path.
