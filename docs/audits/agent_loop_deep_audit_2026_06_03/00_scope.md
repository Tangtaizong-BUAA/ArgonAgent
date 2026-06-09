# 00_scope.md — Agent Loop Deep Audit Scope & Boundaries

**Date:** 2026-06-03
**Branch:** `spike/next-step`
**Trigger:** Full architecture audit of the agent loop / runtime / GUI pipeline
**Audit type:** Read-only architecture review — no code changes in audit phases

## 1. Audit Objective

Conduct a comprehensive, evidence-based architecture review of the deep-code agent
loop and runtime pipeline, covering:

- How the agent loop starts, continues, stops, resumes, and fails
- The identity chain of tool calls, tool results, provider tool IDs, and permission IDs
- The recovery chain for plan approval, file write approval, and shell approval
- The boundary between DeepSeek/Qwen native profiles and compatible providers
- Context, compaction, duplicate observation, and long-task progression
- The GUI event reducer, transcript, streaming markdown, and intermediate narrative display
- The local API, active turn, cancel, and next-question flow
- Real GUI stress test and deterministic harness adequacy
- Design differences from ClaudeCode / OpenClaudeCode / DeepSeek-TUI
- Whether doc39 design goals have been violated

## 2. What IS in Scope

| Area | Scope |
|---|---|
| Agent loop main cycle (start/continue/stop/resume/fail) | Full |
| Tool call / tool result / provider tool ID / permission ID identity chain | Full |
| Plan approval / file write approval / shell approval recovery chain | Full |
| DeepSeek / Qwen native profile vs compatible provider boundary | Full |
| Context / compaction / duplicate observation / long-task progression | Full |
| GUI event reducer / transcript / streaming markdown / narrative display | Full |
| Local API / active turn / cancel / next question | Full |
| Real GUI stress test and deterministic harness | Full |
| ClaudeCode / OpenClaudeCode / DeepSeek-TUI design comparison | Reference only |
| doc39 design goal drift detection | Full |
| Security / shell classifier | Full |
| Event identity and replayability | Full |

## 3. What is NOT in Scope

| Area | Reason |
|---|---|
| UI polish / CSS / layout aesthetics | Not architecture-critical |
| Rewriting architecture | Audit phase is read-only |
| `final_answer` / `loop_budget` as new features | Explicitly rejected by doc39 |
| ClaudeCode feature parity as a goal | ClaudeCode is reference only, not target |
| Compatible provider production hardening | Native profiles are primary |
| Subagent implementation details | Only loop/runtime/GUI pipeline |
| SQLite persistence adapter | Not in loop path |
| Research Worker internals | Separate subsystem |

## 4. Constraint Documents (Authoritative)

These documents define the architecture target and cannot be overridden by audit findings:

1. `AGENTS.md` — project direction, model scope, native optimization preservation
2. `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md` (doc39) — target architecture
3. `docs/implementation/implementation_status.md` — current implementation status
4. `docs/runtime/p3_p4_completion_status_2026_05_19.md` — P3/P4 completion evidence
5. `docs/implementation/agent_kernel_tool_contract_long_task_todos.md` — doc39 execution contract
6. `docs/decisions/` — D1-D4 architecture decisions

## 5. Key Architectural Invariants (from doc39, non-negotiable)

1. AgentKernel is the sole turn orchestrator
2. RuntimeFacade is the external public API but does NOT own loop policy
3. NativeProfile is the entry point for DeepSeek/Qwen native behavior
4. No scattered `if family == DeepSeek` inside AgentKernel
5. ToolManifest stays complete; PermissionPolicy decides execution
6. TCML is the sole mediation path for all model tool calls
7. StreamProcessor is the DeepSeek-native heart
8. ReasoningReplayManager, CachePrefixPolicy, AliasRegistry, RepairCatalog, RoleSplit are first-class kernel primitives
9. GUI/TUI/local API consume structured events, not raw text
10. DeepSeek/Qwen native promotion must pass eval gates

## 6. Explicitly Rejected Directions (must flag if found in code)

These were considered and rejected in doc39. If they appear in the runtime, they are drifts:

- `final_answer` as a special tool or message type
- `disable_tools_and_request_final_answer` pattern
- `agent.loop_budget_reached` as a completion trigger
- `model_continuation_skipped` pattern
- `visible_finalizer_answer` pattern
- `loop_budget` as a hard tool cap
- Tool disabling to force model completion
- Prompt-keyword-based tool exposure control

## 7. Audit Phases

| Phase | Name | Action |
|---|---|---|
| 0 | Scope & directory setup | THIS DOCUMENT |
| 1 | Parallel code reconnaissance | 16 explorer agents, read-only |
| 2 | External reference comparison | 4 comparator agents |
| 3 | Evidence merge & conflict detection | Issue matrix + conflict matrix |
| 4 | Architecture layer review | Layer-by-layer diagram & analysis |
| 5 | Test system review | Deterministic / canary / endurance |
| 6 | Remediation grading | A/B/C classification |
| 7 | Red team review | 3 reviewer agents |
| 8 | Final audit package | Report, matrices, diagrams |

## 8. Output Files

All output goes under `docs/audits/agent_loop_deep_audit_2026_06_03/`:

```
00_scope.md                          ← THIS FILE
01_architecture_map.md               ← Phase 4 architecture diagram
02_agent_reports/                    ← Phase 1 agent reports (16 files)
03_cross_reference/                  ← Phase 2 comparator reports (4 files)
04_issue_matrix.md                   ← Phase 3 merged issue matrix
05_conflict_matrix.md                ← Phase 3 conflict matrix
06_test_gap_matrix.md                ← Phase 5 test gap analysis
07_architecture_recommendations.md   ← Phase 6 graded recommendations
08_final_report.md                   ← Phase 8 final report
architecture_graph.mmd               ← Phase 4 Mermaid diagram
event_identity_flow.mmd              ← Phase 4 event identity flow
permission_resume_flow.mmd           ← Phase 4 permission resume flow
task_progression_state_machine.mmd   ← Phase 4 task progression FSM
```

## 9. Agent Report Format

Each Phase 1 agent must produce a report with:

- **Conclusion:** One-paragraph summary
- **Severity:** P0 (crash/data-loss) / P1 (broken feature) / P2 (degraded) / P3 (cosmetic)
- **Files involved:** Absolute paths
- **Events involved:** Event type names
- **State involved:** State machine states
- **Reproduction clues:** How to trigger
- **Evidence fragments:** Code snippets or event log excerpts
- **Root cause:** Which layer is responsible
- **Hidden risks:** What could go wrong that isn't obvious
- **doc39 conflict:** Yes/No with citation
- **Suggested fix:** What to change (advisory only, no code edits in audit)
- **Not suggested:** What NOT to do
- **Handoff needed:** Which other agent should follow up

## 10. Severity Definitions

| Level | Definition |
|---|---|
| P0 | Data loss, security bypass, unrecoverable crash, event log corruption |
| P1 | Broken feature path, permission bypass, model cannot continue, GUI shows wrong state |
| P2 | Degraded UX, missing telemetry, inefficient context use, missing test coverage |
| P3 | Cosmetic, naming, documentation, non-critical edge case |

## 11. Rules of Engagement

1. **No code changes during audit phases.** All findings are advisory.
2. **Evidence required.** Every issue must cite a file path, line range, or event log excerpt.
3. **doc39 is the architecture target.** Drift from doc39 is a finding, not a matter of opinion.
4. **ClaudeCode is reference only.** We do not copy ClaudeCode; we learn from its discipline.
5. **Native profiles first.** DeepSeek/Qwen behavior is the primary concern.
6. **Compatible providers are secondary.** They must not contaminate native profiles.
