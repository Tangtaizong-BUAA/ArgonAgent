# Autonomous Build TODOs

This is the working build queue. It converts the architecture bundle into executable slices.

## Slice 1: Runtime Kernel CLI

- [x] Minimal Rust workspace.
- [x] Bounded TaskContract action validator.
- [x] Secret scanner for cloud/model/artifact boundaries.
- [x] Permission classifier.
- [x] Patch validator.
- [x] Agent state transition validator.
- [x] In-memory event log.
- [x] Event invariant validator for model/tool/permission/patch lifecycle ordering.
- [x] Approval queue extractor that keeps PlanApproval and PermissionRequest separate for future external decision wiring.
- [x] Event payload support.
- [x] JSONL event export/import.
- [x] CLI commands for checks and runtime primitives.
- [x] Product Kernel ToolSpec registry.
- [x] CLI Agent TUI toolbench for local read/search/repo/git/shell/patch/research tool execution.
- [x] Scriptable Agent TUI mode and smoke test for non-GUI tool-loop regression coverage.
- [x] Agent TUI session event log with tool lifecycle, permission decisions, patch-applied events, and JSONL export.
- [x] Agent TUI context/compaction commands backed by repo map, tracked reads/searches, git status, and model-family context budgets.
- [x] Product Kernel MemoryItem primitive for project facts, user preferences, model failure memory, repo facts, and research memory, with secret-like content rejection and ContextBundle rendering.
- [x] Context compaction preserves active plan and memory provenance so Qwen compact modes do not lose long-task intent or failure lessons.
- [x] Agent TUI patch diff preview before file-write approval.
- [x] Agent TUI scripted ask command that runs the deterministic DeepSeek/Qwen native agent loop and exports its full event log.
- [x] Agent TUI disabled-by-default live DeepSeek ask command with native prompt assembly, sidecar preflight, safe blocked event export, and no raw key persistence.

## Slice 2: Model Boundary

- [x] Native vs compatible model primitives.
- [x] DeepSeek/Qwen native context policy.
- [x] ContextBudgetManager v0 for DeepSeek S3, Qwen S1/S2, and compatible S0 scaffold budgeting.
- [x] Wire native prompt assembler to ContextBudget before enabling full prompt scaffold by default.
- [x] Compatible provider cannot be native.
- [x] Compatible provider config includes env-var key reference, alias, capability hints, transforms, and health-check metadata.
- [x] CLI provider validation.
- [x] Compatible provider request/response transform boundary for OpenAI-compatible, Anthropic-compatible, and explicit custom passthrough configs without native optimization.
- [x] DeepSeek parser fixture runner.
- [x] Qwen parser fixture runner.
- [x] Native adapter trait skeleton.
- [x] Compatible adapter skeleton.
- [x] Native DeepSeek/Qwen provider endpoint gate.
- [x] DeepSeek V4 Flash Anthropic-compatible endpoint metadata without storing raw key material.
- [x] Disabled-by-default DeepSeek Anthropic-compatible request builder.
- [x] Disabled-by-default Qwen3.6-27B OpenAI-compatible/custom request builder.
- [x] Live model executor preflight that records blocked calls without network I/O.
- [x] Provider health-check sidecar mode for DeepSeek/Qwen native endpoints, disabled by default and env-var-key-only.

## Slice 3: Agent Loop Skeleton

- [x] Standard-library local agent runner smoke tests.
- [x] AgentSession governance/safety boundary.
- [x] Policy-driven multi-agent orchestration guard.
- [x] Worktree isolation planner dry-run.
- [x] Plan approval typed payloads.
- [x] Product Kernel Plan/PlanStep primitive with validation, progress counts, next-step selection, and ContextBundle rendering.
- [x] Permission typed payloads.
- [x] Tool request/result payloads.
- [x] Patch proposal lifecycle.
- [x] Command execution placeholder with permission gate.
- [x] Command-output artifact capture placeholder.
- [x] Command-output artifact redaction for secret-like stdout/stderr before persistence.
- [x] Reviewer state path.
- [x] Failure-diagnosis transition helpers.
- [x] No-model coding task fixture runner.
- [x] Coding fixture event-log export and replay validation.
- [x] Tool result artifact links in executor event stream.
- [x] Shell command approval path inside coding fixture.
- [x] Repo map builder for first-pass project structure, important files, and tech-stack context.
- [x] Reusable AgentExecutor skeleton extracted from the fixture.
- [x] Failure diagnosis and repair fixture.
- [x] DeepSeek reasoning replay policy guard.
- [x] DeepSeek stream receiver with reasoning/content/tool/telemetry separation.
- [x] Qwen stream receiver with thinking/content/tool/telemetry separation and canonical deployment validation.
- [x] Model stream event lifecycle for sanitized stream deltas and completion telemetry.
- [x] Model call started/completed boundary events before live provider execution.
- [x] Recorded live-response executor fixture.
- [x] Model event contract validator for stream/call lifecycle event logs.
- [x] Native provider-response adapter boundary for DeepSeek/Qwen stream-to-event conversion.
- [x] Native provider-response adapter boundary for non-stream response-to-event conversion.
- [x] Native non-stream response normalizer for DeepSeek Anthropic and Qwen/OpenAI payloads.
- [x] Structured tool-call parser boundary for native JSON, DeepSeek XML/DSML fallback, argument-string decoding, and low-confidence JSON repair.
- [x] Live-transport model-driven executor fixture: prepared native HTTP response -> normalizer -> model events -> ToolCall parser -> tools.
- [x] Live-transport stream response path: prepared native HTTP stream -> DeepSeek/Qwen SSE assembler -> sanitized model events -> streamed tool_call reconstruction.
- [x] Disabled-by-default provider HTTP sidecar transport boundary that sends prepared requests only when network is explicitly enabled.
- [x] DeepSeek sidecar live smoke path: Product Kernel preflight -> sidecar transport -> event log, with default blocked/skip behavior.
- [x] Qwen3.6-27B sidecar live smoke path: canonical endpoint/profile -> Product Kernel preflight -> sidecar transport -> event log, with default blocked/skip behavior.
- [x] Native prompt assembler for DeepSeek/Qwen modes with stable tool catalog, model-specific reasoning/thinking rules, and ContextBundle rendering.
- [x] Native prompt assembler budget enforcement: scaffold level metadata, active tool limit, dynamic context omission warning, estimated token accounting.
- [x] Reusable native agent loop orchestrator: transport-generic DeepSeek/Qwen model calls -> parser policy -> shared tool execution -> permission events -> review.
- [x] Scripted live HTTP transport for deterministic native agent loop fixtures without network access.
- [x] Native agent loop CLI smoke and event-log export with runtime/model contract validation and SQLite persistence.
- [x] Native agent loop blocked-permission fixture that stops at `WaitingForToolApproval` with a valid event log for GUI approval wiring.
- [x] Blocked-permission event contract validator to distinguish intentional approval waits from failed completed-loop contracts.
- [x] EventLog-backed native loop resume reconstructs blocked-session state and executes a typed pending tool after external approval.
- [x] CLI pending-decision package export/resume contract for future TUI/local API approval surfaces.
- [x] Interactive local API approval endpoint can package an existing blocked session event log plus typed pending tool into the same resumable pending-package contract used by scripted native-loop approvals.
- [x] Native agent loop sidecar-live event-log entrypoint for DeepSeek/Qwen that blocks safely by default and becomes live only with explicit network/provider env gates.
- [x] Native loop v2 converts tool execution errors into structured `tool_result` continuation instead of aborting the agent loop.
- [x] Native loop v2 uses no-tool finalizer turns for repeated tool batches and max-iteration stops so DeepSeek returns a usable answer instead of hanging or failing silently.
- [x] Tool catalog exposes explicit capability status (`production`, `governance_only`, `preview_only`, `gated`) for TUI/GUI/model prompt filtering.
- [x] DeepSeek sidecar preserves complete tool input when providers send `input` in `content_block_start` or final JSON message rather than streaming only `input_json_delta`.
- [x] Provider-facing DeepSeek/Qwen tool schemas are generated from Product Kernel `ToolSpec` instead of hardcoded TUI/runtime copies.
- [x] TUI FastAuto schema exposes file write/edit tools but excludes shell, patch, artifact export, worktree, MCP, and other gated/high-risk tools.
- [x] Native loop v2 read-only/governance schema excludes write tools while still exposing plan/todo/question tools for ClaudeCode-style planning flow.
- [x] TUI `/agent` live DeepSeek path now routes through `RuntimeFacade.run_deepseek_agent_loop_with_transport`, keeping loop recovery, session memory, and event replay on the shared TUI/GUI boundary.
- [x] TUI file-write tool regression covers complete HTML content preservation through parsed tool arguments and Runtime tool execution.
- [x] Native loop v2 routes `ask_user` to `WaitingForUser` with a `user.question_requested` event instead of pretending it is a normal read-only tool result.
- [x] RuntimeFacade routes direct `ask_user` tool execution to `WaitingForUser` with no PermissionRequest, so TUI/GUI/local API clients share the same clarification boundary as native loop v2.
- [x] RuntimeFacade owns DeepSeek live agent token budget selection: chat gets 8k completion budget, generation gets 16k, deep analysis/UltraPlan/UltraReview gets 20k instead of the old 1024-token truncation ceiling.
- [x] Native loop v2 auto-recovers `file.read` directory mistakes by embedding a `repo.map` recovery result in the structured tool_result before asking the model to continue, reducing repeated directory-read LoopGuard stops.
- [x] RuntimeFacade live DeepSeek generation tasks can expose FastAuto write tools to native loop v2, so `file.write` can create complete files through the shared runtime boundary instead of falling back to the old TUI-private loop or text-only answers.

## Slice 4: Storage

- [x] SQLite schema draft.
- [x] JSONL event store.
- [x] File-backed EventStore wrapper.
- [x] Artifact store implementation.
- [x] Artifact manifest/index output.
- [x] SQLite persistence adapter v0.
- [x] Runtime fixture SQLite import and tool-result artifact indexing.
- [x] Runtime fixture SQLite import and model-transcript stream artifact indexing.
- [x] SQLite model_calls indexing from model.call_started/model.stream_completed/model.call_completed.
- [x] SQLite query/export helpers for GUI/API summaries.

## Slice 7: Eval Gates

- [x] Executable DeepSeek/Qwen parser eval gate.
- [x] Executable DeepSeek/Qwen native profile promotion gate.
- [x] Qwen lite-vs-full scaffold eval fixture.
- [x] DeepSeek full-vs-lite scaffold eval fixture.
- [x] Scaffold/context-budget eval gate v0 covering DeepSeek S3, Qwen S1/S2, protected reserves, and active-tool limits.
- [x] Budget-aware model-call telemetry records scaffold level, prompt hash, tool catalog hash, prompt estimate, context budget, dynamic budget, protected reserve, and warning count.
- [x] Budget-aware prompt/tool catalog eval gate that records scaffold level, prompt/tool hashes, and reserve usage.
- [x] Local API and desktop client expose model-call scaffold telemetry for GUI/TUI review surfaces.
- [x] Eval result persistence.
- [x] Coding task fixture runner.
- [x] Coding fixture result persistence in SQLite.
- [x] Build-failure repair fixture.
- [x] Model-planned fixture using recorded parser outputs.
- [x] Recorded patch proposal fixture with stale-file validation.
- [x] DeepSeek reasoning replay unit guard.
- [x] DeepSeek stream parsing unit guard.
- [x] Qwen stream parser unit guard for deployment/thinking/tool/usage cases.
- [x] DeepSeek/Qwen stream parser eval cases beyond unit tests.
- [x] Recorded live-response executor fixture.
- [x] Recorded live-response event-log export and SQLite persistence check.
- [x] Optional live DeepSeek smoke script that skips unless network and env are explicitly enabled.
- [x] Provider-response adapter unit gate for DeepSeek reasoning sanitization and Qwen deployment rejection.
- [x] Prepared live-response recorder that appends DeepSeek/Qwen responses after preflight without duplicate `model.call_started`.
- [x] Prepared live-response recorder CLI event-log export, model-contract validation, and SQLite persistence check.
- [x] Injectable live HTTP transport boundary with recorded DeepSeek/Qwen transport smoke.
- [x] Recorded native agent loop: model response -> parser -> tools -> patch/command approvals -> review.
- [x] Recorded native agent loop now executes from parsed ToolCall arguments instead of raw-output string searches.
- [x] Live-transport native agent loop: streamed model HTTP response bodies feed the same parser/tool/permission/patch/review path.
- [x] Reusable native agent loop fixture: DeepSeek/Qwen live transport -> parser -> ToolExecutionService -> approval events -> event-log/SQLite validation.
- [x] Native agent loop external permission mode returns `Blocked` plus event log instead of losing approval-boundary state as an error.
- [x] AgentSession EventLog resume reconstructs blocked-session state and pending approval boundaries from JSONL.
- [x] Native agent loop external-decision resume executes a typed pending tool after approval without reparsing raw model text.
- [x] Native agent loop pending-decision package writes `events.jsonl`, `pending_tool.json`, workspace, artifacts, and a resume manifest.
- [x] Provider HTTP sidecar unit/CLI guard: skips safely without `RESEARCHCODE_ALLOW_NETWORK=1`, rejects raw key material as `authorization_env`, and keeps response bytes outside stdout.
- [x] Optional DeepSeek sidecar live event-log smoke: blocked-by-default path exports valid `model.call_blocked` event logs and can become live only with explicit env/network approval.
- [x] Optional Qwen sidecar live event-log smoke: blocked-by-default path exports valid event logs and requires canonical Qwen3.6-27B adapter plus resolved `QWEN_BASE_URL` for live use.
- [x] Live native eval runner that validates provider health checks, sidecar-live native loop event logs, and secret-leak guards without requiring network access.
- [x] Real DeepSeek visible terminal streaming through product CLI:
  - `cargo run -q -p researchcode-cli -- deepseek-stream-visible [prompt]`
  - Uses native DeepSeek request construction, disabled-by-default live gates, sidecar SSE reading, sanitized visible `text_delta` output, separate reasoning-count telemetry, and token/cache usage capture.
  - Verified against the real DeepSeek API with explicit network/provider environment gates.
- [x] Real DeepSeek visible streaming from Agent TUI:
  - `cargo run -q -p researchcode-cli -- agent-tui`
  - `/stream-live-deepseek [prompt]`
  - Reuses the same native request/gate/sidecar boundary as the CLI command and prints live visible deltas inside the TUI.
- [x] Real DeepSeek provider tool-use smoke:
  - `cargo run -q -p researchcode-cli -- deepseek-stream-tool-visible`
  - Adds DeepSeek Anthropic-compatible request construction with native tool schemas.
  - Uses provider-facing Anthropic-safe names such as `file_read`, while runtime parser normalization maps them back to Product Kernel ids such as `file.read`.
  - Verified against the real DeepSeek API: the stream emitted `[tool_call name=file_read]`.
- [x] Event replay/session snapshot module for reconstructing state, health, pending approvals, counts, and resume eligibility from JSONL.
- [x] Native agent loop provided-permission resume fixture and CLI event-log export.
- [x] Native agent loop EventLog-backed external-decision resume fixture and CLI event-log export.
- [x] Native agent loop pending-decision package CLI export/resume and resumed EventLog validation.
- [x] Runtime harness suite aggregating coding, repair, parser/model, patch, native loop, blocked approval, resume, and research cases.
- [x] Core tool harness suite covering every Product Kernel ToolSpec with positive/boundary fixtures.
- [x] Multi-file patch-set validation and atomic apply smoke.
- [x] Foundation harness command aggregating runtime, tool, research, event-invariant, and patch-set gates.
- [x] Approval queue CLI smoke and summary command for blocked-session EventLogs.

## Slice 5: Research Worker

- [x] CSV profiler prototype.
- [x] Research manifest sketch.
- [x] Research manifest validation tests.
- [x] Research job runner CLI.
- [x] Artifact manifest output.
- [x] PII report export.
- [x] Rust runtime sidecar launcher.
- [x] Sidecar resource-limit policy checks.
- [x] Research manifest resource-limit recording.
- [x] Analysis script artifact generation.
- [x] Markdown report artifact generation.
- [x] Notebook skeleton artifact generation.
- [x] Manifest hashes for data profile, privacy report, script, report, and notebook artifacts.
- [x] Package install approval policy classifier.
- [x] Package install injection guard.
- [x] Package install approval event integration.
- [x] Notebook/report lifecycle v0.
- [x] Research manifest data lineage edges from input hash to artifact hashes.
- [x] Rust sidecar result reports manifest hash and artifact count.
- [x] Research harness suite for manifest lineage, artifact hashes, privacy approval detection, resource limits, and package install boundaries.
- [x] Recorded Research Coworker loop: Qwen researcher response -> `research.csv_profile` -> Python sidecar -> event log/artifact.
- [x] Research Worker child process records best-effort OS sandbox limits for CPU and address-space in the reproducibility manifest.

## Slice 6: Desktop

- [x] Desktop placeholder.
- [x] Static mock event timeline.
- [x] Static mock JSONL/manifest data.
- [x] Plan approval view mock.
- [x] Permission approval view mock.
- [x] Diff review mock.
- [x] Local API client contract.
- [x] Static mock can consume local API event data.
- [x] Runtime event contract validator for GUI/API consumers.
- [x] Local API `/summary` backed by SQLite import of runtime JSONL.
- [x] Desktop local API client summary reader and command-center view model.
- [x] Static mock renders summary metrics from local API or mock JSONL.
- [x] Desktop local API client model stream timeline builder.
- [x] Static mock exposes model-call count in summary metrics.
- [x] Local API model timeline endpoint.
- [x] Local API session snapshot endpoint reconstructing state, health, pending permissions, pending plan approvals, event counts, and resume eligibility from JSONL.
- [x] Local API latest-run endpoint and static GUI auto-discovery of latest dev fixture bundle.
- [x] Local API approval queue endpoint backed by blocked permission fixture events.
- [x] Local API repo-map endpoint and static GUI project map panel.
- [x] GUI/local API event stream integration test with socket-permission skip fallback.
- [x] Static mock renders model stream timeline details with sanitized reasoning/thinking previews and token telemetry.
- [x] Static mock renders workflow panels for tools, permissions, patches, and commands from event logs.
- [x] Static mock renders the Permission Request drawer from the local API approval queue when latest dev fixture data is available.
- [x] Local API native pending-package endpoints for scripted approval package creation and resume.
- [x] No-network dev fixture bundle generator for local API/static GUI inspection using full agent-loop event data.
- [x] Dev fixture manifest includes model timeline and session snapshot URLs for completed and blocked sessions.
- [x] Dev fixture bundle includes Research Coworker event log for CSV profiling.
- [x] Dev fixture bundle includes blocked-permission event log for approval-panel testing.
- [x] Local API `/` and `/app` directly serve the command-center GUI preview.
- [x] Local API CORS/OPTIONS support for browser-based command-center previews.
- [x] Local API optional Bearer token and standard-library rate limiter for local development server hardening.
- [x] Local API `/tool-catalog` endpoint exposing v0 kernel ToolSpec metadata.
- [x] Local API `POST /tool/preview` for read-only `file.read`, `search.ripgrep`, and `repo.map`.
- [x] Local API rejects permission-required tools from preview execution.
- [x] Desktop local API client exposes `previewTool()`.
- [x] Static mock renders Tool Catalog with permission-gated vs auto tools.
- [x] Recorded agent loop executes parsed `repo.map` tool calls and records tool result artifacts.
- [x] Rust `ToolExecutionService v0` for shared read-only tool preview execution.
- [x] Rust tool execution service blocks sensitive paths, path escapes, and permission-required tools.
- [x] CLI `tool-execution-smoke` and full check hook for the Rust tool execution boundary.
- [x] Recorded agent loop routes `file.read`, `search.ripgrep`, and `repo.map` through the shared Rust tool execution boundary.
- [x] Rust `ToolExecutionService v0` apply mode for permission-aware `patch.apply` and `shell.command`.
- [x] Shell command boundary uses a shared safe tokenizer for classifier and executor: quoted arguments are preserved, while pipes, redirection, command substitution, injected separators, destructive commands, and sensitive paths remain blocked before execution.
- [x] Recorded agent loop routes `patch.apply` and `shell.command` through the shared Rust tool execution boundary after session permission events.
- [x] Tool execution apply mode rejects stale patch base hashes before writes.
- [x] CLI `tool-execution-smoke` covers apply mode for authorized shell, authorized patch, and stale patch rejection.
- [x] Rust `ToolExecutionService v0` executes `research.csv_profile` through the bounded Python sidecar policy.
- [x] Recorded Research Coworker loop routes `research.csv_profile` through the shared Rust tool execution boundary.
- [x] CLI `tool-execution-smoke` covers Research Worker CSV profiling through the shared tool execution boundary.
- [x] Local API `POST /tool/preview` supports `research.csv_profile` through the local Python Research Worker preview path.
- [x] Local API helper and HTTP tests cover `research.csv_profile` preview.
- [ ] Real Tauri IPC bridge.

## Current Focus

Move from bottom-layer safety/protocol coverage into production live-session hardening. The runtime now has blocked-by-default native sidecar live loops, provider health checks, live-native eval, local API auth/rate limiting, typed pending-tool package/resume, and best-effort Research Worker OS limit recording.

Recent hardening completed:

- [x] RuntimeFacade DeepSeek generation prompts expose FastAuto write tools through native loop v2 instead of the legacy TUI-private loop.
- [x] RuntimeFacade Qwen native loop entry supports Qwen/OpenAI-style tool schema, FastAuto `file.write`, tool-result continuation, and event replay.
- [x] Kernel provides separate Anthropic-compatible and OpenAI-compatible provider tool schemas so DeepSeek and Qwen do not share the wrong tool envelope.
- [x] RuntimeFacade supports cursor-based incremental event deltas for future TUI/GUI streaming consumers.
- [x] Legacy TUI-private DeepSeek loop is quarantined as historical comparison; production TUI commands should use RuntimeFacade.

The next bottom-layer slice is production-grade real socket streaming into RuntimeFacade event deltas, followed by Rust-native persistence boundaries.
