# Architecture Upgrade From Claude Code: Review

Source reviewed:

- `.claude/worktrees/gracious-dubinsky-f08f39/docs/engineering/architecture_upgrade_from_claude_code.md`

This review updates that document against the current main workspace. The source
document is useful because it names the right class of failures: approval
continuation, blocking execution, tool identity, streaming tool input, and
session/event durability. It is not safe to execute as-is because several claims
are stale or pointed at the wrong desktop/runtime boundary.

## Review Verdict

Use the document as a problem map, not as the implementation contract.

The correct target is still Claude-Code-grade runtime discipline, but the
execution plan must be revised around the current codebase:

- root `desktop/` is the real Tauri GUI, not `apps/desktop/`;
- `desktop/src-tauri/src/main.rs` exists and already exposes runtime commands;
- shell timeout exists in `crates/runtime/src/command.rs`;
- native loop pending approval state is now partially persisted in
  `RuntimeFacade`;
- streaming tool-call accumulation exists internally, but is not fully surfaced
  as GUI/event lifecycle;
- the runtime still has major architectural gaps around provider tool-call ids,
  blocking session locks, native approval resume semantics, and broad timeout
  coverage.

## Findings

### P0: Desktop Boundary Is Wrong In The Source Document

The source document scopes the GUI to `apps/desktop` and says the Tauri bridge is
missing. In current main, the real GUI is root `desktop/`, with:

- `desktop/src/components/AppShell.tsx`
- `desktop/src/runtime/localRuntimeClient.ts`
- `desktop/src-tauri/src/main.rs`

`apps/desktop` is not the product GUI path. Any TODO against `apps/desktop` would
miss the user-visible application.

### P0: Approval Bug Diagnosis Is Partly Stale

The source document correctly identifies the historic failure mode:

```text
submit_permission_decision only records permission.decided and does not resume the tool.
```

Current main has already moved past that exact state:

- `RuntimeFacade` stores `pending_native_tool`;
- `submit_permission_decision` can execute the pending native tool;
- Tauri has `runtime_submit_permission_decision`;
- the frontend now exposes in-flight/error state for approval clicks.

Remaining gap: approval is still not a true native-loop external-decision resume.
It executes the pending tool and then uses the existing continuation path, which
can still be less precise than resuming the same native loop frame.

### P0: Full Tokio Migration Is Not A Safe First Step

The source document makes `tokio + parking_lot + async loop` Phase 0. That is
too broad for the immediate bug class and violates the current long-task
discipline unless it is explicitly approved as a dependency and architecture
change.

The practical first step is smaller and testable:

- split blocking tool execution out of session locks;
- add missing timeout guards around every blocking boundary;
- preserve pending approval state and resume semantics;
- only then decide whether the runtime needs a full async migration ADR.

Async migration may still be valuable, but it should not be treated as the only
route to unblock approval and tool hangs.

### P0: Tool Call Identity Gap Is Still Real

This is one of the strongest findings in the source document.

Current `ParsedToolCall` and `MediatedToolCall` do not carry the provider's
original tool-use id. Streaming assembly internally sees `provider_tool_use_id`,
but the id is not preserved through the canonical parsed/mediated structures.

This can break multi-tool and resumed-tool pairing. It must be fixed before
claiming Claude Code-level tool runtime reliability.

### P0: Blocking Boundaries Are Partly Fixed, Not Fully Fixed

Shell command timeout now exists, but the architecture still has blocking risk:

- `RuntimeFacade.sessions` is still one `std::sync::Mutex<HashMap<...>>`;
- `execute_session_tool` can execute tools while holding the session record lock;
- sidecar/provider process boundaries need explicit timeout and cancellation
  review;
- GUI approval and model continuation still depend on spawned blocking work.

The correct TODO is not "no timeout exists"; it is "timeout and lock boundaries
must be audited and enforced everywhere."

### P1: Streaming Tool Lifecycle Exists But Is Not Product-Grade

The runtime has `StreamingToolCallAssembler`, and some native loop tests cover
streaming tool inputs. But the top-level stream event handler still drops
`ToolCallStarted`, `ToolCallArgumentsDelta`, and `ToolCallFinished` for the
generic GUI event surface.

The missing work is event and GUI lifecycle fidelity:

- `tool.input_started`;
- `tool.input_delta`;
- `tool.input_finalized`;
- provider id preservation;
- no duplicate execution;
- clear permission behavior for streaming control tools.

### P1: Compaction Claim Needs Nuance

The source document says compaction is not connected to the loop. Current main
has native preflight compaction telemetry and budget guard events. The gap is
not "no compaction"; it is:

- compaction has to be proven as a real continuation mechanism under long
  sessions;
- DeepSeek reasoning replay must survive compaction boundaries;
- GUI/event replay must show compaction as a first-class lifecycle event.

### P2: Hooks, Durable Session Rotation, And Subagent Isolation Are Valid But Not P0

These are real maturity gaps compared with Claude Code/OpenCode-style systems.
They should remain in the backlog, but they do not block the immediate approval
and tool-hang failures.

## Architecture Decision

The implementation contract should be:

```text
Do not copy Claude Code's protocol.
Do copy its runtime discipline:
  - every blocking boundary has timeout/cancel;
  - every external wait has a wake/resume path;
  - every tool use has stable identity;
  - every transition is evented and replayable;
  - GUI/TUI are clients of RuntimeFacade events.

Keep DeepSeek/Qwen native primitives inside the kernel:
  - reasoning replay;
  - DSML/content fallback;
  - alias/repair catalog;
  - provider capability matrix;
  - context budget and compaction policy;
  - role split and temperature policy.
```

## Required TODO Document

The executable TODO plan derived from this review lives at:

- `docs/implementation/architecture_upgrade_from_claude_code_todos.md`
