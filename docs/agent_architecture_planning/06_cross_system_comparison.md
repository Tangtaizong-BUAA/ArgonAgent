# 06 Cross System Comparison

## Comparison Matrix

| System | agent loop | planner | executor | reviewer | tools | context | file edit | shell | permission | git | patch | session | memory | compression | subagent | hooks | MCP | TUI | GUI | model profile | eval | research/data | extensibility |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| ClaudeCode-like source | Strong `src/query.ts` recursive stream/tool loop | Todo/plan/plan-mode | Rich tool executor | Verification nudges and agent roles | Very rich `Tool.ts` | Attachments, memory, tool result store | Read-before-edit, stale checks | Bash AST/security | Multi-source allow/deny/ask | Git/worktree utilities | Structured patch/git diff | Session recovery/storage | `memdir` | Robust compact service | `AgentTool` with isolation | Pre/Post/Compact | Built in | React Ink | Not native GUI | Model choices but vendor-centric | Telemetry, not full eval | Low | Plugins/skills/MCP/hooks |
| OpenCode | Strong `session/prompt.ts` + `processor.ts` | Explicit `plan` agent | `build` agent | Specialized agents | Built-in + custom JS/TS tools | System/env/skills/compaction | Edit/write/apply_patch model-aware | tree-sitter parser | Project/session rules | Worktree manager | Patch parts/snapshots | SQLite parts | Instructions/skills | Structured summaries | Task tool/subagents | Plugin hooks | Built in | Rich TUI | Server/UI routes | Provider/model abstraction | Tests, snapshots | Low | Config/plugins/server |
| DeepSeek-TUI | Rust engine loop | Plan mode/cycle | Tool registry | Some eval/recovery | DeepSeek-tuned tools | 1M context, cache-aware | Edit/apply_patch tools | Exec policy/sandbox | Approval modes | Side-git snapshots | apply_patch | Save/resume/handoff | User memory | V4-aware compaction | Subagents/RLM | Hooks | Built in | Rust TUI | Runtime API only | DeepSeek-focused | Offline eval harness | Low | Skills/MCP/runtime API |
| ClawCode | Clear Rust `ConversationRuntime` | CLI slash workflows | ToolExecutor trait | Recovery recipes | Rust tools crate | Prompt builder/git context | Minimal structured file ops | Rust bash/sandbox | Permission modes | Git context/stale branch | Structured patch fields | Session store | CLAUDE.md context | Compact module | Worker/subagent surfaces | Hooks crate | Runtime MCP modules | CLI REPL | None | Anthropic/OpenAI compat | Mock parity harness | Low | Plugins/commands |
| Anthropic official public | Product-level | Slash workflows/plugins | Claude Code | PR/code review plugins | Public tools via product | CLAUDE.md/memory docs | Product feature | Product feature | Settings examples | GitHub workflows | Product feature | Session history | Memory docs | Product feature | Subagents/plugins | Public hooks | Public MCP | CLI | IDE/GitHub | Claude-focused | Product feedback | Low | Plugins/marketplace |
| DeepSeek docs | Model-level | Strong long reasoning | Agent benchmarks | GRM/evaluator insight | DSML/XML tool schema | 1M context/interleaved thinking | Bash/file-edit eval tools | Bash eval tool | Not app-level | Not app-level | Not app-level | Reasoning persistence | Not app-level | Context modes | Not app-level | Not app-level | Tool/MCP eval | None | None | V4 Pro/Flash modes | Benchmarks | Strong long-doc potential | Model capability |

## Shared Mechanisms

Observed:
- All serious systems move beyond chat by making the model operate inside a constrained loop with typed tools, persistent state, and feedback from the real environment.
- ClaudeCode and OpenCode both enforce a separation between thought/plan, tool calls, tool results, and final response.
- DeepSeek-TUI shows that model-specific protocol details can dominate success rate.
- ClawCode shows a Rust runtime can express these mechanics cleanly and safely.

## Key Design Patterns

### 1. Runtime Loop as Control Plane

Observed:
- ClaudeCode: `src/query.ts`.
- OpenCode: `session/prompt.ts` + `session/processor.ts`.
- DeepSeek-TUI: `core/engine.rs`.
- ClawCode: `runtime/src/conversation.rs`.

Recommendation:
- ResearchCode must have a persisted runtime state machine, not a GUI-only chat state.

### 2. Tool Calls as Typed Transactions

Observed:
- Tools validate input, check permissions, execute, return structured results, and persist metadata.

Recommendation:
- Every tool call should have `pending -> approved/denied -> running -> succeeded/failed -> summarized` lifecycle.

### 3. Context Is Built, Not Dumped

Observed:
- ClaudeCode uses attachments/tool-result storage/compaction.
- OpenCode uses env/system/skills/compaction summaries.
- DeepSeek-TUI orders prompt layers for prefix-cache hits.

Recommendation:
- Context manager must be model-aware and task-aware.

### 4. Edits Are Guarded

Observed:
- ClaudeCode enforces read-before-edit and stale checks.
- OpenCode uses locks, formatter, LSP diagnostics.
- ClawCode provides structured patch outputs.

Recommendation:
- Patch manager should sit between model and filesystem.

### 5. Human-in-the-Loop Is a System Feature

Observed:
- Permissions exist in all mature systems.
- GUI is the natural place to expose approvals, diffs, risky commands, and worktree merges.

Recommendation:
- Approval UI must be part of runtime protocol, not only a modal inside one client.

### 6. Long Tasks Need Memory and Compression

Observed:
- ClaudeCode and OpenCode use structured compaction.
- DeepSeek-TUI adds handoff artifacts and cache-aware compaction.
- ClawCode has session-health probes after compaction.

Recommendation:
- Combine session summaries, artifacts, and durable project memory.

### 7. Model-Specific Compensation

Observed:
- OpenCode repairs tool calls and changes tools by model.
- DeepSeek-TUI adds reasoning replay, DSML parser, prefix-cache stability, JSON arg repair.

Recommendation:
- ModelProfile must include protocol adapters, parser repair, context budget, and failure memory.

## Architecture Decisions

1. Runtime core is state-machine/event-log based.
2. GUI is command center over runtime events.
3. Tool registry is typed, permissioned, model-aware, and deterministic.
4. Patch manager mediates all file writes.
5. Worktree isolation is first-class for multi-agent tasks.
6. Research Worker is peer module, not plugin-only.
7. ModelRouter uses roles and eval feedback, not simple provider selection.
8. Evaluation and observability are required from early phases.

## Open Questions

- Should direct edit/write ever be allowed without diff approval in local trust mode?
- Should team/cloud mode prohibit arbitrary local plugin code by default?
- Which native mode should be default for first internal dogfood: DeepSeek or Qwen3.6-27B? ClaudeCode remains the scaffold reference, not the dogfood provider default.
