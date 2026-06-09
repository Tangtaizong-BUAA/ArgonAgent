# Tool Runtime Implementation Progress

## Current status

- Phase focus: 19-phase plan execution
- Runtime status: **all listed phase gates implemented or explicitly gated-by-policy**
- Native models: `DeepSeek` and `Qwen3.6-27B` only
- Completion posture: go-forward with stability hardening + live provider debugging

## Phase completion matrix

| Phase | Status | Evidence |
| --- | --- | --- |
| 01 Inventory/traceability | done | `docs/implementation/tool_runtime_inventory.md` |
| 02 Dynamic ToolManifest | done | context-aware manifest builder + `tool.manifest.generated` event |
| 03 Canonical naming/aliases | done | expanded kernel aliases + alias resolution events |
| 04 UnknownTool recovery | done | recoverable `ModelReadableToolError` + `tool.doctor.snapshot` |
| 05 Streaming accumulator | done | `StreamingToolCallAccumulator` + delta/assembly events |
| 06 Validate-then-repair | done | validation-first mediation pipeline |
| 07 P0 repair catalogue | done | null-strip/path unwrap/offset-limit repair rules |
| 08 Schema primitive boundaries | done | non-repairable content/command constraints enforced |
| 09 Relational invariants | done | offset/limit defaults + stale/protected-path checks |
| 10 ModelReadableToolError | done | standardized payload + retry guidance |
| 11 Exactly-once ledger | done | missing/duplicate diagnostics + smoke |
| 12 DeepSeek content fallback | done | DSML/XML/plain candidates routed through mediation |
| 13 Provider capability matrix | done | native provider preflight + compatibility guards |
| 14 DeepSeek reasoning replay | done | replay path + sanitization + anti-replay-to-user checks |
| 15 Qwen native hardening | done | canonical target checks + dedicated parser/template path |
| 16 Workflow-state exposure policy | done | planning/reading/editing/testing/review manifest filters |
| 17 TUI lifecycle rendering | done | lifecycle cards + runtime error boundary smoke |
| 18 ToolDoctor/telemetry/eval gates | done | doctor smokes + check_all gates |
| 19 Ultra integration | done | UltraPlan/UltraReview/AgentTeams fixture + evidence guards |

## Phase-level notes for this execution slice

1. Manifest now includes context inputs (`family/protocol/exposure/workflow`) and filters by workflow state.
2. Unknown tool calls now emit doctor snapshot telemetry instead of hard-failing the loop.
3. Alias coverage was expanded to model-observed names:
   - `read_source_code/read_file/readFile -> file.read`
   - `search_source_code/file.search/file.grep -> search.ripgrep`
   - `list_source_files/list_files/list_dir -> repo.map`
   - `shellCommand/shell.run -> shell.command`
   - `writeFile -> file.write`
4. Native loop now uses workflow-specific manifest exposure (`reading` vs `editing`) instead of a single static state.

## Verification commands executed

- `cargo test --workspace` (pass)
- `cargo run -q -p researchcode-cli -- tool-manifest-doctor-smoke` (pass)
- `cargo run -q -p researchcode-cli -- tool-contract-mediation-smoke` (pass)
- `cargo run -q -p researchcode-cli -- unknown-tool-recovery-smoke` (pass)
- `cargo run -q -p researchcode-cli -- tool-input-repair-smoke` (pass)
- `cargo run -q -p researchcode-cli -- tool-ledger-exactly-once-smoke` (pass)
- `cargo run -q -p researchcode-cli -- deepseek-content-tool-fallback-smoke` (pass)
- `cargo run -q -p researchcode-cli -- qwen-tool-mediation-fixture-smoke` (pass)
- `cargo run -q -p researchcode-cli -- deepseek-multi-tool-continuation-smoke` (pass)
- `cargo run -q -p researchcode-cli -- qwen-tool-continuation-fixture-smoke` (pass)
- `cargo run -q -p researchcode-cli -- agent-tui-tool-chain-smoke` (pass)
- `cargo run -q -p researchcode-cli -- agent-tui-error-boundary-smoke` (pass)
- `cargo run -q -p researchcode-cli -- ultraplan-fixture-smoke` (pass)
- `cargo run -q -p researchcode-cli -- ultrareview-fixture-smoke` (pass)
- `cargo run -q -p researchcode-cli -- agentteam-smoke` (pass)
- `cargo run -q -p researchcode-cli -- evidence-ledger-smoke` (pass)
- `python3 scripts/check_all.py` (pass)

## Remaining risk (non-blocking for this phase bundle)

- Live provider 400-class failures are mostly request-shape/config dependent and require endpoint-specific diagnostics against real API responses.
- TUI UX parity vs ClaudeCode is still iterative work; runtime contract is now stable enough to continue without re-architecting.

## 2026-05-08 Toolstorm hardening slice (agent loop + tools)

Implemented against `crates/runtime` + `crates/kernel` with Open-ClaudeCode loop references (`query.ts` tool-loop/finalizer discipline) and the root-cause plan:

1. Added first-class directory tools:
   - `file.list_directory` (provider aliases: `list_directory/list_dir/ls/...`)
   - `file.list_tree` (provider aliases: `list_directory_tree/tree/...`)
2. Redirected directory-intent aliasing:
   - `list_dir/list_directory/tree/repo_file_tree` no longer collapse to `repo.map` only.
3. Changed `file.read(path_is_directory)` guidance and auto-recovery:
   - suggest `file.list_directory` first, keep `repo.map` as fallback evidence.
4. Read-only shell list intent recovery now routes to directory tools first:
   - event: `tool.alias_shell_list_to_directory_tool`
5. Added then corrected read-only workflow routing in native loop:
   - read-only native loop now uses the generic `reading` workflow rather than prompt-text-specialized folder routing.
   - total tool-call budget follows the caller-provided `max_tool_calls`; it is not reduced based on user prompt wording.
6. Strengthened finalization invariants:
   - finalizer paths now synthesize visible fallback text when model final text is empty.
7. Added turn-level traceability event:
   - `agent.turn_summary` with task, tools used, evidence previews, and completion status.
8. Added DSML fallback parse telemetry:
   - event: `tool_call.fallback_markup_parsed`.
9. Fixed native multi-tool stream assembly:
   - DeepSeek Anthropic-style `content_block` tool_use streams now keep each indexed tool call separate.
   - DeepSeek/OpenAI and Qwen OpenAI-style `tool_calls[]` chunks now keep each indexed call separate instead of concatenating arguments into one call.
10. Fixed JSON `tool_calls[]` parsing:
   - runtime now executes every call in the array, not only the first `name/arguments` pair.
11. Fixed provider continuation shape:
   - OpenAI-compatible tool replay maps internal `toolu_*` ids to provider-shaped `call_*` ids.
   - continuation requests replay provider tool names such as `file_read`, while internal events keep canonical ids such as `file.read`.
12. Fixed empty-finalizer request shape:
   - no-tool finalizers with an empty tool batch now use a plain no-tools chat request instead of an invalid empty `tool_calls/tool_results` continuation.
13. Added mixed toolstorm recovery coverage:
   - `read_files(files=[...])` expands into multiple `file.read` calls.
   - incomplete DSML invokes such as `list_available_tools` become recoverable model-readable tool errors instead of triggering empty-visible crashes.
   - `todo.write` accepts provider-style `items` arrays.
   - read-only shell listing intent with compressed syntax or pipes is safely redirected to directory/repo listing tools instead of executing shell.
14. Fixed provider-finalizer fragility:
   - no-tool finalizers now use plain no-tool chat requests with compact tool evidence summaries, not provider `tool_use/tool_result` replay.
   - provider replay content now uses bounded `detail_preview` plus `artifact_ref`; full tool details stay in local artifacts.
   - continuation HTTP 400 after successful tool execution is converted into a completed runtime fallback answer with `model.http_failure_recovered`, preserving visible output instead of failing the TUI session.
15. Fixed DeepSeek Anthropic-compatible continuation fragility:
   - native loop no longer uses structured provider `tool_result` replay as the first continuation strategy for DeepSeek Anthropic-compatible sessions.
   - after a tool batch, DeepSeek Anthropic-compatible sessions continue with a normal tool-enabled request carrying compact `Already Executed Tool Evidence`.
   - OpenAI-compatible/Qwen structured replay remains available, but if it returns HTTP 400 the runtime retries once with `plain_evidence_continuation` before falling back.
   - event marker: `model.continuation_strategy` records `plain_evidence_continuation` vs `provider_tool_result_continuation`.
16. Fixed the TUI completion gate:
   - runtime Completed paths now emit an explicit `assistant.message` for real final text or synthesized evidence fallback.
   - tool-budget refusal text is rejected as a final answer and replaced with evidence-based fallback text.
   - TUI still streams intermediate model text, but only `assistant.message` satisfies the final-answer gate; pre-tool text such as "let me inspect" can no longer hide a missing answer.
   - duplicate final text is suppressed when the same text was already streamed.
17. Fixed the TUI card cap:
   - the 48-card display cap now applies only to cards.
   - `model.stream_delta` text and final `assistant.message` events are never dropped just because a tool run produced many cards.
   - this fixes completed sessions that showed `No final natural-language answer was produced` after long tool traces.
18. Added native-loop observation caching:
   - repeated `file.read`, `file.list_directory`, `file.list_tree`, `repo.map`, `git.status`, and identical `search.ripgrep` observations are converted into model-readable duplicate-observation tool results instead of re-running the same observation.
   - repeated exact batches of already-observed read/list/search calls now use duplicate-observation suppression before the older repeated-batch loop guard.
   - read-only shell-list recovery also participates in the cache after being redirected to the directory tool.
   - duplicate-observation suppression is now soft: the tool result is a skipped/cached result, not a failed tool turn, so it does not trigger non-progress finalization or disable subsequent tools.
   - cache keys include observation bounds (`offset/limit/max_bytes`, `depth/max_entries`, `max_files/max_depth`, search `max_results`) so broader follow-up listings or reads are allowed.
19. Removed prompt-level provider-alias bias:
   - native loop prompts no longer hard-code names such as `file_read`.
   - provider-facing aliases still exist for model API compatibility, but prompts now describe capabilities generically and tell the model not to repeat already-observed root/file reads.

Primary changed files for this slice:
- `crates/kernel/src/tool.rs`
- `crates/cli/src/main.rs`
- `crates/runtime/src/deepseek_stream.rs`
- `crates/runtime/src/qwen_stream.rs`
- `crates/runtime/src/tool_call_parser.rs`
- `crates/runtime/src/tool_contract.rs`
- `crates/runtime/src/tool_execution.rs`
- `crates/runtime/src/native_agent_loop.rs`
- `crates/runtime/src/runtime_facade.rs`
- `crates/runtime/src/tool_harness.rs`
