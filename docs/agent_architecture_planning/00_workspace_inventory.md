# 00 Workspace Inventory

## Scope

Observed from read-only scans in `/Users/gongyuxuan/Documents/deep-code` using `pwd`, `find`, `rg --files`, and targeted reads of README/config/source files.

## Workspace Overview

Observed projects and documents:

| Priority | Project / Material | Path | Stack | Relationship to ResearchCode Coworker |
|---|---|---|---|---|
| P0 | ClaudeCode-like source | `Open-ClaudeCode-main/Open-ClaudeCode-main` | TypeScript, React Ink/TUI, Node runtime | Main source for Claude Code-style agent loop, tool registry, file edit, permissions, compaction, subagents, hooks, MCP, memory. |
| P0 | OpenCode | `opencode-dev (1)/opencode-dev` | TypeScript/Bun, Effect, AI SDK, Hono/HTTP API, TUI | Open-source runtime reference for session loop, model/provider abstraction, config, plugins, worktree, permission, persistent storage, server API. |
| P0 | DeepSeek-TUI | `DeepSeek-TUI-main/DeepSeek-TUI-main` | Rust, Tokio, Ratatui-style TUI, OpenAI-compatible chat APIs | Main source for DeepSeek-specific optimization: 1M context, thinking-mode replay, prefix-cache strategy, tool-call repair, RLM, compaction/eval. |
| P0 | DeepSeek V4 paper | `DeepSeek_V4.pdf` | PDF research paper | Model-profile source for V4 roles, reasoning efforts, tool schema, interleaved thinking, long-context limits, code/research eval assumptions. |
| P1 | ClawCode Rust rewrite | `claw-code-main (1)/claw-code-main` and `claw-code-main (1)/claw-code-main/rust` | Rust workspace plus Python/reference workspace | Rust runtime architecture reference: crate boundaries, CLI, session loop, permissions, shell sandboxing, file ops, MCP, parity harness. |
| P1 | Anthropic official ClaudeCode public repo/docs/examples | `claude-code-main/claude-code-main` | Docs, plugins, examples, settings, hooks, GitHub workflows | Public product surface reference: settings hierarchy, permissions, plugins, slash commands, hooks, official workflows. |

## Project Details

### Open-ClaudeCode-main/Open-ClaudeCode-main

Observed:
- Entry: `src/main.tsx`.
- Main loop: `src/query.ts`.
- Tool contract: `src/Tool.ts`.
- Tool registry: `src/tools.ts`.
- Core tools: `src/tools/BashTool/BashTool.tsx`, `src/tools/FileReadTool/FileReadTool.ts`, `src/tools/FileEditTool/FileEditTool.ts`, `src/tools/FileWriteTool/FileWriteTool.ts`, `src/tools/AgentTool/AgentTool.tsx`, `src/tools/TodoWriteTool/TodoWriteTool.ts`.
- Tool orchestration: `src/services/tools/toolOrchestration.ts`, `src/services/tools/StreamingToolExecutor.ts`, `src/services/tools/toolExecution.ts`.
- Prompt construction: `src/constants/prompts.ts`.
- Compaction: `src/services/compact/compact.ts`.
- Permissions: `src/utils/permissions/permissions.ts`, `src/hooks/useCanUseTool.tsx`, `src/tools/BashTool/bashPermissions.ts`, `src/tools/BashTool/bashSecurity.ts`.
- Memory/context attachments: `src/memdir/memdir.ts`, `src/utils/attachments.ts`, `src/utils/toolResultStorage.ts`.
- Git/worktree/subagents/MCP: `src/utils/git.ts`, `src/tools/AgentTool/runAgent.ts`, `src/bridge/*`, `src/components/mcp/*`.

Recommendation:
- Treat this as the most important source for the coding-agent runtime design.
- Do not clone UI details directly; extract runtime mechanisms and expose them as GUI events.

### opencode-dev (1)/opencode-dev

Observed:
- Package root: `packages/opencode`.
- Active session loop: `packages/opencode/src/session/prompt.ts`.
- Streaming/persistence processor: `packages/opencode/src/session/processor.ts`.
- LLM/provider path: `packages/opencode/src/session/llm.ts`, `packages/opencode/src/provider/*`.
- Tool framework: `packages/opencode/src/tool/tool.ts`, `packages/opencode/src/tool/registry.ts`, plus tools in `packages/opencode/src/tool/*`.
- Permission: `packages/opencode/src/permission/index.ts`, `packages/opencode/src/permission/evaluate.ts`.
- Agents: `packages/opencode/src/agent/agent.ts`.
- Config: `packages/opencode/src/config/config.ts`.
- Worktree: `packages/opencode/src/worktree/index.ts`.
- Storage schema: `packages/opencode/src/session/session.sql.ts`.
- Server/API: `packages/opencode/src/server/server.ts`.
- TUI: `packages/opencode/src/cli/cmd/tui/*`, one-shot run: `packages/opencode/src/cli/cmd/run.ts`.
- Plugins: `packages/opencode/src/plugin/index.ts`.

Recommendation:
- Use OpenCode to validate a modular runtime with a local HTTP/WebSocket API, persistent event parts, plugin hooks, worktree management, and agent-specific permissions.
- Borrow its separation of `Session`, `Processor`, `LLM`, `Tool`, `Permission`, `Worktree`, and `Server`.

### DeepSeek-TUI-main/DeepSeek-TUI-main

Observed:
- Workspace root: `Cargo.toml`.
- Config: `config.example.toml`.
- README claims DeepSeek V4 focus: `README.md`.
- Protocol/types: `crates/tui/src/models.rs`.
- Chat API adapter: `crates/tui/src/client/chat.rs`.
- Engine: `crates/tui/src/core/engine.rs`.
- Prompt layering: `crates/tui/src/prompts.rs`, prompt files under `crates/tui/src/prompts/*`.
- Tool parsing and repair: `crates/tui/src/core/tool_parser.rs`, `crates/tui/src/tools/arg_repair.rs`.
- Tool registry: `crates/tui/src/tools/registry.rs`.
- Compaction: `crates/tui/src/compaction.rs`.
- Evaluation harness: `crates/tui/src/eval.rs`.
- Other relevant modules: `crates/tui/src/rlm/*`, `crates/tui/src/cycle_manager.rs`, `crates/tui/src/capacity*`, `crates/tui/src/error_taxonomy.rs`, `crates/tui/src/tools/large_output_router.rs`, `crates/tui/src/execpolicy/*`, `crates/tui/src/sandbox/*`, `crates/tui/src/mcp*`, `crates/tui/src/runtime_api.rs`.

Recommendation:
- Treat this as a model-specific runtime lab, not as a complete product architecture.
- Adopt DeepSeek-specific compensation mechanisms only when supported by evals.

### DeepSeek_V4.pdf

Observed after local `pdftotext` extraction:
- V4-Pro: 1.6T parameters, 49B activated; V4-Flash: 284B parameters, 13B activated; both support one million tokens.
- V4 includes Non-think, Think High, Think Max modes.
- Think Max uses explicit system-prompt instruction and longer reasoning budget.
- Paper proposes DSML/XML-style tool-call schema with special token.
- Tool-calling scenarios preserve full reasoning content across rounds and user-message boundaries.
- Agent evals use bash/file-edit tools, up to 500 interaction steps, and 512K max context for code/search agent tasks.
- Paper says V4-Pro is strong but open models still lag behind closed-source models on some code-agent tasks; Flash underperforms Pro for coding agent tasks.

Recommendation:
- Use V4-Pro-Max for deep planning/diagnosis/research synthesis, V4-Flash for cheap exploration/summarization, and keep strong external reviewer models for high-risk edits.

### claw-code-main (1)/claw-code-main

Observed:
- Root docs: `README.md`, `USAGE.md`, `PARITY.md`, `ROADMAP.md`, `PHILOSOPHY.md`.
- Canonical runtime: `rust/`.
- Rust workspace: `rust/Cargo.toml`, crates under `rust/crates/*`.
- Runtime crate exports: `rust/crates/runtime/src/lib.rs`.
- Core loop: `rust/crates/runtime/src/conversation.rs`.
- Permissions: `rust/crates/runtime/src/permissions.rs`, `permission_enforcer.rs`, `policy_engine.rs`.
- Prompt: `rust/crates/runtime/src/prompt.rs`.
- File operations: `rust/crates/runtime/src/file_ops.rs`.
- Shell/sandbox: `rust/crates/runtime/src/bash.rs`, `sandbox.rs`, `bash_validation.rs`.
- API provider layer: `rust/crates/api/src/*`.
- CLI: `rust/crates/rusty-claude-cli/src/main.rs`.
- Tools crate: `rust/crates/tools/src/lib.rs`.
- Plugins: `rust/crates/plugins/src/*`.
- Mock parity harness: `rust/crates/mock-anthropic-service/*`, `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`.

Recommendation:
- Use this as evidence that a Rust core can represent agent runtime cleanly.
- Avoid its current default `danger-full-access`; our product should default to workspace-write with explicit network/destructive approvals.

### claude-code-main/claude-code-main

Observed:
- Official public README: `README.md`.
- Settings examples: `examples/settings/README.md`, `settings-strict.json`, `settings-bash-sandbox.json`, `settings-lax.json`.
- Hook example: `examples/hooks/bash_command_validator_example.py`.
- Plugin catalog: `plugins/README.md`.
- GitHub automation workflows: `.github/workflows/*`.
- Plugin examples include specialized agents, commands, hooks, MCP, output styles, PR review, feature development.

Recommendation:
- Use as product-facing requirements for settings, managed policy, plugins, hooks, marketplace, GitHub/CI workflows, and organization deployment.

## Recommended Analysis Priority

1. ClaudeCode loop/tools/permissions/context.
2. DeepSeek-TUI + DeepSeek V4 model-specific behavior.
3. OpenCode runtime abstractions, storage, API, worktree.
4. ClawCode Rust module boundaries and safety surfaces.
5. Official ClaudeCode public docs/plugins for product UX.

## Inventory Gaps

Open Questions:
- Is `Open-ClaudeCode-main` intended as the canonical ClaudeCode source for this workspace, or only a reverse-engineered reference?
- Should the product reuse any code from these repositories, or only architecture patterns?
- Should DeepSeek V4 be assumed available through `api.deepseek.com` in development, or should local/open checkpoints be the default target?

