# 04 ClawCode Rust Analysis

Primary source: `claw-code-main (1)/claw-code-main`.

## 1. Overall Rust Architecture

Observed:
- Root README says `rust/` is the canonical Rust workspace; `src/` and `tests/` are companion Python/reference workspace and audit helpers.
- `rust/Cargo.toml` is a workspace with `members = ["crates/*"]`, edition 2021, MIT, `unsafe_code = "forbid"`.
- `rust/README.md` lists crates:
  - `api`
  - `commands`
  - `compat-harness`
  - `mock-anthropic-service`
  - `plugins`
  - `runtime`
  - `rusty-claude-cli`
  - `telemetry`
  - `tools`

Inferred:
- ClawCode is a parity-driven Rust rewrite of a ClaudeCode-like runtime, with a strong emphasis on CLI compatibility and deterministic harnesses.

## 2. Crate Boundaries

Observed in `rust/crates/runtime/src/lib.rs`:
- Runtime exports conversation, config, prompt, permissions, policy engine, hooks, file ops, bash/sandbox, MCP, OAuth, usage, branch lock, git context, worker boot, recovery recipes, task packets, session store, session control, stale branch/base, telemetry events.

Architecture Decision:
- Our Rust runtime should use similar crates but with product-specific boundaries:
  - `runtime-core`
  - `model-router`
  - `tools`
  - `permissions`
  - `patch`
  - `worktree`
  - `research-worker-bridge`
  - `storage`
  - `server`
  - `cli`
  - `tauri-app`

## 3. Agent Loop

Observed in `runtime/src/conversation.rs`:
- `ConversationRuntime<C,T>` owns `session`, `api_client`, `tool_executor`, `permission_policy`, `system_prompt`, usage tracker, hook runner, compaction threshold, hook abort signal, optional session tracer.
- `run_turn()` pushes the user message, loops model calls, builds assistant messages from stream events, records usage/cache events, extracts pending tool uses, runs pre-tool hooks, evaluates permissions, executes tools, runs post hooks, pushes tool results, and continues until no tool use remains.
- It has a max-iteration guard and a session-health probe after compaction.

Comparison:
- ClawCode's loop is clearer and easier to port than ClaudeCode's UI-integrated loop, but less feature-rich than ClaudeCode.

Recommendation:
- Use this style for Rust runtime core: generic model client and tool executor traits, event tracing, hooks, explicit permission policy.

## 4. Type System and Events

Observed:
- `ApiClient` trait streams `AssistantEvent`.
- `ToolExecutor` trait executes tool calls.
- `AssistantEvent` includes text delta, tool use, usage, prompt cache, message stop.
- `TurnSummary` includes assistant messages, tool results, prompt cache events, iterations, usage, auto compaction.

Recommendation:
- Expand event model to include GUI-specific events: `PlanUpdated`, `PermissionRequested`, `PatchProposed`, `DiffReady`, `CommandStarted`, `ArtifactCreated`, `ReviewCompleted`.

## 5. Permissions

Observed in `runtime/src/permissions.rs`:
- Permission modes: `ReadOnly`, `WorkspaceWrite`, `DangerFullAccess`, `Prompt`, `Allow`.
- `PermissionPolicy` maps tools to required modes and merges allow/deny/ask rules.
- Hook-provided `PermissionOverride` can allow, deny, or ask.
- `authorize_with_context()` checks deny rules first, then hook overrides, ask rules, allow rules, active mode, and optional prompt.
- Permission matching extracts subject from JSON keys such as command, path, file_path, url, pattern, code, message.

Recommendation:
- Borrow the enum shape but make policy subject-aware: command AST, file path scope, network domain, data sensitivity, and project policy.

## 6. File Operations

Observed in `runtime/src/file_ops.rs`:
- `read_file()` checks max size and binary content, supports line windows.
- `write_file()` returns structured patch metadata and original file.
- `edit_file()` performs targeted string replacement and returns patch metadata, original file, replaceAll, and userModified.
- `glob_search()` and `grep_search()` provide search capabilities.

Limitations:
- This implementation is simpler than ClaudeCode/OpenCode: no full read-before-write guard in this module, no LSP integration here, and path boundary validation is present but not broadly wired in the shown read/write paths.

Recommendation:
- Use ClawCode file_ops as a minimal core shape, then add ClaudeCode/OpenCode safeguards.

## 7. Shell and Sandbox

Observed in `runtime/src/bash.rs`:
- `BashCommandInput` includes command, timeout, background flag, sandbox override, namespace/network/filesystem settings, allowed mounts.
- `execute_bash()` chooses sandbox status, supports background processes, async timeout, output truncation, and returns sandbox status.
- It can build Linux sandbox commands or fall back to `sh -lc`.
- It emits a provisional ship event for git push to main/master.

Recommendation:
- Runtime shell execution belongs in Rust for safety and portability. Add a stronger parser/policy layer before execution.

## 8. Prompt and Project Context

Observed in `runtime/src/prompt.rs`:
- `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` separates static and dynamic prompt.
- `ProjectContext` discovers `CLAUDE.md`, `CLAUDE.local.md`, `.claw/CLAUDE.md`, `.claw/instructions.md`.
- `discover_with_git()` reads git status/diff and detects git context.
- `SystemPromptBuilder` composes intro, output style, system section, task section, actions section, dynamic boundary, environment, project context, config, appended sections.

Recommendation:
- Adopt this builder pattern, but add model-profile prompt layers and research-project context.

## 9. Provider, MCP, Plugins, Harness

Observed:
- `api` crate contains Anthropic/OpenAI-compatible providers, SSE, request building, prompt cache.
- Runtime exports MCP client/server/stdio bridge modules.
- Plugins crate handles metadata and hooks.
- Mock parity harness provides deterministic Anthropic-compatible service and CLI scenarios.

Recommendation:
- Build a local mock model server and deterministic fixture harness early. It will make GUI/runtime development independent from API instability and cost.

## 10. Rust/Tauri Implications

Observed:
- ClawCode uses Rust effectively for local shell, file ops, sessions, provider clients, and CLI.
- It already has abstractions that can be exposed through Tauri commands or local HTTP.

Architecture Decision:
- Use Rust for the core local runtime and Tauri+React for GUI.
- Use Python only as a research/data sidecar where the ecosystem is much stronger.

## 11. What to Borrow

1. Rust workspace/crate separation.
2. `unsafe_code = forbid`.
3. Generic `ApiClient` and `ToolExecutor` traits.
4. `ConversationRuntime` loop structure.
5. Permission mode enum.
6. Hook override integration.
7. Prompt builder with dynamic boundary.
8. Structured file-op outputs.
9. Shell sandbox request model.
10. Mock parity harness.
11. Usage/prompt-cache event tracking.
12. MCP as runtime module.

## 12. What Not to Borrow Directly

Observed/Inferred:
- Default permissions in README say `danger-full-access`, inappropriate for local-first product default.
- Runtime is CLI/parity-first, not GUI/task-board-first.
- Research/data workflows are not a first-class module.
- File edit safeguards are not as strong as ClaudeCode's source.

Recommendation:
- Borrow Rust structure, not default safety posture.

