# Implementation Status

Last updated: 2026-05-08

## Current Decision

Phase 0 prototype gate and Phase 1 runtime-kernel gate have passed locally. The project is now in Phase 2: ClaudeCode-level reliability hardening for long-running DeepSeek/Qwen native agent work.

This is not a full ClaudeCode parity claim. The native runtime now has the core loop, event replay, permission resume, DeepSeek/Qwen request boundaries, 256K DeepSeek context guard, live-loop compaction preflight, reasoning replay, and streaming tool-call assembly. The remaining work is to make those paths survive real provider traffic, long-horizon sessions, GUI/TUI approval flows, and tool-contract repair/eval pressure at production depth.

Phase markers:

- Phase 0: schemas, prototypes, scaffold, deterministic fixtures. Passed.
- Phase 1: local runtime kernel, native agent loop, event replay, shared tool execution, permission resume, context guard. Passed for no-network fixtures and focused runtime tests.
- Phase 2: reliability hardening toward ClaudeCode parity. Current phase.
- Phase 3: live provider promotion, GUI command center, production persistence, and gated ecosystem expansion.

DeepSeek/Qwen native scaffold changes must still pass ContextBudgetManager, parser/stream/tool evals, and native promotion gates before promotion.

## Completed

### Phase 0 Schemas

- Kernel schemas under `docs/schemas/kernel/`.
- PlanApproval is separated from Permission.
- CompatibleProviderConfig and ModelAliasMapping schemas under `docs/schemas/provider/`.
- TaskContract schema under `docs/schemas/task_contract/`.

### Phase 0 Prototypes

- Event replay prototype: `docs/prototypes/event_log_replay/coding_task_sequence.jsonl`.
- Patch validator fixture/script: `eval/fixtures/patch/`, `scripts/prototype_patch_validator.py`.
- Command classifier fixture/script: `eval/fixtures/shell/permission_cases.json`, `scripts/prototype_command_classifier.py`.
- DeepSeek parser golden fixtures: `eval/fixtures/deepseek/parser_golden.json`.
- Qwen parser/executor golden fixtures: `eval/fixtures/qwen/parser_golden.json`.
- Research CSV profiler fixture/script: `eval/fixtures/research/csv-quality-small/`, `scripts/prototype_csv_profiler.py`.

### Scaffold

- Root `AGENTS.md`.
- Root `README.md`.
- Rust workspace:
  - `crates/kernel`
  - `crates/runtime`
  - `crates/cli`
- Desktop placeholder:
  - `apps/desktop`
- Research worker placeholder:
  - `workers/research_worker`
- Scaffold validator:
  - `scripts/validate_scaffold.py`
- SQLite storage draft:
  - `docs/storage/sqlite_schema_v0.sql`

### First Runtime Slice

- Rust permission classifier:
  - `crates/runtime/src/permission.rs`
- Product Kernel plan primitive:
  - `crates/kernel/src/plan.rs`
  - `cargo run -q -p researchcode-cli -- plan-smoke`
  - Adds validated `Plan` and `PlanStep` structures, progress counting, next-actionable-step selection, and context rendering through `ContextBundleBuilder::add_plan`.
  - Keeps PlanApproval as task-governance approval while giving future TUI/GUI/reviewer surfaces a concrete plan payload to display and summarize.
- Product Kernel memory primitive:
  - `crates/kernel/src/memory.rs`
  - `cargo run -q -p researchcode-cli -- memory-smoke`
  - Adds `MemoryItem` scopes for project facts, user preferences, model failure memory, repo facts, and research project memory.
  - Rejects secret-like memory content before it can be placed into ContextBundle memory items.
  - `ContextBundleBuilder::add_memory` lets long-task summaries and model failure lessons enter model context without inventing a separate prompt-only memory path.
- Context compaction:
  - `crates/runtime/src/compaction.rs`
  - `cargo run -q -p researchcode-cli -- compact-context-smoke`
  - Compaction summaries now preserve active plan, constraints, relevant files, latest tool evidence, pending permission context, recovery notes, next steps, and before/after token estimates.
  - Native DeepSeek loops now use live preflight compaction events before prepared model requests instead of relying only on offline `/compact`.
- Rust patch invariant validator:
  - `crates/runtime/src/patch.rs`
- Rust in-memory event log:
  - `crates/runtime/src/event_log.rs`
- JSONL event export/import:
  - `crates/runtime/src/event_log.rs`
- Rust agent state transition validator:
  - `crates/runtime/src/state.rs`
- Rust AgentSession core:
  - `crates/runtime/src/session.rs`
- Rust parser policy harness:
  - `crates/runtime/src/parser.rs`
- Structured ToolCall parser boundary:
  - `crates/runtime/src/tool_call_parser.rs`
  - Parses native JSON `tool_calls`, function-style string arguments, and DeepSeek XML/DSML fallback tool calls.
  - Normalizes `patch.propose` into the runtime `patch.apply` tool boundary after policy approval.
  - Repairs only trivial argument JSON trailing-comma cases and marks the parse as repaired for eval/telemetry.
  - Keeps policy decisions in `parser.rs` while keeping execution arguments out of raw-output string search.
- CLI runtime commands:
  - `crates/cli/src/main.rs`
- Kernel model/provider/task primitives:
  - `crates/kernel/src/model.rs`
  - `crates/kernel/src/task.rs`
  - TaskContract now validates bounded-autonomy path, tool, retry, parallel-agent, and stop-condition boundaries.
- Parser fixture policy validator:
  - `scripts/validate_parser_fixtures.py`
- Local artifact store primitives:
  - `crates/runtime/src/artifact.rs`
- Artifact manifest/index support:
  - `crates/runtime/src/artifact.rs`
- Native/compatible model adapter skeletons:
  - `crates/runtime/src/model_adapter.rs`
- Native context policy:
  - `crates/runtime/src/context_policy.rs`
  - DeepSeek uses the empirically safe 256K hard cap, a 192K live-compaction threshold, stable prompt prefix, and prefix-cache-aware ordering.
  - Qwen3.6-27B uses 262K native policy, compaction threshold, preserved thinking channel, and patch-sized edits.
- Context budget and scaffold policy:
  - `docs/agent_architecture_planning/36_context_budget_and_scaffold_policy.md`
  - `crates/runtime/src/context_budget.rs`
  - `cargo run -q -p researchcode-cli -- context-budget-smoke`
  - Encodes DeepSeek S3 Full Scaffold under a 256K effective cap, Qwen S1 Fast/S2 Guarded Scaffold, compatible S0 Minimal Scaffold, output reserve, emergency reserve, active-tool limits, and compaction guardrails.
  - Preserves ClaudeCode-like runtime scaffold while preventing Qwen from receiving a DeepSeek-sized prompt scaffold by default.
- Native provider endpoint gate:
  - `crates/runtime/src/native_provider.rs`
  - Validates DeepSeek V4 Flash over Anthropic-compatible transport and canonical Qwen3.6-27B endpoint metadata.
  - Keeps raw API keys out of config by requiring environment variable names.
  - Live calls are disabled by default and require explicit enablement plus network approval.
- Provider sidecar health gate:
  - `scripts/provider_http_sidecar.py`
  - `crates/runtime/src/sidecar_http_transport.rs`
  - `cargo run -q -p researchcode-cli -- provider-health-smoke`
  - Adds `mode=health_check` for prepared DeepSeek/Qwen requests.
  - Skips safely unless `RESEARCHCODE_ALLOW_NETWORK=1` is set and the configured API-key environment variable exists.
  - Never accepts raw key material as `authorization_env` and never writes provider response bodies during health checks.
- Secret scanner:
  - `crates/runtime/src/secret_scan.rs`
  - Detects common API keys, AWS keys, private-key headers, and `.env` path hints without echoing raw values.
  - Provides reusable redaction for command output, model transcript, and artifact boundaries.
  - Live model preflight blocks high-severity secret content before request building and records `model.call_blocked` with `gate=secret_detected`.
- Compatible provider config parity:
  - `crates/kernel/src/model.rs`
  - Compatible providers now track schema version, API-key environment variable name, model alias, capability hints, transforms, health check, and default-enabled status.
  - Compatible providers still cannot be marked native.
- Compatible provider request transform:
  - `crates/runtime/src/compatible_provider.rs`
  - `cargo run -q -p researchcode-cli -- compatible-provider-request-smoke`
  - Builds prepared HTTP request shapes for OpenAI-compatible, Anthropic-compatible, and explicit custom passthrough providers.
  - Normalizes generic compatible OpenAI/Anthropic response bodies into visible text and token counts using `compatible_generic_parser`.
  - Keeps compatible providers generic: no native DeepSeek/Qwen parser, scaffold, eval promotion, or model-specific compensation is applied.
- Command permission-gate skeleton:
  - `crates/runtime/src/command.rs`
- Command-output artifact capture:
  - `crates/runtime/src/command.rs`
  - Redacts secret-like stdout/stderr before command output is persisted as an artifact.
- Product Kernel ToolSpec registry:
  - `crates/kernel/src/tool.rs`
- Agent TUI toolbench:
  - `crates/cli/src/main.rs`
  - `cargo run -q -p researchcode-cli -- agent-tui`
  - `cargo run -q -p researchcode-cli -- agent-tui-script <script-file>`
  - `cargo run -q -p researchcode-cli -- agent-tui-smoke`
  - Provides a terminal-first local tool loop while GUI design is deferred.
  - Supports `/task`, `/status`, `/tools`, `/repo`, `/read`, `/search`, `/git`, `/context`, `/compact`, `/events`, `/ask-scripted`, `/ask-live-deepseek`, `/run`, `/replace`, `/csv`, and `/exit`.
  - `/run` and `/replace` use explicit approval prompts in interactive mode and auto-approval only in script/smoke mode.
  - Routes execution through the shared `ToolExecutionService v0`, so shell and patch paths use the same permission, stale-file, sensitive-path, and command-policy guards as the native agent loop.
  - Records TUI tool calls, permission decisions, tool result artifacts, and patch application into the Product Kernel `AgentSession` event log.
  - `/replace` now shows a minimal diff preview before file-write approval.
  - `/context` builds a model-family-aware ContextBundle from repo map, tracked file reads, tracked searches, and git status; `/compact` renders the structured compaction summary.
  - `/events <path>` exports the session JSONL event log for replay, local API ingestion, or GUI review.
  - `/ask-scripted [path]` runs the deterministic DeepSeek/Qwen native agent loop fixture from inside TUI and can export that full model/parser/tool/permission/review event log.
  - `/ask-live-deepseek [path]` runs the same DeepSeek native prompt/request/event boundary against the disabled-by-default provider sidecar. It blocks by default, exports `model.call_blocked`, and only becomes a live network call when `RESEARCHCODE_ENABLE_LIVE_PROVIDER=1`, `RESEARCHCODE_ALLOW_NETWORK=1`, and `DEEPSEEK_API_KEY` are present.
- Tool call request/completion event lifecycle:
  - `crates/runtime/src/session.rs`
  - `crates/runtime/src/payload.rs`
- Reviewer/failure-diagnosis lifecycle helpers:
  - `crates/runtime/src/session.rs`
- Safe file.read runtime tool:
  - `crates/runtime/src/file_tool.rs`
- Safe search runtime tool:
  - `crates/runtime/src/search_tool.rs`
- Patch proposal lifecycle events:
  - `crates/runtime/src/session.rs`
- ToolDispatcher scheduling policy:
  - `crates/runtime/src/tool_dispatcher.rs`
- Validated replace patch application:
  - `crates/runtime/src/patch.rs`
- Multi-file patch-set validator:
  - `crates/runtime/src/patch_set.rs`
  - `cargo run -q -p researchcode-cli -- patch-set-smoke`
  - Validates every file in a patch set before any write, rejects stale/protected/duplicate operations, and applies only after the whole set passes.
- Read-only git.status runtime tool:
  - `crates/runtime/src/git_tool.rs`
- ContextBundle primitives and builder:
  - `crates/kernel/src/context.rs`
  - `crates/runtime/src/context_builder.rs`
- Repo map context tool:
  - `crates/runtime/src/repo_map.rs`
  - `crates/kernel/src/tool.rs`
  - Adds read-only `repo.map` ToolSpec and `ContextItemKind::RepoMap`.
  - Scans project structure with file/depth limits, skips `.git`, `.env*`, `.ssh`, dependency/build/output noise, detects tech stack, and records important config/docs files.
  - `context-bundle-smoke` now includes repo map before targeted file/search/git context.
- SQLite persistence adapter v0:
  - `scripts/sqlite_store.py`
  - `scripts/test_sqlite_store.py`
- Executable DeepSeek/Qwen parser eval gate:
  - `scripts/run_parser_eval.py`
- Eval harness with SQLite result persistence:
  - `scripts/run_eval_harness.py`
- Native profile promotion gate:
  - `scripts/run_native_profile_promotion_gate.py`
  - Requires DeepSeek/Qwen parser eval, stream eval, and scaffold/context-budget eval to pass before native profile promotion.
  - Compatible providers are explicitly excluded from native promotion.
- Scaffold/context-budget eval:
  - `eval/fixtures/scaffold/scaffold_cases.json`
  - `eval/fixtures/scaffold/scaffold_comparison_cases.json`
  - `scripts/run_scaffold_eval.py`
  - `scripts/run_scaffold_comparison_eval.py`
  - `python3 scripts/run_eval_harness.py --suite scaffold`
  - `python3 scripts/run_eval_harness.py --suite scaffold_comparison`
  - Verifies DeepSeek planner uses `DeepSeekFull` under the safe 256K cap with a bounded reasoning replay budget, Qwen executor uses `QwenFast` under the 10% prompt guardrail with <=5 active tools, and Qwen planner uses `QwenGuarded` under the same prompt guardrail.
  - Adds deterministic comparison gates so Qwen lite/guarded modes cannot accidentally inherit DeepSeek full scaffold, while DeepSeek full mode must retain protected reserves, reasoning replay, and broad enough tool budget inside the 256K safety envelope.
- Local API server contract stub:
  - `scripts/local_api_server.py`
  - `scripts/test_local_api_server.py`
  - Exposes `/session-snapshot` so a client can reconstruct state, health, pending security approvals, pending plan approvals, event counts, and resume eligibility from any JSONL event log.
  - Exposes scaffold-aware model timeline metadata for native DeepSeek/Qwen calls, including scaffold level, prompt/tool hashes, estimated prompt tokens, context budget, dynamic context budget, protected reserve, and budget warning count.
  - Adds optional Bearer-token authentication via `RESEARCHCODE_LOCAL_API_TOKEN`.
  - Adds a standard-library per-client rate limiter for local development server hardening.
- Standard-library local agent runner:
  - `scripts/researchcode_agent.py`
  - `scripts/test_researchcode_agent.py`
  - Mock provider can run a full local loop and write `runs/.../events.jsonl` plus `summary.json`.
  - Emits Product Kernel-style `model.call_started` and `model.call_completed` events so local runs can be indexed by the same GUI/SQLite model-call path.
  - Stores sanitized model response artifacts under each run directory and keeps raw API keys out of CLI arguments.
  - Raw API-key CLI argument was removed; provider credentials are read only from environment variables.
  - DeepSeek local runner defaults to Anthropic-compatible `https://api.deepseek.com/anthropic/v1/messages` with model `deepseek-v4-flash`.
  - Sensitive path reads such as `.env` are denied by the tool runtime.
  - `runs/` is ignored by `.gitignore` because local agent runs generate artifacts there.
- ClaudeCode/OpenCode-inspired ToolSpec metadata:
  - `concurrency_safe`
  - `max_result_size_chars`
  - `result_policy`
- File-backed event store wrapper:
  - `crates/runtime/src/event_store.rs`
- Research Worker manifest validation tests:
  - `workers/research_worker/tests_manifest.py`
- Research Worker CSV profiling CLI:
  - `workers/research_worker/research_worker/__main__.py`
  - `workers/research_worker/research_worker/csv_profile.py`
  - `scripts/validate_research_worker_cli.py`
- Research Worker privacy report export:
  - `workers/research_worker/research_worker/csv_profile.py`
- Research Worker harness:
  - `crates/runtime/src/research_harness.rs`
  - `cargo run -q -p researchcode-cli -- research-harness-smoke`
  - Covers manifest lineage, artifact hashes, sensitive column/cloud approval detection, oversized input rejection, network/package-install sandbox limits, and package install classifier boundaries.
- Research Worker OS sandbox recording:
  - `workers/research_worker/research_worker/__main__.py`
  - Rust sidecar launcher passes timeout, input-byte, and memory-limit policy into the child process via environment variables.
  - Python child applies best-effort `resource.setrlimit` CPU/address-space limits where the platform allows it and records `os_sandbox` in the manifest.
- Static desktop command-center mock:
  - `apps/desktop/static_mock.html`
- Static desktop mock data:
  - `apps/desktop/mock_data/events.jsonl`
  - `apps/desktop/mock_data/artifact_manifest.json`
  - `apps/desktop/mock_data/privacy_report.json`
- No-model coding task fixture runner:
  - `cargo run -q -p researchcode-cli -- coding-fixture-smoke`
  - Covers read/search/patch validation/file-write approval/command approval/command output artifact/review/completion.
- Failure repair fixture runner:
  - `cargo run -q -p researchcode-cli -- failure-repair-fixture-smoke`
  - Covers failed command, diagnosis, repair patch, rerun, review, and completion.
- Runtime AgentExecutor skeleton:
  - `crates/runtime/src/executor.rs`
  - CLI fixture commands now call runtime orchestration instead of embedding session flow in CLI.
- Multi-agent orchestration policy gate:
  - `crates/runtime/src/multi_agent_policy.rs`
  - Default is forced single-agent.
  - Research swarm/adversarial review are read/report-only.
  - Spike parallel and implementation shards require isolation rules.
  - Kernel/schema/permission/patch/model-router/native adapter/security paths are blocked from parallel modification.
- Worktree isolation planner dry-run:
  - `crates/runtime/src/worktree.rs`
  - Builds auditable `git worktree add` argument plans without invoking git.
  - Rejects non-git project roots, unsafe agent ids/branch names, escaping paths, and duplicate worktree paths.
- Typed tool result artifacts:
  - `crates/runtime/src/tool_result.rs`
  - `tool.result_recorded` events link tool calls to artifact ids, content hashes, and GUI previews.
- Runtime event contract validator:
  - `scripts/validate_runtime_event_contract.py`
  - Verifies GUI/API-required event types and tool result artifact links.
- Event replay/session snapshot:
  - `crates/runtime/src/replay.rs`
  - `cargo run -q -p researchcode-cli -- event-replay-smoke`
  - `cargo run -q -p researchcode-cli -- event-replay-summary <events.jsonl>`
  - Reconstructs session state, health, pending permissions, pending plan approvals, tool/model counts, patch count, and resume eligibility from JSONL without mutating the log.
- Event invariant validator:
  - `crates/runtime/src/event_invariants.rs`
  - `cargo run -q -p researchcode-cli -- event-invariant-smoke`
  - `cargo run -q -p researchcode-cli -- validate-event-invariants <events.jsonl>`
  - Validates semantic event ordering beyond JSONL parsing: session creation first, model call start/completion pairing, stream completion pairing, tool request/completion/result ordering, pending permission decisions, PlanApproval/Permission separation, and patch apply after pass validation plus file-write approval.
- Approval queue extractor:
  - `crates/runtime/src/approval_queue.rs`
  - `cargo run -q -p researchcode-cli -- approval-queue-smoke`
  - `cargo run -q -p researchcode-cli -- approval-queue-summary <events.jsonl>`
  - Extracts pending PlanApproval and PermissionRequest items from EventLog while keeping task governance approval separate from safety approval.
- Runtime harness suite:
  - `crates/runtime/src/harness.rs`
  - `cargo run -q -p researchcode-cli -- runtime-harness-smoke`
  - Aggregates deterministic bottom-layer cases for no-model coding, failure repair, recorded model planning, patch validation, recorded agent loop, scripted native loop, blocked permission boundary, provided-permission resume, and recorded research loop.
- Core tool harness suite:
  - `crates/runtime/src/tool_harness.rs`
  - `cargo run -q -p researchcode-cli -- tool-harness-smoke`
  - Verifies every Product Kernel ToolSpec has concrete positive/boundary coverage for read/search/repo/git/shell/patch/research/artifact behavior.
- Foundation harness:
  - `cargo run -q -p researchcode-cli -- foundation-harness-smoke`
  - Aggregates runtime, tool, research, event-invariant, and patch-set smoke checks as a single lower-layer health gate.
- Recorded model-planned fixture:
  - `cargo run -q -p researchcode-cli -- recorded-model-fixture-smoke`
  - Uses recorded DeepSeek/Qwen parser outputs to drive safe tool execution without network calls.
  - Verifies Qwen2/Qwen2-7B cannot enter Qwen native session.
- Recorded patch fixture:
  - `cargo run -q -p researchcode-cli -- recorded-patch-fixture-smoke`
  - Verifies Qwen stale-file patch is rejected before apply.
  - Verifies Qwen ambiguous patch is rejected before apply.
  - Verifies a valid DeepSeek recorded patch still requires file-write permission before apply.
- DeepSeek reasoning replay policy:
  - `crates/runtime/src/deepseek_reasoning.rs`
  - Blocks Claude-style replay of `reasoning_content` as generic chat/tool-result messages.
  - Allows replay only through native DeepSeek request fields or sanitized summaries.
  - Redacts secret-like content before persistence.
- DeepSeek stream receiver contract:
  - `crates/runtime/src/deepseek_stream.rs`
  - Parses recorded SSE-style DeepSeek deltas without network calls.
  - Keeps `reasoning_content`, visible `content`, tool-call fragments, usage tokens, and prefix-cache telemetry as separate channels.
  - Sanitizes reasoning deltas before event/artifact persistence and never replays them as ordinary chat/tool-result messages.
- Qwen stream receiver contract:
  - `crates/runtime/src/qwen_stream.rs`
  - Parses recorded Qwen/OpenAI-compatible SSE-style deltas without network calls.
  - Requires canonical `Qwen3.6-27B` deployment metadata for native mode.
  - Keeps thinking, visible content, tool-call fragments, and token telemetry as separate channels.
  - Handles chunks that contain both deployment metadata and thinking content without dropping either channel.
- Stream parser eval gate:
  - `eval/fixtures/deepseek/stream_golden.json`
  - `eval/fixtures/qwen/stream_golden.json`
  - `scripts/run_stream_eval.py`
  - Promotes DeepSeek/Qwen stream parsing behavior from unit guards into executable eval fixtures.
- Model stream event lifecycle:
  - `crates/runtime/src/session.rs`
  - `crates/runtime/src/payload.rs`
  - Adds `model.stream_delta` and `model.stream_completed` events for GUI/API observability.
  - Records sanitized reasoning previews and prefix-cache telemetry without raw reasoning leakage.
- Model call boundary events:
  - `crates/runtime/src/session.rs`
  - Adds `model.call_started` and `model.call_completed` events before any future live provider execution.
  - Events include provider/model/adapter metadata and transcript artifact references, but never API-key material.
- Runtime event import into SQLite:
  - `scripts/persist_runtime_fixture_sqlite.py`
  - `scripts/sqlite_store.py`
  - Imports generated runtime JSONL and indexes `tool_calls`, `tool_result` artifacts, and `model_transcript` stream artifacts.
- SQLite GUI/API summary helpers:
  - `scripts/sqlite_store.py`
  - `scripts/local_api_server.py`
  - Exports event counts, latest state, tool counts, artifact counts, recent events, and local API `/summary` output.
- Coding fixture event-log export:
  - `cargo run -q -p researchcode-cli -- coding-fixture-eventlog /private/tmp/researchcode-coding-fixture-events.jsonl`
  - Exported JSONL validates through the same event-log replay path.
- Desktop local API client contract:
  - `apps/desktop/local_api_client.mjs`
  - `apps/desktop/test_local_api_client.mjs`
  - `apps/desktop/test_static_mock_contract.mjs`
  - `apps/desktop/static_mock.html` can read either local mock JSONL or local API events.
  - Client now reads `/summary` and builds a command-center view model with event counts, latest event, tool counts, artifact counts, and recent events.
  - Client now derives a model stream timeline from `model.call_*` and `model.stream_*` events, including sanitized reasoning/thinking previews, transcript artifact ids, reasoning token counts, and DeepSeek prefix-cache telemetry.
  - Static mock surfaces model-call counts in the summary metrics and renders model stream details from `apps/desktop/mock_data/model_events.jsonl`.
- Model transcript artifact format:
  - `crates/runtime/src/model_transcript.rs`
  - Sanitizes secret-like text and does not persist raw reasoning by default.
  - Can build a transcript from a DeepSeek stream assembly while using only visible content as the response preview.
  - Can build a transcript from a Qwen stream assembly while using only visible content as the response preview.
- Controlled command execution:
  - `crates/runtime/src/command.rs`
  - Executes tokenized commands directly without a shell.
  - Dangerous commands remain blocked even with user permission.
  - Command classifier and executor now share a safe tokenizer from `crates/runtime/src/permission.rs`, preserving quoted arguments for common commands such as `rg "hello world" docs` while rejecting pipes, redirection, command substitution, injected separators, unclosed quotes, destructive commands, and sensitive paths before execution.
- Research Worker sidecar launcher:
  - `crates/runtime/src/research_worker.rs`
  - Rust runtime can invoke the local Python CSV profiler and verify manifest outputs.
  - Enforces no-network/no-package-install policy and input-size limits before launching the sidecar.
  - Runtime result reports manifest content hash and artifact count for audit/indexing.
- Research Worker resource-limit manifest:
  - `workers/research_worker/research_worker/manifest.py`
  - `workers/research_worker/research_worker/__main__.py`
  - Manifest records max input/output bytes, max rows, timeout seconds, and package-install policy.
  - Manifest records data lineage edges from the input content hash to each derived artifact hash.
- Research Worker artifact lifecycle v0:
  - `workers/research_worker/research_worker/artifacts.py`
  - CSV profiling now emits a reproducible analysis script, Markdown report, and notebook skeleton in addition to data profile and privacy report.
  - Manifest records content hashes for every generated research artifact.
  - Manifest records each artifact's `source_input_hash`.
  - Rust sidecar result includes paths for profile, privacy report, analysis script, report, notebook, and manifest.
- Research Worker package-install policy classifier:
  - `crates/runtime/src/research_worker.rs`
  - Valid package specs are classified as `PermissionRequired`; package injection strings are rejected before any permission prompt.
  - Valid package requests can be converted into `permission.requested` events with `request_type=package_install`.
  - Denied package-install decisions return the AgentSession to `Executing`; no package installation is performed by the worker.
  - CLI smoke: `cargo run -q -p researchcode-cli -- research-package-install-policy-smoke`.
- Disabled live request builder:
  - `crates/runtime/src/live_model_request.rs`
  - Builds auditable DeepSeek V4 Flash Anthropic-compatible request shapes without network I/O.
  - Builds auditable Qwen3.6-27B OpenAI-compatible/custom request shapes after endpoint base URL has been resolved.
  - Uses `DEEPSEEK_API_KEY` only as an environment-variable name and never stores key material.
  - Uses `QWEN_API_KEY` only as an environment-variable name and never stores key material.
- Live model executor preflight:
  - `crates/runtime/src/live_model_executor.rs`
  - Records `model.call_started` and `model.call_blocked` events.
  - Blocks live calls unless native provider gate, explicit live enablement, API-key env value, and network approval all pass.
  - Still performs no network I/O.
- Optional live DeepSeek smoke script:
  - `scripts/live_deepseek_smoke.py`
  - Skips by default unless `RESEARCHCODE_ALLOW_NETWORK=1` and `DEEPSEEK_API_KEY` are present.
  - Targets the DeepSeek Anthropic-compatible endpoint shape without persisting key material.
- Live native eval runner:
  - `scripts/run_live_native_eval.py`
  - Exercises provider health checks, DeepSeek/Qwen native sidecar-live loop event-log export, event-log validation, and secret-leak guards.
  - Acts as a no-network regression gate by default and as a live promotion smoke when provider/network env gates are explicitly enabled.
- Disabled provider HTTP sidecar:
  - `scripts/provider_http_sidecar.py`
  - `crates/runtime/src/sidecar_http_transport.rs`
  - `cargo run -q -p researchcode-cli -- provider-sidecar-smoke`
  - `cargo run -q -p researchcode-cli -- deepseek-sidecar-live-smoke`
  - `cargo run -q -p researchcode-cli -- deepseek-sidecar-live-eventlog /private/tmp/researchcode-deepseek-sidecar-live-events.jsonl`
  - `cargo run -q -p researchcode-cli -- qwen-sidecar-live-smoke`
  - `cargo run -q -p researchcode-cli -- qwen-sidecar-live-eventlog /private/tmp/researchcode-qwen-sidecar-live-events.jsonl`
  - Sends only already-prepared native provider requests and only when `RESEARCHCODE_ALLOW_NETWORK=1` is present.
  - Reads API key values only through the prepared request's `authorization_env` name and rejects secret-like `authorization_env` values before network I/O.
  - Writes raw provider response bytes to a temporary body file for Rust to pass into the existing DeepSeek/Qwen normalizer or SSE assembler; stdout contains only status/skip metadata.
  - Normalizes the user-facing DeepSeek Anthropic base URL `https://api.deepseek.com/anthropic` into `/v1/messages` inside the sidecar without changing Product Kernel request metadata.
  - DeepSeek live smoke is also blocked unless `RESEARCHCODE_ENABLE_LIVE_PROVIDER=1`; default runs emit a valid `model.call_blocked` event log instead of touching the network.
  - Qwen live smoke uses the canonical `Qwen/Qwen3.6-27B` native adapter and requires a resolved `QWEN_BASE_URL` before live use; default runs stay blocked before network access.
  - Optional live smoke requests now use the native prompt assembler with task context, repo map, and stable tool catalog instead of a bare one-line test prompt.
- Recorded live-response fixture:
  - `crates/runtime/src/executor.rs`
  - `cargo run -q -p researchcode-cli -- recorded-live-response-fixture-smoke`
  - `cargo run -q -p researchcode-cli -- recorded-live-response-fixture-eventlog /private/tmp/researchcode-recorded-live-response-events.jsonl`
  - Consumes recorded DeepSeek and Qwen stream fragments through the native stream assemblers.
  - Emits `model.call_started`, sanitized `model.stream_delta`, `model.stream_completed`, `model.call_completed`, and transcript artifacts.
  - Verifies raw reasoning, `.env` path references, and secret-like tokens are not exported in event JSONL.
- Model event contract validator:
  - `scripts/validate_model_event_contract.py`
  - Requires stream and call lifecycle events for DeepSeek/Qwen recorded live-response event logs.
  - Blocks secret/API-key/path leakage in model event payloads.
- Native provider-response adapter boundary:
  - `crates/runtime/src/provider_response_adapter.rs`
  - Converts DeepSeek/Qwen provider stream fragments into `model.*` Product Kernel events and transcript artifacts.
  - Converts normalized non-stream DeepSeek/Qwen responses into the same `model.call_started`, sanitized `model.stream_delta`, `model.stream_completed`, and `model.call_completed` event contract.
  - Reuses DeepSeek reasoning sanitizer, prefix-cache telemetry capture, Qwen3.6-27B deployment validation, and Qwen thinking/content separation.
  - Keeps future live provider responses on the same path as recorded fixtures instead of replaying hidden reasoning as generic chat content.
- Native prompt assembler:
  - `crates/runtime/src/prompt_assembler.rs`
  - `cargo run -q -p researchcode-cli -- native-prompt-smoke deepseek`
  - `cargo run -q -p researchcode-cli -- native-prompt-smoke qwen`
  - Builds model-facing system/user messages from `PlannedModelCall`, `ContextBundle`, and the Product Kernel `ToolSpec` registry.
  - DeepSeek system prompt preserves stable prefix, prefix-cache ordering, native tool-call priority, DSML/XML fallback, and the rule that `reasoning_content` must never be replayed as ordinary chat/tool-result content.
  - Qwen system prompt preserves the canonical Qwen3.6-27B native contract, thinking/preserve-thinking usage, parser/template dependency, patch-sized edits, stale-file protection, and 262K native context limit.
  - Tool catalog output is stable and sorted so prompt-cache/eval comparisons do not drift with registry order.
  - Prompt assembly now consumes `ContextBudget`: DeepSeek records `DeepSeekFull`, Qwen executor records `QwenFast`, active tools are capped by scaffold level, oversized context is omitted with a budget warning, and estimated input tokens are exposed for future eval telemetry.
- Model-call scaffold telemetry:
  - `crates/runtime/src/session.rs`
  - `crates/runtime/src/live_model_executor.rs`
  - `scripts/validate_model_event_contract.py`
  - `model.call_started` events now include `scaffold_level`, `prompt_tokens_estimate`, `prompt_hash`, `tool_catalog_hash`, `max_context_tokens`, `prompt_scaffold_budget`, `dynamic_context_budget`, `protected_reserve_tokens`, and `budget_warning_count`.
  - The model event contract validator now blocks native DeepSeek/Qwen event logs that omit scaffold telemetry or protected reserve data.
- Native non-stream response normalizer:
  - `crates/runtime/src/native_response_normalizer.rs`
  - `crates/runtime/src/executor.rs`
  - Normalizes DeepSeek Anthropic-style message JSON and Qwen/OpenAI-style message JSON into the adapter input shape.
  - Extracts visible content, sanitized hidden reasoning/thinking, token counts, and DeepSeek prefix-cache token telemetry.
  - Rejects non-canonical Qwen deployments before native-mode persistence.
  - Runtime fixture now follows `provider JSON -> normalizer -> provider_response_adapter -> event log/SQLite/GUI contract`; CLI only delegates to runtime.
- Prepared live-response recorder:
  - `crates/runtime/src/live_model_executor.rs`
  - `crates/cli/src/main.rs`
  - Adds `record_live_model_response` for response bodies received after `prepare_live_model_execution` has already emitted `model.call_started`.
  - Uses the native normalizer plus `record_native_provider_response_after_started` so each live call has exactly one call-start boundary.
  - CLI command `live-model-response-record-eventlog` emits both DeepSeek and Qwen model streams, passes the model-event contract validator, and persists through SQLite.
- Injectable live HTTP transport boundary:
  - `crates/runtime/src/live_http_transport.rs`
  - Adds a `LiveHttpTransport` trait and `run_live_model_http_once` orchestration.
  - Adds `ScriptedLiveHttpTransport` for deterministic fixture-driven native loops that still use the same live transport trait boundary.
  - Keeps socket implementation replaceable while preserving native preflight, response normalization, event logging, and transcript artifact persistence.
  - Recorded transport tests cover DeepSeek, Qwen, HTTP failure events, and streaming SSE response routing without network access or raw key material.
  - Streaming responses use `record_live_model_stream_response` and `record_native_provider_stream_after_started`, so `model.call_started` remains single-emission even when the provider response arrives as DeepSeek/Qwen stream fragments.
- Reusable native agent loop:
  - `crates/runtime/src/native_agent_loop.rs`
  - `crates/runtime/src/native_turn_controller.rs`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-eventlog /private/tmp/researchcode-native-agent-loop-events.jsonl`
  - Extracts the native executor path into a transport-generic orchestrator: AgentSession state transitions, DeepSeek/Qwen live model call recording, visible-output tool parsing, parser policy checks, permission-gated tool execution, tool-result artifacts, and review completion.
  - Adds a native turn controller that emits `agent.turn.started`, `agent.turn.ledger_updated`, `agent.tool.pending`, `agent.tool.completed`, `agent.recovery.started`, `model.context_budget`, and `context.compaction.*` events.
  - DeepSeek prepared requests are guarded before provider I/O: requests target <240K, trigger compaction telemetry after 192K, and block before exceeding the 256K hard cap.
  - DeepSeek OpenAI-compatible tool-result continuation now always replays `reasoning_content` or a bounded placeholder so thinking-mode tool replay remains well-formed after tool calls.
  - Streaming native tool calls now assemble `tool_use` / `tool_calls` argument deltas while the provider stream is still open; safe read/search/repo/git tools and FastAuto file-write tools execute as soon as their JSON input is complete, then continue through the normal tool-result continuation path.
  - Mixed or permission/control tool batches still fall back to the full-response parser so `plan.enter`, `ask_user`, shell permission, and unsupported-tool recovery preserve the existing blocking semantics.
  - Legacy visible-output tool execution now also routes through Tool Contract Mediation, so old loop, full-response parser, and streaming tool calls share alias resolution, malformed JSON recovery, issue-path repair, model-readable errors, and wrong-tool intent recovery.
  - External-decision resume records `agent.recovery.started` and skips re-execution when replay shows the pending tool already has a recorded result.
  - Uses `ToolExecutionService v0` for file reads, search, repo maps, patch application, and shell commands instead of duplicating tool execution inside the loop.
  - Supports fixture auto-approval for deterministic evals, external-decision blocking for GUI/CLI approval wiring, and injected permission decisions for resumed execution tests.
  - Passes event-log validation, runtime event contract, model event contract, and SQLite persistence with 40 events, 3 model calls, and 3 tool calls.
- Native sidecar-live loop entrypoint:
  - `cargo run -q -p researchcode-cli -- native-agent-loop-sidecar-live-eventlog deepseek <events.jsonl>`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-sidecar-live-eventlog qwen <events.jsonl>`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-sidecar-live-pending-package deepseek <package-dir>`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-sidecar-live-pending-package qwen <package-dir>`
  - Produces valid blocked event logs by default.
  - Only attempts live provider I/O when `RESEARCHCODE_ENABLE_LIVE_PROVIDER=1`, `RESEARCHCODE_ALLOW_NETWORK=1`, and the endpoint API-key env var is present.
- Blocked permission loop contract:
  - `scripts/validate_blocked_permission_event_contract.py`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-blocked-eventlog /private/tmp/researchcode-native-agent-loop-blocked-events.jsonl`
  - Exports a valid event log that intentionally stops at `WaitingForToolApproval` after `permission.requested` without recording `permission.decided`, `patch.applied`, `tool.call_completed`, or `tool.result_recorded`.
  - This gives the GUI a concrete approval-boundary fixture instead of treating a waiting session as a runtime failure.
- Provided-permission resume loop contract:
  - `cargo run -q -p researchcode-cli -- native-agent-loop-resume-smoke`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-resume-eventlog /private/tmp/researchcode-native-agent-loop-resume-events.jsonl`
  - Verifies a supplied external permission decision can continue the native loop through patch apply, tool-result recording, review, and completion.
  - The exported event log replays to `Completed` with one requested permission, one decided permission, and one applied patch.
- EventLog-backed external-decision resume contract:
  - `crates/runtime/src/session.rs`
  - `crates/runtime/src/native_agent_loop.rs`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-external-resume-smoke`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-external-resume-eventlog /private/tmp/researchcode-native-agent-loop-external-resume-events.jsonl`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-export-pending-package /private/tmp/researchcode-native-pending-package`
  - `cargo run -q -p researchcode-cli -- native-agent-loop-resume-pending-package /private/tmp/researchcode-native-pending-package allow_once`
  - `AgentSession::resume_from_event_log` reconstructs project/session/task ids, last state, and pending approval boundaries from JSONL without mutating the prior log.
  - `NativeAgentLoopResult` now returns a typed `PendingNativeToolExecution` when external file-write or command approval is required, so GUI/TUI approval code no longer needs to infer tool arguments from raw model text.
  - `resume_native_agent_loop_after_external_decision` imports the blocked EventLog, records the external permission decision, executes exactly the pending tool through `ToolExecutionService v0`, records patch/tool-result events, and continues remaining native steps.
  - The package export path writes `events.jsonl`, `pending_tool.json`, `resume_package.json`, a persistent workspace, and an artifact directory, which gives TUI/local API approval surfaces a concrete handoff format.
  - The deterministic fixture proves approval resume applies the pending Qwen patch and completes review with zero additional model calls when there are no remaining steps.
- Recorded native agent loop:
  - `crates/runtime/src/recorded_agent_loop.rs`
  - `cargo run -q -p researchcode-cli -- recorded-agent-loop-eventlog /private/tmp/researchcode-recorded-agent-loop-events.jsonl`
  - Records DeepSeek/Qwen model responses, parses visible model output, dispatches file read, validates/applies a patch with file-write approval, runs a command with command approval, writes tool-result artifacts, and completes review.
  - Executes tool arguments through `tool_call_parser` rather than reading file paths, patch strings, or commands directly from the raw model text.
  - Passes runtime event contract, model event contract, and SQLite persistence.
- Live-transport native agent loop:
  - `crates/runtime/src/recorded_agent_loop.rs`
  - `cargo run -q -p researchcode-cli -- live-transport-agent-loop-eventlog /private/tmp/researchcode-live-transport-agent-loop-events.jsonl`
  - Uses the same live HTTP preflight and stream-response recorder as future real provider calls, but injects recorded DeepSeek/Qwen SSE bodies through `RecordedLiveHttpTransport`.
  - Proves the end-to-end runtime path `prepared native request -> streaming HTTP response -> DeepSeek/Qwen SSE assembler -> sanitized model events/transcripts -> streamed tool_call reconstruction -> ToolCall parser -> file/patch/command tools -> approvals -> review`.
  - Passes event-log validation, runtime event contract, model event contract, and SQLite persistence.
- Recorded Research Coworker loop:
  - `crates/runtime/src/recorded_research_loop.rs`
  - `cargo run -q -p researchcode-cli -- recorded-research-loop-eventlog /private/tmp/researchcode-recorded-research-loop-events.jsonl`
  - Records a Qwen researcher response, parses `research.csv_profile`, runs the Python Research Worker sidecar with network/package install disabled, records manifest hash and artifact count, and completes review.
  - Passes the Research Coworker event contract and SQLite persistence.
- SQLite model-call indexing:
  - `scripts/sqlite_store.py`
  - Imports `model.call_started`, `model.stream_completed`, and `model.call_completed` into `model_calls`.
  - Exposes `model_counts` in GUI summaries while still indexing transcript artifacts.
- Local API model timeline:
  - `scripts/local_api_server.py`
  - Adds `/model-timeline` for GUI consumers.
  - Adds `/latest-run` and default event-path resolution from `runs/latest_dev_fixture_bundle.json` when present.
- Local API approval queue:
  - `scripts/local_api_server.py`
  - Adds `/approval-queue`, defaulting to `blocked_permission_events` from the latest dev fixture bundle.
  - Returns pending permission requests plus related patch/tool context, so GUI approval drawers can render a real safety boundary without parsing raw events themselves.
- Local API native pending-package endpoints:
  - `scripts/local_api_server.py`
  - `POST /native-loop/pending-package` creates a local pending-decision package through the Rust CLI and returns the pending approval queue plus typed pending tool JSON.
  - `POST /native-loop/live-pending-package` creates the same package from a real live provider-produced blocked tool call, failing explicitly if provider/network/key gates are not ready.
  - `POST /native-loop/pending-package-from-session` packages an existing blocked session event log plus a typed pending tool into the same `resume_package.json` contract, after verifying the session is actually blocked for permission and the pending tool permission id matches the pending permission queue.
  - `POST /native-loop/resume-pending-package` resumes the package with `allow_once` or `deny`, exports `resumed_events.jsonl`, and returns the resumed approval state.
  - Package paths are constrained through the local API safe path boundary; scripted and real-live blocked package creation now share the same `resume_package.json` contract.
- Local API repo map:
  - `scripts/local_api_server.py`
  - Adds `/repo-map` for GUI project summaries.
  - Uses the same safe workspace path boundary as event/artifact APIs, skips `.env*`, `.git`, `.ssh`, dependency/build/output noise, and returns tech stack, important files, and a compact tree.
- Desktop workflow panels:
  - `apps/desktop/local_api_client.mjs`
  - `apps/desktop/static_mock.html`
  - Builds GUI-facing summaries for tools, permissions, patches, commands, and repo map from the same local API boundary used by the model timeline.
  - Static mock now auto-discovers the latest dev fixture bundle via local API and shows model timeline, workflow panels, and project map from `live_transport_agent_loop_events.jsonl`.
  - Static mock also reads the `/approval-queue` endpoint backed by `blocked_permission_events` and populates the Permission Request drawer from a real pending approval event log.
- Dev fixture bundle:
  - `scripts/run_dev_fixture_bundle.py`
  - `scripts/test_dev_fixture_bundle.py`
  - Runs deterministic coding, native DeepSeek/Qwen model fixtures, the recorded native agent loop, the live-transport native agent loop, the reusable native agent loop fixture, the blocked-permission approval fixture, and the recorded Research Coworker loop.
  - Validates event logs, writes a manifest under `runs/`, and records the latest bundle path for static GUI/local API inspection.
  - The static GUI URL points at `live_transport_agent_loop_events.jsonl` so the page shows model, tool, patch, permission, command, and review events together from the closest no-network shape to a live provider call.
  - The manifest also exposes `blocked_permission_events` and `blocked_permission_summary` so GUI approval panels can be tested against a session that intentionally waits at `permission.requested`.
  - `scripts/test_local_api_http.py` verifies `/events/stream` and `/model-timeline` over a real local HTTP server when the environment allows socket binding; default sandbox runs skip this test safely.

## Validation Commands Passing

```bash
python3 scripts/validate_kernel_schemas.py
python3 scripts/validate_event_sequence.py docs/prototypes/event_log_replay/coding_task_sequence.jsonl
python3 scripts/prototype_patch_validator.py eval/fixtures/patch
python3 scripts/prototype_command_classifier.py eval/fixtures/shell/permission_cases.json
python3 -m json.tool eval/fixtures/deepseek/parser_golden.json >/dev/null
python3 -m json.tool eval/fixtures/qwen/parser_golden.json >/dev/null
python3 scripts/validate_parser_fixtures.py
python3 scripts/run_parser_eval.py
python3 scripts/run_stream_eval.py
python3 scripts/validate_csv_profile_fixture.py
python3 -m unittest scripts/test_sqlite_store.py
python3 scripts/sqlite_store.py
python3 scripts/run_eval_harness.py --suite parser
python3 -m unittest scripts/test_local_api_server.py
python3 -m unittest scripts/test_local_api_http.py
python3 scripts/live_deepseek_smoke.py
node apps/desktop/test_local_api_client.mjs
node apps/desktop/test_static_mock_contract.mjs
python3 scripts/validate_research_worker_cli.py
python3 scripts/validate_scaffold.py
python3 -m unittest workers/research_worker/tests_manifest.py
python3 -m unittest scripts/test_dev_fixture_bundle.py
sqlite3 :memory: < docs/storage/sqlite_schema_v0.sql
cargo test --workspace
cargo run -q -p researchcode-cli -- agent-tui-smoke
cargo run -q -p researchcode-cli -- native-agent-loop-smoke
cargo run -q -p researchcode-cli -- native-agent-loop-eventlog /private/tmp/researchcode-native-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-native-agent-loop-events.jsonl
python3 scripts/validate_runtime_event_contract.py /private/tmp/researchcode-native-agent-loop-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-native-agent-loop-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-native-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- native-agent-loop-blocked-smoke
cargo run -q -p researchcode-cli -- native-agent-loop-blocked-eventlog /private/tmp/researchcode-native-agent-loop-blocked-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-native-agent-loop-blocked-events.jsonl
python3 scripts/validate_blocked_permission_event_contract.py /private/tmp/researchcode-native-agent-loop-blocked-events.jsonl
cargo run -q -p researchcode-cli -- native-agent-loop-external-resume-smoke
cargo run -q -p researchcode-cli -- native-agent-loop-external-resume-eventlog /private/tmp/researchcode-native-agent-loop-external-resume-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-native-agent-loop-external-resume-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-invariants /private/tmp/researchcode-native-agent-loop-external-resume-events.jsonl
cargo run -q -p researchcode-cli -- event-replay-summary /private/tmp/researchcode-native-agent-loop-external-resume-events.jsonl
cargo run -q -p researchcode-cli -- native-agent-loop-export-pending-package /private/tmp/researchcode-native-pending-package
cargo run -q -p researchcode-cli -- native-agent-loop-resume-pending-package /private/tmp/researchcode-native-pending-package allow_once
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-native-pending-package/resumed_events.jsonl
cargo run -q -p researchcode-cli -- validate-event-invariants /private/tmp/researchcode-native-pending-package/resumed_events.jsonl
cargo run -q -p researchcode-cli -- classify-command 'npm test -- parser'
cargo run -q -p researchcode-cli -- prepare-command 'npm install lodash'
cargo run -q -p researchcode-cli -- validate-event-log docs/prototypes/event_log_replay/coding_task_sequence.jsonl
cargo run -q -p researchcode-cli -- artifact-store-smoke
cargo run -q -p researchcode-cli -- model-adapter-smoke
cargo run -q -p researchcode-cli -- list-tools
cargo run -q -p researchcode-cli -- search-text crates ToolSpec
cargo run -q -p researchcode-cli -- repo-map-smoke .
cargo run -q -p researchcode-cli -- coding-fixture-smoke
cargo run -q -p researchcode-cli -- failure-repair-fixture-smoke
cargo run -q -p researchcode-cli -- recorded-model-fixture-smoke
cargo run -q -p researchcode-cli -- recorded-patch-fixture-smoke
cargo run -q -p researchcode-cli -- deepseek-reasoning-policy-smoke
cargo run -q -p researchcode-cli -- deepseek-stream-smoke
cargo run -q -p researchcode-cli -- deepseek-stream-eventlog-smoke
cargo run -q -p researchcode-cli -- qwen-stream-smoke
cargo run -q -p researchcode-cli -- qwen-stream-eventlog-smoke
cargo run -q -p researchcode-cli -- native-provider-gate-smoke
cargo run -q -p researchcode-cli -- model-call-boundary-smoke
cargo run -q -p researchcode-cli -- deepseek-request-builder-smoke
cargo run -q -p researchcode-cli -- qwen-request-builder-smoke
cargo run -q -p researchcode-cli -- live-model-preflight-smoke
cargo run -q -p researchcode-cli -- coding-fixture-eventlog /private/tmp/researchcode-coding-fixture-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-coding-fixture-events.jsonl
python3 scripts/validate_runtime_event_contract.py /private/tmp/researchcode-coding-fixture-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-coding-fixture-events.jsonl
cargo run -q -p researchcode-cli -- recorded-live-response-fixture-smoke
cargo run -q -p researchcode-cli -- recorded-live-response-fixture-eventlog /private/tmp/researchcode-recorded-live-response-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-recorded-live-response-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-recorded-live-response-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-recorded-live-response-events.jsonl
cargo run -q -p researchcode-cli -- recorded-agent-loop-smoke
cargo run -q -p researchcode-cli -- tool-call-parser-smoke
cargo run -q -p researchcode-cli -- recorded-agent-loop-eventlog /private/tmp/researchcode-recorded-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-recorded-agent-loop-events.jsonl
python3 scripts/validate_runtime_event_contract.py /private/tmp/researchcode-recorded-agent-loop-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-recorded-agent-loop-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-recorded-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- live-transport-agent-loop-smoke
cargo run -q -p researchcode-cli -- live-transport-agent-loop-eventlog /private/tmp/researchcode-live-transport-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-live-transport-agent-loop-events.jsonl
python3 scripts/validate_runtime_event_contract.py /private/tmp/researchcode-live-transport-agent-loop-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-live-transport-agent-loop-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-live-transport-agent-loop-events.jsonl
cargo run -q -p researchcode-cli -- recorded-research-loop-smoke
cargo run -q -p researchcode-cli -- recorded-research-loop-eventlog /private/tmp/researchcode-recorded-research-loop-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-recorded-research-loop-events.jsonl
python3 scripts/validate_research_event_contract.py /private/tmp/researchcode-recorded-research-loop-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-recorded-research-loop-events.jsonl
cargo run -q -p researchcode-cli -- native-response-adapter-smoke
cargo run -q -p researchcode-cli -- native-response-normalizer-smoke
cargo run -q -p researchcode-cli -- native-response-adapter-eventlog /private/tmp/researchcode-native-response-adapter-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-native-response-adapter-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-native-response-adapter-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-native-response-adapter-events.jsonl
cargo run -q -p researchcode-cli -- live-model-response-record-smoke
cargo run -q -p researchcode-cli -- live-model-response-record-eventlog /private/tmp/researchcode-live-response-record-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-live-response-record-events.jsonl
python3 scripts/validate_model_event_contract.py /private/tmp/researchcode-live-response-record-events.jsonl
python3 scripts/persist_runtime_fixture_sqlite.py /private/tmp/researchcode-live-response-record-events.jsonl
cargo run -q -p researchcode-cli -- live-http-transport-smoke
cargo run -q -p researchcode-cli -- provider-sidecar-smoke
cargo run -q -p researchcode-cli -- deepseek-sidecar-live-smoke
cargo run -q -p researchcode-cli -- deepseek-sidecar-live-eventlog /private/tmp/researchcode-deepseek-sidecar-live-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-deepseek-sidecar-live-events.jsonl
cargo run -q -p researchcode-cli -- qwen-sidecar-live-smoke
cargo run -q -p researchcode-cli -- qwen-sidecar-live-eventlog /private/tmp/researchcode-qwen-sidecar-live-events.jsonl
cargo run -q -p researchcode-cli -- validate-event-log /private/tmp/researchcode-qwen-sidecar-live-events.jsonl
cargo run -q -p researchcode-cli -- run-safe-command-smoke
cargo run -q -p researchcode-cli -- model-transcript-artifact-smoke
cargo run -q -p researchcode-cli -- native-prompt-smoke deepseek
cargo run -q -p researchcode-cli -- native-prompt-smoke qwen
cargo run -q -p researchcode-cli -- research-worker-sidecar-smoke
```

## Current Test Result

- Rust runtime tests: 167 unit tests passed.
- Rust workspace tests: passed.
- Python/schema/prototype/Research Worker checks: passed.
- Desktop local API client, static GUI contract, and local API helper tests: passed.
- Local HTTP/SSE API integration test: passed with escalated local socket permission; default sandbox run skips when binding 127.0.0.1 is not available.
- `python3 scripts/check_all.py`: passed.
- Local preview server restarted on `http://127.0.0.1:8766/` after the approval queue endpoint update.

## Latest Local Preview / Tooling Update

- `scripts/local_api_server.py`
  - `/` and `/app` now serve the static command-center GUI directly.
  - `/local_api_client.mjs` serves the GUI API client module.
  - All JSON/SSE/static responses include local CORS headers for file-based and HTTP-based previews.
  - `OPTIONS` preflight is supported for browser clients.
  - `/tool-catalog` exposes the v0 kernel ToolSpec catalog for GUI inspection.
  - `POST /tool/preview` executes read-only previews for `file.read`, `search.ripgrep`, `repo.map`, and `research.csv_profile`.
  - `research.csv_profile` preview runs the local Python Research Worker sidecar with bounded local policy and no shell.
  - `POST /tool/preview` rejects permission-required tools such as `shell.command`, `patch.apply`, and `artifact.export`.
- `apps/desktop/static_mock.html`
  - Can be opened directly at `http://127.0.0.1:<port>/`.
  - Auto-detects the local API origin when served over HTTP.
  - Renders Project Map, Tool Catalog, summary metrics, model stream timeline, workflow panels, recent events, and review drawers.
- `apps/desktop/local_api_client.mjs`
  - Adds `previewTool()` for browser clients that need to probe read-only tools through the local API boundary.
- `crates/runtime/src/tool_execution.rs`
  - Adds `ToolExecutionService v0` as the shared Rust execution boundary for read-only tool previews and permission-aware apply mode.
  - Supports `file.read`, `search.ripgrep`, `repo.map`, `git.status`, and `research.csv_profile` in `ReadOnlyPreview` mode.
  - `research.csv_profile` executes the bounded Python Research Worker sidecar through the same tool execution boundary, with network/package install disabled by policy.
  - Blocks sensitive paths and path escapes before calling concrete tools.
  - Rejects permission-required tools such as `shell.command`, `patch.apply`, and `artifact.export` in `ReadOnlyPreview` mode.
  - Adds `ApplyWithPermission` mode for `shell.command` and `patch.apply`; permission decisions must already be recorded by the caller.
  - Enforces read-before-write patch validation and stale-base rejection before writes in the shared tool execution boundary.
- `crates/runtime/src/event_invariants.rs`
  - Adds semantic EventLog validation for model/tool/permission/patch lifecycle ordering.
  - Keeps PlanApproval as governance and PermissionRequest as safety approval in separate event families.
- `crates/runtime/src/approval_queue.rs`
  - Adds EventLog-derived approval queues for future TUI/GUI/external-decision wiring without mixing plan approval and security permission types.
- `crates/runtime/src/session.rs`
  - Adds EventLog resume support for blocked sessions, including pending plan approval and pending permission reconstruction.
- `crates/runtime/src/native_agent_loop.rs`
  - Adds typed pending native tool continuations and external-decision resume from a blocked EventLog.
  - Resumed execution records the permission decision, applies the pending tool through the shared execution service, records patch/tool-result events, and can continue later native model steps.
  - Adds a pending-decision package format for approval surfaces: `events.jsonl`, `pending_tool.json`, `resume_package.json`, workspace, and artifacts.
- `crates/runtime/src/sidecar_http_transport.rs`
  - Adds a provider health-check boundary for prepared requests.
  - Keeps health checks disabled by default and reports skipped/healthy/unhealthy status without persisting response bodies.
- `scripts/provider_http_sidecar.py`
  - Adds `mode=stream_visible_text` for real DeepSeek Anthropic SSE terminal streaming.
  - Emits sanitized JSONL events for visible text, reasoning-count telemetry, tool-call notices, usage, HTTP errors, and skipped network/key states.
  - Keeps DeepSeek `thinking_delta` separate from visible output instead of replaying it as ordinary chat text.
- `crates/cli/src/main.rs`
  - Adds `deepseek-stream-visible [prompt]`, a real CLI command that builds a native DeepSeek Anthropic request, enforces live-call gates, streams through the Python sidecar, and prints only user-visible text deltas.
  - Adds `deepseek-stream-tool-visible`, a real DeepSeek Anthropic tool schema smoke command for validating provider `tool_use` streaming before automatic execution is enabled.
  - Adds Agent TUI `/stream-live-deepseek [prompt]`, which reuses the same visible streaming boundary from inside `cargo run -q -p researchcode-cli -- agent-tui`.
  - Fixes the sidecar stdin EOF boundary by closing child stdin after writing the prepared request.
  - Parses sidecar JSONL by reading the `event` field, so it handles standard JSON spacing instead of relying on compact string matching.
  - Real API verification succeeded with `RESEARCHCODE_ALLOW_NETWORK=1`, `RESEARCHCODE_ENABLE_LIVE_PROVIDER=1`, and `DEEPSEEK_API_KEY` in the environment. The command printed: `ResearchCode DeepSeek 原生模式的真实流式输出已成功接通。`
  - TUI verification also succeeded; `/stream-live-deepseek` printed: `DeepSeek 真实流式输出已在 TUI 中成功接通，您可以实时接收响应内容。`
  - Real DeepSeek tool schema verification also succeeded; `deepseek-stream-tool-visible` received `[tool_call name=file_read]`.
- `crates/runtime/src/live_model_request.rs`
  - Adds `build_deepseek_anthropic_request_with_tools` for DeepSeek Anthropic-compatible native tool schemas.
  - Keeps internal ToolSpec names out of provider validation where needed; provider-facing tools can use Anthropic-safe names such as `file_read`.
- `crates/runtime/src/tool_call_parser.rs`
  - Adds provider-facing tool name normalization such as `file_read -> file.read`, `patch_apply -> patch.apply`, and `shell_command -> shell.command`.
  - This preserves the Product Kernel ToolSpec registry while allowing provider-specific tool naming constraints.
- `crates/runtime/src/tool_harness.rs`
  - Adds positive and negative fixtures for every core ToolSpec and fails if a new core tool lacks harness coverage.
- `crates/runtime/src/patch_set.rs`
  - Adds multi-file patch-set dry-run and atomic apply semantics for future diff review.
- `crates/runtime/src/research_harness.rs`
  - Adds Research Worker harness cases for data lineage, privacy classification, resource limits, and package-install policy.
- `crates/cli/src/main.rs`
  - Adds `tool-execution-smoke` to exercise the Rust tool execution boundary from CLI.
  - `tool-execution-smoke` now covers read-only tools, preview denial for shell, authorized shell execution, stale patch rejection, authorized patch application, and Research Worker CSV profiling.
  - Adds `event-invariant-smoke`, `tool-harness-smoke`, `patch-set-smoke`, `research-harness-smoke`, and `foundation-harness-smoke` for lower-layer regression gates.
  - Adds `approval-queue-smoke` and `approval-queue-summary <events.jsonl>` for blocked-session approval extraction.
  - Adds `native-agent-loop-external-resume-smoke` and `native-agent-loop-external-resume-eventlog <events.jsonl>` for EventLog-backed approval resume verification.
  - Adds `native-agent-loop-export-pending-package <package-dir>` and `native-agent-loop-resume-pending-package <package-dir> <allow_once|deny>` as a concrete TUI/local API handoff contract.
  - Adds `provider-health-smoke` and `native-agent-loop-sidecar-live-eventlog <deepseek|qwen> <events.jsonl>` for live provider boundary verification.
- `scripts/local_api_server.py`
  - Adds local HTTP endpoints for creating and resuming native pending-decision packages.
  - Keeps the endpoint under the workspace safe-path policy and delegates execution to the Rust CLI instead of duplicating runtime behavior in Python.
  - Adds optional local API Bearer-token auth and rate limiting for development-server hardening.
- `scripts/run_live_native_eval.py`
  - Adds a no-network-by-default eval that verifies provider health, sidecar-live native loop event logs, event-log validation, and secret-leak guards for DeepSeek/Qwen.
  - Adds `--families` and `--qwen-ollama` so local Ollama Qwen3.6-27B can be promoted independently through the same native event-log contract without requiring a cloud Qwen endpoint.
- `scripts/setup_qwen_ollama_native.sh`
  - Creates/verifies the Ollama alias `Qwen/Qwen3.6-27B -> qwen3.6:27b-coding-nvfp4` and prints the environment needed for ResearchCode native Qwen live runs.
  - Keeps ResearchCode's canonical Qwen3.6-27B native gate intact instead of loosening the runtime to arbitrary Ollama model tags.
- `scripts/check_all.py`
  - Adds event invariant, tool harness, patch-set, research harness, and foundation harness commands to the full verification suite.
- `crates/runtime/src/recorded_agent_loop.rs`
  - `repo.map` is now executable from parsed model tool calls in the recorded agent loop.
  - `file.read`, `search.ripgrep`, `repo.map`, `patch.apply`, and `shell.command` now execute through the shared Rust `ToolExecutionService v0` boundary.
  - `patch.apply` and `shell.command` still require recorded `permission.requested` / `permission.decided` session events before the service receives `ApplyWithPermission`.
  - The execution path records `tool.call_requested`, `tool.call_completed`, and `tool.result_recorded` artifact events like other tools.
- `crates/runtime/src/recorded_research_loop.rs`
  - `research.csv_profile` now executes through the shared Rust `ToolExecutionService v0` boundary instead of calling the sidecar directly from the loop.

## Still Not Done

- TUI command path for live model-produced blocked tool packages is not yet exposed as an interactive slash command; CLI/local API now have the real-live package path.
- Real DeepSeek visible streaming, provider `tool_use` emission, live eventlog, and live pending-package creation work through CLI/local API. Qwen local Ollama live promotion smoke now passes through `qwen3.6:27b-coding-nvfp4` aliased to `Qwen/Qwen3.6-27B`; long-horizon live stress fixtures still need provider evidence.
- Tool Contract Mediation is not yet a single authoritative runtime layer. ToolSpec metadata, parser normalization, low-risk repairs, relation checks, permission policy, telemetry, and evals exist in separate modules, but they are not yet one per-tool contract registry with schema issue-path validation, repair rules, relational invariants, permission policy, telemetry, and eval ownership.
- Crash/resume is strong at blocked permission boundaries, but not yet ClaudeCode-depth for every mid-stream/mid-tool crash boundary. Exactly-once recovery still needs stress tests for tool executed but continuation not sent, streaming fallback after partial tool execution, and interrupted concurrent batches.
- Rust-native SQLite adapter/migrations.
- GUI product integration beyond the static local API command-center preview.
- Real Tauri IPC.
- OS-level sandboxed Research Worker sidecar process beyond best-effort Python `resource` limits.
- DeepSeek native adapter live-run hardening and eval promotion against real provider responses.
- Qwen native adapter live-run hardening beyond local Ollama single-call promotion, especially streaming tool-call and long-horizon eval capture.
- Compatible provider runtime adapter has generic request/response transforms, but not production live baseline evals.
- Local API auth/rate limiting is implemented for the development server, but production local API auth/session binding is not finalized.

## Next Recommended Implementation Order

1. Collapse the scattered tool-validation pieces into a native Tool Contract Mediation layer: generated manifest, alias resolution, streaming accumulator handoff, schema issue-path validator, bounded repair catalogue, relational invariant resolver, model-readable tool errors, permission policy, telemetry, and eval ownership.
2. Add live DeepSeek/Qwen fixture capture mode that records sanitized streaming and non-stream responses for eval without persisting key material, then promote streaming tool-call execution only after those fixtures pass.
3. Harden crash/resume exactly-once behavior across mid-stream fallback, partial tool execution, interrupted concurrent tool batches, and tool-result continuation after restart.
4. Wire pending-decision package creation to a real live model-produced blocked tool call in TUI/local API, not only deterministic fixture packages.
5. Add Rust-native SQLite adapter/migrations or freeze Python SQLite as the v0 persistence boundary.
6. Add OS-level Research Worker sandbox enforcement after platform target is chosen.
7. Add compatible provider live baseline eval smoke without contaminating native DeepSeek/Qwen adapters.

## Release Blockers Remaining

- Live model calls exist behind a disabled-by-default sidecar boundary; DeepSeek text streaming and provider tool-use detection are verified, but real streaming tool-call loops are not yet promoted against live provider evals.
- Real DeepSeek live eventlog and live pending-package generation have been verified locally with network approval; Qwen local Ollama live eventlog promotion smoke is verified with `scripts/run_live_native_eval.py --families qwen --qwen-ollama --require-live`.
- Tool Contract Mediation is still module-level, not a fully centralized contract registry with issue-guided repair telemetry and per-tool eval gates.
- Crash/resume is not yet proven across every mid-stream and mid-tool boundary that a long ClaudeCode-style session hits.
- Research Worker has a bounded Python sidecar, data lineage, artifact hashes, and best-effort OS limit recording, but not a hardened OS sandbox.
- GUI is a placeholder, not a command center.
- No dependency-managed Tauri/React app has been installed or built.
