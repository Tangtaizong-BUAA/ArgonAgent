# 02 OpenCode Deep Analysis

Primary source: `opencode-dev (1)/opencode-dev`.

## 1. Overall Architecture

Observed:
- OpenCode is a TypeScript/Bun monorepo with the main package at `packages/opencode`.
- It separates session loop, streaming processor, provider/model calls, tool registry, permissions, worktree management, config, plugin loading, storage, server routes, and TUI.
- It exposes both CLI/TUI and server/API surfaces.

Inferred:
- OpenCode is a strong reference for a productized open-source agent runtime: its runtime concepts are less intertwined with one vendor than ClaudeCode.

Recommendation:
- Use OpenCode's modular runtime boundaries and persistent storage shape as the default control-plane inspiration.

## 2. Entry and TUI Structure

Observed:
- One-shot/automation path: `packages/opencode/src/cli/cmd/run.ts`.
- TUI files live in `packages/opencode/src/cli/cmd/tui/*`.
- `run.ts` formats tool parts for terminal output and supports options for `--continue`, `--session`, `--fork`, `--share`, `--model`, `--agent`, and JSON output.
- TUI components include dialogs for model, provider, MCP, agent, sessions, commands, skills, status, workspace creation, and themes.

Recommendation:
- Our CLI/TUI should be a thin client over the same runtime API used by GUI, not an independent runtime.

## 3. Session Loop

Observed in `packages/opencode/src/session/prompt.ts`:
- `prompt()` creates a user message, stores per-session permission overrides, then calls `loop()`.
- `runLoop()` sets session status, retrieves non-compacted messages, finds last user/assistant/finished state, handles pending subtasks and compactions, exits when no tool calls remain, generates title/summary in background, resolves model and agent, injects reminders, resolves tools, applies plugin message transforms, constructs system prompt, and calls the LLM handler.
- Plan mode injects a system reminder prohibiting edits except plan-file edits.
- The loop can continue after tool calls, compaction, or structured-output tool completion.

Architecture Decision:
- Adopt OpenCode's split between `SessionPrompt` orchestration and `Processor` stream persistence.

## 4. Model Abstraction

Observed in `packages/opencode/src/session/llm.ts`:
- Uses Vercel AI SDK `streamText`.
- Builds system from provider prompt, agent prompt, custom/user system.
- Provider-specific prompts come from `SystemPrompt.provider`.
- Plugin hooks can transform system messages, params, and headers.
- Handles model options from provider/model/agent/variant.
- Includes special compatibility behavior such as `_noop` tool injection when history has tool calls but active tools are empty.
- `experimental_repairToolCall` lowercases tool names if possible or routes to `invalid` tool.

Recommendation:
- Our `ModelRouter` must be explicit rather than hidden in provider glue: role, budget, retry policy, output parser, tool-call repair, and fallback rules belong in `ModelProfile`.

## 5. Tool System

Observed:
- `packages/opencode/src/tool/tool.ts` defines `Tool.Def` with `id`, `description`, Effect Schema parameters, `execute(args, ctx)`, and validation formatting.
- `Tool.Context` has session/message IDs, agent, abort signal, callID, message history, live metadata, and `ask()` permission helper.
- `packages/opencode/src/tool/registry.ts` registers builtins and custom JS/TS plugin tools.
- Registry filters tools by model: GPT non-OSS/non-gpt4 gets `apply_patch` and disables edit/write; others get edit/write and no apply_patch.
- Built-ins include invalid, question, shell, read, glob, grep, edit, write, task, fetch, todo, search, skill, patch, LSP, plan.

Recommendation:
- Tool registry should be model-aware, but model-specific tool availability must be visible in GUI and eval logs.

## 6. File Edit and Patch Strategy

Observed:
- `tool/edit.ts` uses a file lock semaphore, BOM/line-ending preservation, path normalization, permission ask with diff metadata, formatter integration, watcher events, and LSP diagnostics in output.
- It uses fallback matchers: simple exact match, line-trimmed, block anchor with Levenshtein.
- Registry switches some models to `apply_patch` instead of direct edit/write.

Recommendation:
- Implement both:
  - structured replace/edit for models that reliably call exact file-edit tools;
  - apply-patch fallback for models that do better with patch syntax.

## 7. Shell Execution

Observed in `tool/shell.ts`:
- Uses tree-sitter wasm for Bash/PowerShell parsing.
- Scans path arguments for external-directory permissions.
- Has default timeout, metadata limit, tailing output, and permission asks for command patterns and external dirs.

Recommendation:
- Tree-sitter parsing is a good cross-platform path for shell safety.

## 8. Permission

Observed:
- `permission/evaluate.ts`: last matching wildcard rule wins, default is ask.
- `permission/index.ts`: evaluates agent/session/project approved rules, publishes `permission.asked`, waits for reply, persists `always` approvals in the project, and rejects all pending requests when a rejection arrives.
- Agents can define permission defaults.

Recommendation:
- Permission requests should be indexed by `session_id`, `tool_call_id`, `scope`, and `decision_source`. GUI and CLI can both answer the same request.

## 9. Agents and Planner/Executor Separation

Observed in `agent/agent.ts`:
- Default agents: `build`, `plan`, `general`, `explore`, `compaction`, `title`, `summary`.
- Plan denies edit except plan files.
- Explore can only grep/glob/list/bash/web/read.
- Agents are configurable by model, prompt, tools, permission, and step count.

Recommendation:
- Product should use named runtime roles: Planner, Executor, Reviewer, Researcher, DataAnalyst, Summarizer, Explorer, Compactor.

## 10. Context and Compaction

Observed in `session/compaction.ts`:
- Selects head/tail; default preserves two recent turns and 2K-8K recent tokens or 25% of usable window.
- Summary template includes Goal, Constraints, Progress, Decisions, Next Steps, Critical Context, Relevant Files.
- Tool outputs are truncated and old summaries pruned.

Recommendation:
- Use structured compaction outputs, not free-form summaries. They should be queryable in GUI and memory.

## 11. Session Persistence

Observed in `session/session.sql.ts`:
- SQLite/Drizzle tables include session, message, part, todo, v2 session message, and permission.
- Processor writes stream parts as they arrive, including text, reasoning, tool input, tool results, and patch parts.

Recommendation:
- Adopt event-sourced persistence: every model/tool/permission/patch action becomes an append-only event, with materialized views for GUI.

## 12. Worktree and Git

Observed in `worktree/index.ts`:
- Creates git worktree under global data path using branch `opencode/<slug>`.
- Populates with hard reset, runs start scripts, and loads instance state.
- Remove force-removes worktree and branch.
- Reset refuses primary workspace, fetches default branch, hard resets, cleans, updates submodules, and checks status.

Recommendation:
- Our GUI multi-agent model should make worktree state visible: branch, base commit, dirty files, merge conflicts, tests, and merge eligibility.

## 13. Config and Plugins

Observed in `config/config.ts`:
- Config includes shell, server, commands, skills, plugin, model, small_model, default_agent, agent definitions, provider, MCP, formatter, LSP, instructions, permission, tools, tool output limits, compaction, and experimental flags.
- Plugins are resolved relative to config provenance.

Observed in `plugin/index.ts`:
- Loads internal and external plugins.
- Provides client/project/worktree/directory/server URL to plugins.
- Sequentially registers plugin hooks for deterministic order.
- Plugins can transform config, messages, model params, tools, and receive events.

Recommendation:
- Plugin surfaces need capability manifests and sandboxing. Do not allow arbitrary plugin code to run with full runtime trust in team mode.

## 14. Server/API

Observed in `server/server.ts`:
- Supports Hono and Effect HttpApi backends.
- Exposes global, control-plane, workspace, instance, UI, and experimental HTTP API routes.
- Has auth, CORS, compression, middleware, WebSocket upgrade paths, OpenAPI generation, and mDNS.

Recommendation:
- ResearchCode should expose a local runtime API from day one:
  - WebSocket/SSE event stream;
  - REST for projects/tasks/sessions/artifacts/settings;
  - IPC wrapper inside Tauri.

## 15. Comparison With ClaudeCode

OpenCode advantages:
- Cleaner open architecture and server API.
- More explicit agent definitions.
- Built-in worktree service.
- Storage schema easier to borrow.
- Plugin/config system is product-friendly.

ClaudeCode advantages:
- More mature tool safety and edit correctness.
- Richer prompt construction.
- Stronger context attachment/compaction system.
- Deeper subagent/worktree integration.
- More nuanced permissions and shell security.

Architecture Decision:
- Use ClaudeCode for deep runtime behavior, OpenCode for product control-plane structure.

