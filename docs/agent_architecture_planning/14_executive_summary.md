# 14 Executive Summary

## What Was Scanned

Identified and analyzed:
- ClaudeCode-like source: `Open-ClaudeCode-main/Open-ClaudeCode-main`.
- OpenCode: `opencode-dev (1)/opencode-dev`.
- DeepSeek-TUI: `DeepSeek-TUI-main/DeepSeek-TUI-main`.
- ClawCode Rust rewrite: `claw-code-main (1)/claw-code-main/rust`.
- Anthropic official public ClaudeCode repo/docs/examples: `claude-code-main/claude-code-main`.
- DeepSeek V4 paper: `DeepSeek_V4.pdf`.
- Qwen3.6-27B public model card and Qwen function-calling docs for Qwen native mode design.

## Main Finding

The product should not be a chat UI. It should be a local-first agent runtime with a GUI command center, typed tools, patch/permission/worktree managers, model router, research worker, eval harness, and durable event log.

Updated native model scope:
- The code-agent runtime should natively support two optimized modes first: DeepSeek and Qwen3.6-27B.
- ClaudeCode is not excluded. ClaudeCode is the main reference for how to tune an agent scaffold around a specific model family. Its Claude-specific adaptation pattern should be translated into DeepSeek and Qwen3.6-27B adapters.
- Claude/OpenAI/Codex/GPT/GLM/local/custom providers are compatible-only: they can be manual options, baselines, or explicitly approved fallbacks, but they do not receive native optimization and do not enter DeepSeek/Qwen native eval promotion.

## Recommended Architecture

Prototype stack direction:
- GUI: Tauri + React.
- Runtime: Rust.
- CLI/TUI: Rust, same runtime API.
- Research Worker: Python sidecar.
- Storage: SQLite plus local artifact store.
- IPC: Tauri commands first; local HTTP/WebSocket only after local auth/streaming spike.

Why:
- Rust is best for safe local shell/files/worktrees/runtime.
- React/Tauri gives a lightweight desktop command center.
- Python is the right runtime for Pandas/Polars/DuckDB/notebooks.
- SQLite is enough for local-first persistence and later sync.

## ClaudeCode: 10 Mechanisms to Borrow

1. Main stream/tool loop in `src/query.ts`.
2. Rich `Tool` contract in `src/Tool.ts`.
3. Static/dynamic prompt boundary in `src/constants/prompts.ts`.
4. Read-before-edit and stale file checks in `FileEditTool`.
5. Bash security and permission classification.
6. Streaming/concurrent tool executor.
7. Large tool-result persistence and previews.
8. Compaction with post-compact attachments.
9. Subagent/worktree isolation through `AgentTool`.
10. Hooks/MCP/skills as extensibility layer.

Claude-specific scaffold tuning to absorb:
- Canonical Claude family config mapped to provider-specific IDs.
- Model/provider capability gates for context, output tokens, thinking, strict tools, and streaming behavior.
- Session-stable prompt/tool schema bytes to protect cache behavior.
- Thinking/adaptive-thinking policies selected by model support.
- Tool-use/tool-result pairing repair before API submission.
- Latched beta headers and feature flags to avoid mid-session cache churn.

## OpenCode: 10 Mechanisms to Borrow

1. Modular `session/prompt.ts` loop.
2. Stream processor with persisted parts.
3. Provider abstraction through AI SDK.
4. Model-aware tool registry.
5. Agent definitions: plan/build/explore/summary.
6. SQLite session/message/part schema.
7. Worktree manager.
8. Tree-sitter shell analysis.
9. Server/API architecture.
10. Plugin/config system.

## DeepSeek-TUI Real Optimization Points

Real:
- V4 1M context profile.
- Thinking-mode `reasoning_content` replay/sanitizer.
- Reasoning replay token telemetry.
- Prefix-cache-aware prompt layering.
- Sorted/memoized tool catalog.
- High compaction threshold and 500K auto-compaction floor.
- DSML/XML fallback parser.
- Deterministic JSON argument repair.
- Cache hit/miss usage tracking.
- Strict tool mode support.

Not enough alone:
- Merely changing base URL/model name.
- Using DeepSeek without model-specific context/tool/reasoning policy.

## DeepSeek V4 Paper Impact

Architecture implications:
- Preserve reasoning traces only in real tool-calling flows.
- Use Pro/Max for planning, long-context research, hard diagnosis.
- Use Flash for cheap exploration/summarization, not final high-risk coding.
- Compact less aggressively for V4 because prefix-cache economics matter.
- Evaluate tool-call success and reasoning replay cost explicitly.

## Qwen3.6-27B Impact

Architecture implications:
- Qwen mode targets `Qwen/Qwen3.6-27B`, not Qwen2-7B.
- Qwen3.6-27B should have a dedicated native adapter with thinking, non-thinking, and preserve-thinking policies.
- Qwen tool use must be parser/template aware; generic OpenAI-compatible transport is insufficient unless the serving stack enables Qwen-compatible reasoning/tool parsers.
- Use 262K native context deliberately, prefer at least 128K where deployed, and treat extended context beyond 262K as a verified deployment feature.
- Eval must separately measure Qwen parser success, thinking preservation benefit, patch success, long-context behavior, and coding/research role thresholds.

## ClawCode Rust/Tauri Lessons

Borrow:
- Rust crate separation.
- Generic model client/tool executor traits.
- Clear `ConversationRuntime`.
- Permission modes.
- Prompt builder with dynamic boundary.
- Rust shell/sandbox/file modules.
- Mock parity harness.

Avoid:
- `danger-full-access` defaults.
- CLI-only assumptions.
- Treating research workflows as out of scope.

## Target Product

ResearchCode Coworker should have:
- Desktop command center.
- Agent runtime state machine.
- CLI/TUI.
- Model router and model profiles.
- Tool registry.
- Context manager.
- Patch manager.
- Permission manager.
- Worktree manager.
- Research Worker.
- Eval harness.
- Memory system.
- Skill/automation system.
- Observability/audit logs.

## Hardening Review Added

The second-pass review is captured in `16_architecture_gap_review.md`.

Key corrections:
- GUI/CLI use Runtime API only; runtime/storage owns SQLite writes.
- Core types now include lifecycle, creator, consumer, and persistence mapping.
- Model calls now persist DeepSeek/Qwen native telemetry: mode, adapter version, parser flags, thinking settings, prompt/tool hashes, deployment stack, and context size.
- Qwen3.6-27B profiles now include sampling/deployment structures and parser capability checks.
- Research Worker now has concrete sandbox, lineage, PII masking, and reproducibility requirements.
- Phase 0 now requires a clean-room/reference-use policy before implementation code.

## Current Convergence Update

The project is not ready for scaffold yet. The latest convergence pass adds:

- final DeepSeek/Qwen native-first model scope;
- CompatibleProvider and ModelAliasMapping layer for other providers;
- multi-agent policy with default Single Agent + Reviewer;
- TaskContract-bounded autonomy;
- PlanApproval separated from PermissionRequest;
- Phase 0 execution order and go/no-go checklist.

## Next 3 Codex Tasks

1. Create kernel schema consistency drafts, including `PlanApprovalRequest/Decision` separated from `PermissionRequest/Decision`.
2. Create `ProviderConfig` / `ModelAliasMapping` schema spike for compatible providers without touching native DeepSeek/Qwen adapters.
3. Create DeepSeek and Qwen parser/executor fixture spikes required by native eval gates.
