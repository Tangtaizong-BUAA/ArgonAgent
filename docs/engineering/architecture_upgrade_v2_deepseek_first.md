# Architecture Upgrade V2: DeepSeek-First Agent Kernel

This is the main-workspace entry point for the V2 DeepSeek-first architecture
upgrade.

The original source draft reviewed for this plan was found at:

- `.claude/worktrees/gracious-dubinsky-f08f39/docs/engineering/architecture_upgrade_v2_deepseek_first.md`

The executable implementation plan lives at:

- `docs/implementation/architecture_upgrade_v2_deepseek_first_todos.md`

## Architecture Position

V1 focuses on Claude-Code-grade runtime discipline:

- stable tool identity;
- permission resume;
- bounded blocking;
- replayable events;
- GUI-visible approval and error states.

V2 keeps those guarantees and moves the product beyond a ClaudeCode clone:

- provider-agnostic kernel message model;
- permission-safe hook system;
- durable transcript and fork model;
- subagent isolation;
- DeepSeek/Qwen native model capability matrix;
- DeepSeek reasoning budget and replay policy;
- DSML/native tool-call policy;
- DeepSeek protocol fallback;
- DeepSeek error classification and retry;
- automatic context cache planning;
- GUI/TUI lifecycle observability.

The main decision is:

```text
Copy ClaudeCode's runtime discipline, not its provider assumptions.
DeepSeek/Qwen native behavior must be encoded in the agent kernel and runtime
policy, not floating above the kernel as ordinary provider configuration.
```

## Non-Negotiable Constraints

- DeepSeek/Qwen remain native optimized models.
- Claude, OpenAI, GLM, local models, and arbitrary compatible APIs remain
  compatible-only unless explicitly promoted through native rules and evals.
- Compatible providers cannot override native DeepSeek/Qwen prompts, parsers,
  context policy, tool policy, or eval gates.
- DeepSeek reasoning raw volatile data must not persist across sessions.
- DSML fallback is a measured DeepSeek-native recovery path, not a hidden parser
  accident.
- Hooks cannot bypass PermissionGate.
- Async dependency migration requires the existing async ADR to be reopened and
  explicitly approved.

## Execution Plan

Use the TODO document as the implementation contract:

- `docs/implementation/architecture_upgrade_v2_deepseek_first_todos.md`

That file contains:

- execution contract;
- coverage matrix against the V2 source architecture;
- dependency and stop conditions;
- phase-by-phase implementation tasks;
- focused tests;
- acceptance criteria;
- final verification matrix;
- progress ledger.
