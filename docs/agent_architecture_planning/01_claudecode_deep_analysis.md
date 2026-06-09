# 01 ClaudeCode Deep Analysis

Primary source: `Open-ClaudeCode-main/Open-ClaudeCode-main`.

## 1. Overall Architecture

Observed:
- `src/main.tsx` is the application entrypoint. It loads settings, analytics, MCP clients, plugins, skills, models, worktree info, session recovery, sandbox managers, and launches the REPL/UI.
- `src/query.ts` owns the main agent loop.
- `src/Tool.ts` defines the central `Tool` interface used by every model-visible tool.
- `src/tools.ts` assembles the base tool pool and conditionally includes MCP, web, task, LSP, worktree, cron, and agent tools.
- `src/constants/prompts.ts` assembles static and dynamic prompt sections.
- `src/services/compact/compact.ts`, `src/utils/attachments.ts`, `src/memdir/memdir.ts`, and `src/utils/toolResultStorage.ts` manage long context.

Inferred:
- ClaudeCode is not a chat UI with tools bolted on. It is a turn-based runtime where prompt construction, tool admission, streaming tool execution, permission policy, context compaction, and UI state are coordinated inside one loop.

Recommendation:
- ResearchCode Coworker should split this into `AgentRuntime` plus GUI-facing event streams. Runtime owns correctness; GUI owns presentation and approvals.

## 2. Agent Loop

Observed in `src/query.ts`:
- `query()` yields stream events/messages/tool summaries.
- It delegates to `queryLoop`, constructs full system prompt/context, handles auto-compaction through `buildPostCompactMessages`, updates `toolUseContext.messages`, and streams the model via `deps.callModel`.
- It tracks `ToolUseBlock[]`, supports streaming fallback tombstones, backfills observable tool inputs, executes streaming-safe tools through `StreamingToolExecutor`, then appends assistant messages and tool results and continues the loop.
- It handles prompt-too-long errors, fallback model retries, aborts, stop hooks, memory/skill attachments, queued command attachments, and MCP tool refresh.
- `query/tokenBudget.ts` implements continuation based on token budget and diminishing returns.

Architecture Decision:
- Our runtime should implement `AgentStateMachine + EventLog` rather than one monolithic UI hook. Every loop iteration should emit persisted events: model request, assistant delta, tool call, permission request, tool result, patch proposal, command result, reviewer verdict.

## 3. Message and Prompt Organization

Observed:
- `src/constants/prompts.ts` defines `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` to separate cacheable static prompt prefix from dynamic environment and session context.
- `getSimpleSystemSection()` describes permission modes, denial behavior, prompt-injection handling, hooks, and automatic compression.
- `getSimpleDoingTasksSection()` instructs the model to read files before modifying, diagnose failures before switching, avoid unnecessary features, and report verification truthfully.
- `getActionsSection()` lists risky/destructive actions that need confirmation.
- `getUsingYourToolsSection()` tells the model to prefer dedicated tools over shell and to parallelize independent tool reads.
- `getAgentToolSection()` frames subagents/forks as context-protection and parallelization primitives.

Recommendation:
- Use a prompt hierarchy:
  1. Product invariant contract.
  2. Model-profile adapter instructions.
  3. Mode policy: plan/build/review/research.
  4. Tool contract and output format.
  5. Project context.
  6. Session memory.
  7. Current task and current plan.
  8. Recent tool results.

## 4. Tool Contract and Registry

Observed in `src/Tool.ts`:
- Tools include fields such as `call`, `description`, `inputSchema`, `inputJSONSchema`, `outputSchema`, `isConcurrencySafe`, `isEnabled`, `isReadOnly`, `isDestructive`, `interruptBehavior`, `isSearchOrReadCommand`, `isOpenWorld`, `requiresUserInteraction`, `shouldDefer`, `alwaysLoad`, `strict`, `validateInput`, `checkPermissions`, `getPath`, `preparePermissionMatcher`, `maxResultSizeChars`, MCP metadata, prompt and UI render hooks.
- `ToolUseContext` carries options, app state setters, `readFileState`, full messages, memory/skill tracking, content replacement state, and rendered system prompt.

Observed in `src/tools.ts`:
- Base tools include agent/task, Bash, Glob/Grep, ExitPlanMode, Read/Edit/Write, NotebookEdit, WebFetch/WebSearch, TodoWrite, TaskStop, AskUserQuestion, Skill, EnterPlanMode, Config, WebBrowser, Task CRUD, LSP, worktree, MCP resources, ToolSearch, cron/remote/proactive tools.

Recommendation:
- Our `ToolSpec` should include:
  - capability tags: read, write, shell, network, research, git, patch;
  - concurrency safety;
  - permission requirement;
  - path scope extraction;
  - large-output policy;
  - model compatibility;
  - GUI renderer metadata.

## 5. Tool Execution

Observed:
- `src/services/tools/toolOrchestration.ts` partitions tool calls into batches. Concurrent-safe/read-only tools run in parallel; non-concurrency-safe tools serialize.
- `src/services/tools/StreamingToolExecutor.ts` starts concurrent-safe tools as streaming tool calls arrive, keeps result ordering stable, cancels sibling tools on errors, and can discard speculative outputs during streaming fallback.
- `src/services/tools/toolExecution.ts` validates tool input, runs hooks, checks permissions, processes tool result blocks, and records telemetry.

Architecture Decision:
- Use a `ToolDispatcher` that supports:
  - speculative start for read-only/search tools;
  - strict ordering of tool results;
  - cancellation tokens;
  - hook gates;
  - per-tool timeout;
  - event replay.

## 6. File Read/Edit/Write

Observed in `FileReadTool`:
- Supports text, images, PDFs, notebooks, line offsets and limits.
- Blocks device paths, detects token/file-size limits, tracks read state, and checks read permission.

Observed in `FileEditTool`:
- Uses `old_string/new_string/replace_all`.
- Requires file read before edit.
- Checks denied paths, secrets/team memory, max size, file existence, notebook redirection, stale mtime/content, unique `old_string`, and settings validation.
- Preserves encoding and line endings, sends LSP change/save notifications, activates skills by path, tracks file history, updates read state, and returns structured patch/git diff.

Observed in `FileWriteTool`:
- Requires absolute path.
- Requires read-before-overwrite for existing files.
- Performs denial/secrets/mtime checks and returns patch metadata.

Recommendation:
- ResearchCode should implement edits as `PatchProposal` first, not direct write:
  1. model proposes edit;
  2. runtime validates read-before-write/stale content/secrets/path scope;
  3. patch manager creates diff preview;
  4. GUI or policy approves;
  5. runtime applies;
  6. formatter/LSP/test/reviewer runs.

## 7. Shell Execution and Safety

Observed in `BashTool`:
- Schema includes `command`, `timeout`, `description`, `run_in_background`, and internal sandbox override fields.
- Supports background tasks and progress display.
- Classifies search/read commands for UI collapse.
- Output envelope includes stdout, stderr, rawOutputPath, background task ids, persisted output, and sandbox metadata.

Observed in `bashPermissions.ts` and `bashSecurity.ts`:
- Parses shell AST/security patterns.
- Limits subcommands and suggested rules.
- Blocks dangerous shell constructs: process substitutions, command substitutions, zsh qualifiers, zsh dangerous commands, PowerShell comment syntax, and other risky expansions.
- Uses command classifier and permission suggestions.

Recommendation:
- Build a shell policy layer independent from the shell tool:
  - command AST parser;
  - destructive command classifier;
  - workspace/path scope detection;
  - network detection;
  - environment/secret detector;
  - prefix-rule approval;
  - sandbox runner.

## 8. Permission Control

Observed in `src/utils/permissions/permissions.ts` and `src/hooks/useCanUseTool.tsx`:
- Permission rules come from settings, CLI args, commands, session state, and hooks.
- Tools are matched against allow/deny/ask rules, including MCP server-level matching.
- Noninteractive/headless flows can convert ask to deny.
- Auto mode classification tracks denials and can fall back to prompting.
- `useCanUseTool` wires permission checks to interactive UI queues, bridge callbacks, speculative Bash classification, and abort handling.

Architecture Decision:
- Our `PermissionManager` should emit durable `PermissionRequest` records so GUI, CLI, or remote clients can approve the same runtime request.

## 9. Git, Worktree, Session, Memory

Observed:
- `src/utils/git.ts` finds git roots/worktrees with security validation of `.git` commondir/backlinks and memoization.
- `AgentTool` can create worktree-isolated subagents via `createAgentWorktree`.
- `src/memdir/memdir.ts` defines memory entrypoint `MEMORY.md`, memory size limits, taxonomy, and save rules.
- `src/utils/attachments.ts` injects memory files, project context, plan attachments, queued commands, diagnostics, MCP instruction deltas, deferred tool deltas, and auto memory.

Recommendation:
- Treat project memory, session memory, model-failure memory, and research memory as different stores with explicit privacy labels and retention policies.

## 10. Context Window and Compression

Observed in `src/services/compact/compact.ts`:
- Removes media blocks and reinjected attachments.
- Runs PreCompact hooks.
- Builds a summary request and can use a forked-agent path for prompt-cache sharing.
- Retries prompt-too-long by dropping oldest API-round groups.
- Clears readFileState/nested memory and rebuilds post-compact file, async-agent, plan, and skill attachments.

Observed in `src/utils/toolResultStorage.ts`:
- Persists large tool results to session storage and returns previews using `<persisted-output>`.
- Keeps `Read` outputs unpersisted by default so actual file contents remain available to the model when needed.
- Maintains content-replacement state to stabilize prompt cache.

Recommendation:
- Use three layers:
  - lossless artifact store for full tool/file outputs;
  - lossy compressed context for model calls;
  - structured memory for facts/decisions/failure modes.

## 11. Planning, Subagents, Hooks, MCP

Observed:
- `TodoWriteTool` stores plan/checklist state and nudges verification on larger plans.
- `AgentTool` supports subagents with `description`, `prompt`, `subagent_type`, optional model, background execution, team/name/mode/isolation/cwd, and worker permission defaults.
- `runAgent.ts` launches subagent `query()` with its own tool context, MCP servers, read state clone, transcript, hooks, max turns, and cleanup.
- MCP appears in tool pool assembly and resource/command/tool injection.

Recommendation:
- In GUI, render subagents as child task lanes with explicit model, workspace, tool permissions, logs, and merge status.

## 12. Error Recovery and Hallucination Reduction

Observed mechanisms:
- Prompt requires reading files before editing.
- Edit/write tools enforce read-before-write and stale-content checks.
- Bash security blocks high-risk constructs.
- Tool validation rejects invalid inputs.
- Permission denial returns tool results to model, letting it recover.
- Prompt-too-long retries drop older API-round groups.
- Todo/verification nudges encourage self-check.
- Hooks can abort or modify tools.

Architecture Decision:
- Error recovery should be first-class states: `DiagnosingFailure`, `ContextRefresh`, `RetryWithChangedStrategy`, `HumanEscalation`.

## 13. Top 20 Mechanisms to Borrow

1. Static/dynamic prompt boundary.
2. Read-before-edit enforcement.
3. Stale mtime/content edit checks.
4. Structured patch plus git diff return.
5. Tool concurrency classification.
6. Streaming tool executor for read-only tools.
7. Large tool-result persistence with preview.
8. Permission rule sources with allow/deny/ask.
9. Shell AST/security classifier.
10. Hooks around tool use and compaction.
11. Session-level memory file contract.
12. Automatic context compaction with post-compact attachments.
13. Todo/plan tool visible to model and user.
14. Subagent isolation and worktree support.
15. MCP tool/resource injection.
16. LSP diagnostics after edits.
17. UI-specific tool render metadata.
18. Prompt instructions against prompt injection.
19. Explicit destructive-action confirmation.
20. Event telemetry for model/tool/runtime behavior.

## 14. What Not to Copy Directly

Observed/Inferred:
- The source is tightly integrated with its TUI/React state and Anthropic-specific behavior.
- Many gated/internal/ant-only tools do not map to our product.
- Some permissions are optimized for CLI flow; GUI needs durable, multi-client approval requests.
- Direct file writes inside tools are acceptable for CLI but GUI product should prefer patch proposals and review queues.

Recommendation:
- Copy patterns, not code shape. Use Rust runtime modules plus GUI event model.

## 15. GUI Translation

CLI/TUI mechanism to GUI mapping:
- Agent loop -> session timeline with state badges.
- Tool calls -> collapsible event cards.
- Permission ask -> command/file/network approval drawer.
- Patch result -> diff review panel.
- TodoWrite -> plan sidebar.
- Subagent -> lane/task child card.
- Compaction -> context event and summary artifact.
- MCP/plugins/skills -> settings catalog.
- Hooks -> automation/rules view.

