# 28 Native Optimization Consolidation

本文件解决的问题：整合而不是重写已有 DeepSeek/Qwen native optimization。它把已有设计提升为不可丢失的不变量，并标出需要 eval 验证、存在重复、冲突和缺失的地方。

修正旧文档的方式：`03`、`10`、`15`、`20` 已经包含 DeepSeek/Qwen 深设计，本文件不替代它们，而是定义后续文档和实现不得违反的 consolidation contract。

## 1. Source Index

| Area | Authoritative existing docs |
|---|---|
| DeepSeek-TUI and DeepSeek docs analysis | `03_deepseek_tui_and_docs_analysis.md` |
| Model optimization architecture | `10_model_optimization_architecture.md` |
| DeepSeek/Qwen native modes | `15_native_deepseek_qwen_modes.md` |
| Eval gates | `20_eval_suite_v0.md` |
| ADR provider scope | `18_architecture_decision_records.md`, superseded by `33_updated_adr_bundle.md` after this pass |

## 2. Existing DeepSeek Optimization to Preserve

**Decision:** DeepSeek native optimization is already defined and must be preserved.

**DeepSeek invariants:**

1. `reasoning_content` replay is supported where provider requires it.
2. Reasoning sanitizer prevents malformed/missing reasoning history failures.
3. Native tool calls are preferred over text-simulated tools.
4. Tool results must not be simulated as ordinary user messages in thinking mode.
5. DSML/XML fallback parser exists for text-only/legacy outputs.
6. JSON argument repair is deterministic, logged, and eval-counted.
7. Stable system prompt prefix is preserved.
8. Tool catalog is sorted/memoized for byte stability.
9. Prefix-cache-aware prompt ordering is used.
10. Cache hit/miss telemetry is logged where provider exposes it.
11. Reasoning replay token telemetry is logged.
12. Compaction is late and V4-aware.
13. Auto-compaction floor is high; small-history compaction is avoided.
14. Strict tool mode is tied to provider capability.
15. Pro/Flash/Non-think role split is preserved.
16. DeepSeek parser/eval gates block promotion on wrong-tool execution.
17. Profile updates are eval-driven.

**Rationale:** These are the actual mechanisms that make DeepSeek-TUI more than a base_url switch.

## 3. Existing Qwen Optimization to Preserve

**Decision:** Qwen native optimization targets Qwen3.6-27B only.

**Qwen invariants:**

1. Qwen3.6-27B is the canonical Qwen target.
2. Qwen2/Qwen2-7B must not drive production profile behavior.
3. 262K native context is the default context assumption.
4. Extended context is only a verified deployment capability flag.
5. Qwen-specific chat template is required for native tool use.
6. Qwen-specific reasoning parser is required where thinking is exposed.
7. `qwen3_coder` tool-call parser is used where deployment supports it.
8. Generic OpenAI transport does not equal Qwen-native tool use.
9. Thinking mode is used for planning, diagnosis, review, and research synthesis.
10. Non-thinking mode is limited to low-risk/simple tasks.
11. Preserve-thinking is capability-gated and token/cost logged.
12. Precise coding sampling is used for code edits.
13. Edits are patch-sized and structured.
14. Stale-file detection remains mandatory.
15. Tests and reviewer loop remain mandatory for edits.
16. Qwen parser eval gates block promotion on wrong-tool execution.
17. Qwen executor eval gates block unsupervised coding promotion.
18. Qwen long-context eval is required before extended-context claims.
19. Same-family fallback is preferred; cross-family fallback needs user approval.
20. Deployment capability checks are recorded in model-call logs.

## 4. Conclusions to Keep

| Conclusion | Keep? | Why |
|---|---|---|
| DeepSeek V4 needs cache-aware context policy | Yes | Supported by DeepSeek-TUI and V4 analysis. |
| DeepSeek reasoning replay can be useful but expensive | Yes | Needs telemetry and role gating. |
| Qwen3.6-27B needs parser/template-aware native mode | Yes | Generic transport is insufficient. |
| Qwen extended context is not default | Yes | Must be deployment capability flag. |
| ClaudeCode scaffold pattern should be absorbed | Yes | It shows how to adapt an agent runtime to a model family. |
| Compatible providers are not native | Yes | Prevents scope creep and false quality promises. |

## 5. Conclusions to Adjust

| Existing pattern | Adjustment |
|---|---|
| Broad "multi-model router" phrasing | Replace with DeepSeek/Qwen native router plus CompatibleProvider layer. |
| Tauri/Rust stack as final implementation choice | Reframe as prototype choice pending Phase 0 spikes. |
| Plan approval represented as PermissionRequest in GUI flow | Split into PlanApprovalRequest/Decision. |
| Next task suggesting monorepo scaffold | Replace with Phase 0 go/no-go and spikes. |

## 6. Conclusions Needing Eval

| Topic | Eval gate |
|---|---|
| DeepSeek XML/DSML fallback | DS-01/DS-02: zero wrong-tool executions. |
| DeepSeek prefix-cache context strategy | DS-03/DS-05: lower churn/cost without lower success. |
| DeepSeek reasoning replay | DS-04: no secret persistence; pass continuation tasks. |
| Qwen parser/template | Qwen parser fixtures: zero wrong-tool executions. |
| Qwen executor role | Qwen executor promotion: coding/patch/shell pass threshold with zero security failures. |
| Qwen extended context | Long-context eval with verified deployment flags. |
| Compatible provider fallback | Baseline eval only; never native promotion. |

## 7. Duplicates

| Duplicate | Consolidation |
|---|---|
| DeepSeek role split appears in `03`, `10`, `15` | Treat `15` as mode contract, `10` as router role mapping, `03` as source analysis. |
| Qwen parser rules appear in `10`, `15`, `20` | Treat `15` as native invariant, `20` as eval gate. |
| Provider scope appears in `18` and `21` | Use `27` and `33` as latest scope contract. |

## 8. Conflicts

| Conflict | Fix |
|---|---|
| Executive summary still recommends monorepo scaffold as next task | Update `14_executive_summary.md` to recommend Phase 0 convergence/spikes first. |
| GUI flow uses PermissionRequest for plan approval | Update `23_gui_user_flows.md` and `21_product_kernel_v0.md`. |
| ADR-001/006 too accepted for stack/API | Add provisional ADR bundle in `33_updated_adr_bundle.md`. |
| Some wording suggests broad model optimization | Use `27_model_scope_and_provider_layer.md` as authoritative scope. |

## 9. Missing Pieces

1. ProviderConfig/ModelAliasMapping schema and validation spike.
2. PlanApprovalRequest/Decision schema.
3. TaskContract schema and violation handling.
4. Multi-agent policy.
5. Threat control matrix mapped to tasks/tests.
6. Phase 0 execution order.
7. AGENTS.md draft reflecting native model scope.

## 10. DeepSeek Eval Gates

| Gate | Pass requirement | Blocks |
|---|---|---|
| DS parser gate | zero wrong-tool executions in DS parser fixtures | DeepSeek parser promotion |
| DS argument repair gate | low-confidence repair never executes | Strict tool promotion |
| DS reasoning gate | no secret in reasoning persistence | Reasoning replay default |
| DS cache gate | stable prefix improves or preserves success/cost | Cache-aware context promotion |
| DS role gate | Pro/Flash/Non-think role split beats generic baseline | Role router promotion |

## 11. Qwen Eval Gates

| Gate | Pass requirement | Blocks |
|---|---|---|
| Qwen parser gate | zero wrong-tool executions | Qwen native tool mode |
| Qwen executor gate | pass coding/patch/shell thresholds with no security failures | Qwen executor promotion |
| Qwen thinking gate | thinking improves planning/review without unacceptable cost | thinking default for roles |
| Qwen long-context gate | verified 262K/extended behavior | long-context claims |
| Qwen deployment gate | parser/template/capability checks pass | native Qwen session start |

## 12. Promotion Rules

**Rule:** A DeepSeek/Qwen optimization can be promoted only if:

- it names the changed profile/rule;
- it identifies affected eval cases;
- it improves or preserves security-critical metrics;
- it records rollback condition;
- it updates failure memory if needed.

### 12.1 DeepSeek 256K Runtime Safety Cap

- Reason for change: local engineering validation showed DeepSeek becomes unreliable above 256K effective context even when public/model-facing material advertises larger windows.
- Affected profile/rule: `DeepSeekFull` remains the native DeepSeek scaffold, but runtime request budgeting clamps DeepSeek to a 256K hard cap, a 240K preflight target, a 192K live-compaction threshold, and a 12K reasoning replay budget.
- Affected eval case: scaffold/context-budget evals, native loop v2 continuation fixtures, DeepSeek reasoning replay fixtures, and long tool-loop fixtures must assert the 256K cap and preflight events.
- Promotion condition: DeepSeek native loop tests pass with `model.context_budget`, `context.compaction.*`, reasoning replay, compact retry, and long-loop evidence under the cap.
- Rollback condition: if provider/API behavior is later verified to be stable above 256K in real native loop evals, the cap can only be raised by adding a new deployment-specific profile and preserving the 256K safe default.

## 13. Fallback Rules

| Case | Rule |
|---|---|
| DeepSeek Pro unavailable | Prefer DeepSeek same-family lower role profile; user approval for cross-family fallback. |
| Qwen parser unavailable | Qwen native tool mode disabled; compatible/manual mode allowed with warning. |
| Qwen3.6 endpoint actually serves Qwen2 | Native session blocked. |
| Compatible provider requested as fallback | User must explicitly approve; log as compatible fallback, not native. |

## 14. Human Escalation Rules

Escalate to human when:

- parser repair confidence is low;
- tool call maps to a different tool than requested;
- reasoning sanitizer strips sensitive content;
- model asks to cross family;
- native capability check fails;
- DeepSeek/Qwen eval gate has no passing evidence for requested role;
- any action would violate TaskContract.

## Execution Impact

- Do not rewrite DeepSeek/Qwen profiles as generic provider configs.
- Add consolidation check to Phase 0.
- Require native optimizer changes to cite eval cases.

## Next Tasks

1. Create Qwen3.6-27B evidence refresh and parser fixtures.
2. Create DeepSeek parser golden fixtures.
3. Add native capability check schema for both model families.

## Open Questions

- Which DeepSeek API variants expose reliable cache hit/miss telemetry?
- Which Qwen3.6 deployment stack will be primary for first native Qwen eval: vLLM, SGLang, DashScope, or local custom?
