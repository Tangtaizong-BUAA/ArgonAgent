# ClaudeCode/OpenCode Gap Check

This document makes the alignment table executable. The current product is not
claiming full ClaudeCode parity; it tracks which capabilities are implemented,
partial, or deliberately gated.

Run:

```bash
python3 scripts/claudecode_gap_check.py
```

Status definitions:

- `implemented`: required kernel/TUI/runtime marker exists and is covered by smoke tests.
- `partial`: useful foundation exists, but ClaudeCode/OpenCode depth is not complete.
- `gated`: the capability is registered or planned but disabled by default until security/eval gates pass.
- `missing`: no acceptable implementation marker exists.

Release-blocking rules:

- Implemented kernel capabilities must not regress to missing.
- MCP, worktree, subagent, web, browser, and plugin capabilities must stay gated until their threat controls and evals pass.
- DeepSeek/Qwen native tool-result continuation must stay implemented; simulating tool results as ordinary user text is not acceptable.
- Native streaming tool calls must preserve tool-use/tool-result boundaries: safe completed tool inputs may execute before the full assistant message finishes, but control/permission tools must still block through the normal approval path.
- DeepSeek native live-loop requests must stay below the 256K hard cap; the runtime preflight target is 240K and compaction telemetry starts at 192K.
- Context compaction is implemented only when live-loop preflight events and structured summaries are both present; offline summary generation alone is not enough.
- File writes must preserve read-before-write and rollback invariants.
- Shell execution must keep audit reasons and hard-deny controls.
- Allow-session and allow-project permission decisions must remain persisted as
  security policy rules, not only transient UI choices.
- `AGENTS.md` / `RESEARCHCODE.md` project instructions must remain first-class
  context items for prompt assembly and compaction.
