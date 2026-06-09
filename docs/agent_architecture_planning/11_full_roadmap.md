# 11 Full Roadmap

## Phase 0: Architecture & Repo Setup

Goal:
- Establish monorepo, architecture docs, runtime/API contracts, dev tooling.

Tasks:
- Create Rust workspace, React/Tauri app, Python sidecar skeleton, SQLite migrations, shared schemas.
- Create reference-use and clean-room policy for ClaudeCode/OpenCode/ClawCode/DeepSeek-TUI analysis: what can be copied, what must be reimplemented, how source citations are kept out of production code.

Deliverables:
- Compiling empty app and CLI, migration runner, architecture docs.
- `docs/engineering/reference_use_policy.md` with license inventory and implementation boundaries.

Dependencies:
- None.

Tests:
- `cargo test --workspace`, frontend typecheck, Python smoke test.
- Policy check in PR template: every imported pattern must cite architecture docs, not copy incompatible source.

Risks:
- Cross-language boundaries unclear.
- License/clean-room ambiguity contaminates implementation work.

Acceptance:
- GUI launches and can ping runtime; CLI prints version; DB initializes.
- Reference-use policy exists before production runtime code begins.

## Phase 1: Core Agent Runtime

Goal:
- Implement session state machine and event log.

Tasks:
- `AgentSession`, `AgentEvent`, `Task`, state transitions, cancellation, event persistence.

Deliverables:
- Runtime can create session, run fake model fixture, emit events.

Dependencies:
- Phase 0.

Tests:
- State-machine unit tests, event replay tests.

Risks:
- State model too vague.

Acceptance:
- Deterministic fixture session reaches Completed/Failed.

## Phase 2: CLI/TUI Coding Loop

Goal:
- Terminal entry for coding tasks using same runtime.

Tasks:
- `researchcode run`, `resume`, `sessions`, JSON event output.

Deliverables:
- One-shot prompt through mock model and local tools.

Tests:
- CLI snapshot tests.

Risks:
- CLI diverges from GUI runtime.

Acceptance:
- CLI consumes runtime API only.

## Phase 3: Desktop GUI Shell

Goal:
- Command-center shell.

Tasks:
- Project list, task board, session timeline, event stream.

Deliverables:
- Tauri app with React routes and live runtime connection.

Tests:
- Playwright UI smoke, Rust IPC tests.

Risks:
- UI before runtime semantics stabilizes.

Acceptance:
- User can create project/task/session and watch mock events.

## Phase 4: Diff / Permission / Patch System

Goal:
- Safe file modification.

Tasks:
- PatchProposal, diff preview, permission requests, apply/revert, read-before-write checks, stale checks.

Deliverables:
- GUI diff approval and CLI approval.

Tests:
- Patch apply/reject/conflict tests.

Risks:
- Edge cases in line endings/encodings.

Acceptance:
- No file writes without policy approval path.

## Phase 5: Model Router & Model Profiles

Goal:
- Native DeepSeek/Qwen3.6-27B mode routing with ClaudeCode-style scaffold adaptation.

Tasks:
- ModelProvider trait, DeepSeek and Qwen3.6-27B profiles, selected-family role routing, same-family fallback, cost/latency logging.
- ClaudeCode adaptation metadata: capability gates, stable tool schemas, thinking policies, prompt/template hashes, provider/deployment IDs.

Deliverables:
- DeepSeek V4 profile bundle.
- Qwen3.6-27B profile bundle.
- Reference-only ClaudeCode/OpenCode/Codex scaffold notes, non-routable by default.

Tests:
- Routing decision tests, mocked provider tests.

Risks:
- Provider-specific behavior leaks into runtime instead of staying inside DeepSeek/Qwen mode adapters.

Acceptance:
- One task can use different planner/executor/reviewer profiles inside the selected native family.
- Cross-family fallback is blocked unless explicitly enabled.

## Phase 6: DeepSeek/Qwen Specific Optimization

Goal:
- Model-specific compensation.

Tasks:
- DeepSeek reasoning replay policy, stable tool catalog, cache metrics, DSML fallback parser.
- Qwen3.6-27B reasoning parser policy, `qwen3_coder` tool parser adapter, preserve-thinking policy, 262K/128K context policy, sampling profiles, Qwen-Agent vs direct-parser eval.
- Native mode logging: adapter version, prompt hash, tool schema hash, parser flags, thinking settings, deployment stack, context length.

Deliverables:
- DeepSeek profile evals.
- Qwen3.6-27B profile evals for thinking/non-thinking, precise coding, parser fixtures, long-context behavior, and patch success.

Tests:
- Tool-call parser fixtures, cache/context tests, Qwen parser deployment smoke, same-family fallback tests.

Risks:
- Optimizations become folklore without metrics.

Acceptance:
- A/B eval shows measurable improvement or feature remains disabled.
- Qwen3.6-specific features are disabled unless parser/deployment capability checks pass.

## Phase 7: Research Worker

Goal:
- Python sidecar for data workflows.

Tasks:
- File index, schema profiler, quality checks, script runner, artifact capture.

Deliverables:
- Research jobs for CSV/Excel/JSON/Parquet.

Tests:
- Fixture data profiles and script execution.

Risks:
- Python dependency management.

Acceptance:
- GUI can profile a dataset and produce a chart/report artifact.

## Phase 8: Eval Harness

Goal:
- Measure agent/model/tool quality.

Tasks:
- Coding fixtures, data-analysis fixtures, model-specific tool-call eval, metrics dashboard.

Deliverables:
- Local eval runner and dashboard.

Tests:
- Eval runner self-tests.

Risks:
- Eval cases not representative.

Acceptance:
- Every profile has pass/fail/cost/tool-error metrics.

## Phase 9: Multi-Agent / Worktree

Goal:
- Parallel isolated tasks.

Tasks:
- Worktree manager, per-agent branches, merge/conflict UI, subagent lanes.

Deliverables:
- Two agents can edit separate worktrees and merge.

Tests:
- Worktree create/reset/merge tests.

Risks:
- Git conflicts and user changes.

Acceptance:
- User can inspect and merge/reject each agent's changes.

## Phase 10: Research Workspace

Goal:
- Research project UI and workflows.

Tasks:
- Data catalog, experiment index, report viewer, notebook export, literature parser.

Deliverables:
- Research workspace tab.

Tests:
- End-to-end data-to-report scenario.

Risks:
- Scope creep.

Acceptance:
- A research folder can be indexed and summarized into README/metadata/report.

## Phase 11: Skill / Automation

Goal:
- Reusable workflows and hooks.

Tasks:
- Skill manifest, command workflows, hook engine, scheduled tasks.

Deliverables:
- Local skill catalog and hook approval UI.

Tests:
- Hook sandbox and workflow tests.

Risks:
- Arbitrary code execution.

Acceptance:
- Skills declare permissions and can be disabled by policy.

## Phase 12: Alpha Product

Goal:
- Cohesive local product.

Tasks:
- Polish UX, packaging, onboarding, privacy settings, crash/error reporting, docs.

Deliverables:
- Installable alpha for internal users.

Tests:
- Cross-platform smoke, dogfood tasks.

Risks:
- Integration reliability.

Acceptance:
- Internal users can use it for real coding and research tasks.

## Phase 13: Team/Cloud Extension

Goal:
- Collaboration and managed policy.

Tasks:
- Accounts, project sharing, remote approvals, cloud metadata sync, team policies, GitHub/CI integration.

Deliverables:
- Team beta.

Tests:
- Auth, sync, permissions, audit.

Risks:
- Privacy and compliance.

Acceptance:
- Team can collaborate without exposing local files beyond policy.
