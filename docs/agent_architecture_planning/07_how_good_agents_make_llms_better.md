# 07 How Good Agents Make LLMs Better

## Core Answer

Good coding agents make LLMs better by replacing free-form guessing with a closed-loop engineering process:

1. constrain the model with a precise role, tool schema, and permission policy;
2. give it searchable, current, local context;
3. force it to observe before editing;
4. let it act through typed tools;
5. return real tool results/errors;
6. require diffs/tests/reviews;
7. persist state across long work;
8. compress history without losing decisions;
9. route tasks to suitable models;
10. keep humans in the loop for irreversible actions.

## Mechanism Breakdown

### Context Organization

Observed:
- ClaudeCode uses prompt layers, attachments, read-file state, memory, and tool-result storage.
- OpenCode builds system messages from env, instructions, skills, agents, and compaction summaries.
- DeepSeek-TUI orders prompt layers to maximize stable prefix cache.

Recommendation:
- Build `ContextBundle` from structured slots: task, plan, repo map, file snippets, tool results, memory, artifacts, model notes.

### Repo Map and File Retrieval

Observed:
- ClaudeCode and DeepSeek-TUI inject shallow project maps when no explicit context file exists.
- Mature agents prefer `Glob/Grep/Read` over blind shell.

Recommendation:
- Repo map should be refreshed cheaply, then targeted retrieval should use `rg`, AST parse, LSP symbols, git diff, recent files, and user-selected files.

### Task Planning

Observed:
- ClaudeCode uses TodoWrite and plan mode.
- OpenCode has explicit plan/build/explore agents.
- DeepSeek-TUI has plan/agent/yolo modes and cycle manager.

Recommendation:
- Separate plan representation from conversation text. GUI should show current plan as editable structured data.

### Tool Calling

Observed:
- Tool schemas constrain behavior and force interaction with real environment.
- OpenCode and DeepSeek-TUI repair tool-call errors.

Recommendation:
- Tool calls should be auditable, replayable, and linked to model messages and permissions.

### Tool Result Feedback

Observed:
- Agents improve by seeing actual build/test errors, file contents, diffs, and diagnostics.

Recommendation:
- Tool results need compression policies, but raw outputs must remain available as artifacts.

### Diff/Patch

Observed:
- ClaudeCode and OpenCode convert edits into structured diffs.
- ClawCode returns structured patch hunks.

Recommendation:
- Diffs are the main trust interface for GUI. They must be grouped by task/agent/worktree and linked to rationale.

### Shell Execution

Observed:
- Shell gives real feedback but is high risk.
- Mature systems parse/classify/sandbox commands and require approval for dangerous actions.

Recommendation:
- Shell tool must default to limited workspace-write, with separate network and destructive approvals.

### Git State

Observed:
- Agents use git status/diff/root/worktree to understand current changes.

Recommendation:
- GUI should continuously show branch/base/dirty files and isolate multi-agent changes in worktrees.

### Permissions

Observed:
- Permissions reduce blast radius and force users to approve irreversible actions.

Recommendation:
- Permission policy should be project-configurable and visible as a security model, not hidden prompts.

### Error Recovery

Observed:
- Agents repair after real errors: patch fails, test fails, permission denied, prompt too long, tool parse failure.

Recommendation:
- Add explicit recovery policies by error class: syntax/test/build/lint/tool/permission/context/provider.

### Long Task Memory and Compression

Observed:
- Long tasks require summaries, handoffs, memories, and preserved artifacts.

Recommendation:
- Compression should produce structured summaries with goal, constraints, decisions, progress, files, open failures, next action.

### Prompt Hierarchy

Observed:
- Systems separate system/developer/tool/user/memory/project instructions.

Recommendation:
- Never let project memory override safety/system policy. Use clear precedence and conflict handling.

### Reviewer Loop

Observed:
- Verification nudges and specialized review agents reduce false confidence.

Recommendation:
- Reviewer must inspect diff plus tests/logs, not only final code text.

### Subagents and Multi-Agent

Observed:
- ClaudeCode uses AgentTool and worktree isolation.
- OpenCode has subagent/task tool and agents.

Recommendation:
- Subagents should have scoped write sets, tool permissions, and merge gates.

### Hooks and Automation

Observed:
- Hooks are policy, workflow, and quality gates.

Recommendation:
- Hooks should be typed, inspectable, and disable-able per project policy.

### GUI Taskization

Observed:
- CLI/TUI hides much of runtime state in text.

Recommendation:
- GUI should taskize: plan board, session lanes, diff review, approval queues, artifact viewers, eval dashboards.

### Eval-Driven Optimization

Observed:
- DeepSeek-TUI includes offline tool-loop eval; OpenCode has tests/snapshots; ClawCode has mock parity harness.

Recommendation:
- Product quality must be measured by build/test pass rate, patch success, hallucinated file references, approvals, cost/time, tool-call errors, and human intervention.

## Final Principle

The agent's job is not to make the model "smarter" in the abstract. It makes the model more reliable by turning every uncertain claim into an observable action, every risky action into an approval, every edit into a diff, every failure into feedback, and every long session into structured state.

