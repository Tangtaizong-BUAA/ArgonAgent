# Phase 8: Final Audit Package

## Executive Summary

A comprehensive 8-phase audit of the deep-code agent loop/runtime pipeline was conducted on 2026-06-03. The audit deployed **26 agents** (16 reconnaissance + 7 comparators + 3 red team reviewers), identified **85+ issues** (19 P0 including red team discoveries), and assessed **68 doc39 compliance points**.

**Overall Assessment:** The codebase has strong fundamentals (correct tool identity chain, all rejected patterns removed, solid deterministic test coverage) but has critical gaps: (1) AgentKernel is architecturally a hollow pass-through, not an orchestrator, (2) the convergence system cannot self-terminate for non-error plateaus, (3) the compatible Anthropic provider path is broken, (4) `sh`/`bash` interpreters pass through all security layers unblocked, (5) tool manifest filtering violates doc39, and (6) a dual security architecture gives different guarantees on the facade vs. native loop paths. **15 quick-win fixes are available (~5 engineer-days).**

## Audit Scope

- **Source files audited:** 50+ Rust source files, 10+ TypeScript/JavaScript files, 40 runtime session event logs
- **Phases executed:** 8 (Reconnaissance → Comparators → Issue Matrix → Architecture Review → Test Review → Remediation → Red Team → Final Package)
- **Agents deployed:** 16 reconnaissance + 7 comparators + 3 red team reviewers = 26 total
- **Reference documents:** doc39, AGENTS.md, implementation_status.md, 5 decision documents
- **Reference codebase:** OpenClaudeCode TypeScript (read-only comparison)

## Deliverables Index

| # | Document | Description |
|---|---|---|
| 1 | `00_scope.md` | Audit scope, boundaries, rules of engagement |
| 2 | `02_agent_reports/01-16` | 16 Phase 1 reconnaissance agent reports |
| 3 | `02_agent_reports/17-23` | 7 Phase 2 comparator reports (OpenClaudeCode line-by-line & architecture comparisons) |
| 4 | `03_issue_matrix.md` | 60 issues sorted by severity (P0→P3) + 12 positive findings |
| 5 | `04_doc39_conflict_matrix.md` | 68 doc39 sections cross-referenced against findings |
| 6 | `04_architecture_review.md` | Layer-by-layer architecture review with 8 Mermaid diagrams |
| 7 | `05_test_system_review.md` | Test coverage matrix, 7 critical gaps, recommended additions |
| 8 | `06_remediation_grading.md` | Grade A/B/C classification with effort estimates |
| 9 | `05_red_team/` | 3 independent red team reviews (security, architecture, robustness) |
| 10 | `08_final_audit_package.md` | This document — executive summary and recommendations |
| 11 | `09_corrected_architecture_remediation_plan.md` | Post-audit planning correction: corrected severity model, implementation phases, and validation gates |

## Post-Audit Planning Correction

The execution order in this final package is superseded by
`09_corrected_architecture_remediation_plan.md`. The audit findings remain the
evidence base, but the corrected plan reorders the work around event truth,
permission/cancel generation, convergence authority, stable manifest/TCML, and
AgentKernel ownership migration. This avoids reintroducing the rejected
finalizer/tool-hiding/loop-cap patch pattern while preserving doc39's
DeepSeek/Qwen-native direction.

## Key Findings by Priority

### P0 — 12 Issues (Must Fix Before Production)

1. **Compatible Anthropic path broken** — Sends flat strings instead of structured content blocks; any tool call gets HTTP 400
2. **shell.command hidden from manifest** — ReadOnly exposure hides shell from model; direct doc39 §2.3 violation
3. **file.write/edit/patch hidden from manifest** — Same mechanism; direct doc39 violation
4. **Runtime UTF-8 panic** — `is_char_boundary` assertion crashes sessions on multi-byte characters
5. **HTTP 400 infinite retry** — DeepSeek API blocks cause sessions to loop forever without backoff
6. **Model validation error loops** — Model repeats same `file.edit` error 8+ times without adaptation
7. **Same-tool-error plateau** — Takes 70 iterations to detect and stop error loop
8. **No Flash model for compaction** — In-process only; doc39 §12 requires separate Flash model
9. **No L1 state object** — CompactionSummary is an unstructured markdown blob
10. **Irreversible compaction** — Model cannot see pre-compaction events
11. **Unknown tool hallucination** — Model calls non-existent tools (`create_file`, `memory_get_all`)
12. **DSML leak fallback** — Visible finalizer fails with http_status_400/empty_visible_response

### Quick Wins — 15 Fixes in ~5 Days

| # | Fix | Grade | Effort |
|---|---|---|---|
| 1 | Include all tools in manifest regardless of exposure | A | XS |
| 2 | Fix UTF-8 string boundary with `floor_char_boundary()` | A | XS |
| 3 | Reduce plateau threshold from ~70 to ~10 | A | XS |
| 4 | Map `SafetyCheck` to `PermissionResolution::Deny` | A | S |
| 5 | Clear `pending_native_decision` in `cancel_session` | B | XS |
| 6 | Add `permission_id`, `plan_approval_id` to merge rewrite keys | B | XS |
| 7 | Tag synthetic `provider_tool_call_id` values | B | XS |
| 8 | Remove non-standard `reasoning_content` from Anthropic builder | B | XS |
| 9 | Replace substring `is_error` check with JSON field parse | B | XS |
| 10 | Use canonical tool ID in concurrent execution path | B | S |
| 11 | Reset `suppressNextCallCompletedSettleRef` on session boundaries | B | XS |
| 12 | Fix TCML stage order in event declaration | C | XS |
| 13 | Suppress `below_threshold` telemetry for small requests | C | XS |
| 14 | Add `rmdir` to program deny list | C | XS |
| 15 | Add `sudo` to Layer A hard block | C | XS |

### doc39 Compliance Summary

| Status | Count | % |
|---|---|---|
| ✅ Compliant | 24 | 32% |
| ⚠️ Partial | 16 | 21% |
| ❌ Conflict | 25 | 33% |
| 🔴 Not Implemented | 3 | 4% |

**All doc39 rejected patterns REMOVED from production:** `final_answer`, `disable_tools`, `disable_tools_and_request_final_answer`, `model_continuation_skipped`, `visible_finalizer_answer`.

### Architecture Health by Layer

| Layer | Grade | Critical Issues |
|---|---|---|
| L0: System Topology | B | Dual desktop/local-API turn management |
| L1: Agent Loop | C | 6 loop owners, 23 exit paths, manifest changes mid-loop |
| L2: Event Identity | B | String-based ID mapping, merge omissions |
| L3: Provider Projection | D | Broken compatible path, dirty fallback events |
| L4: TCML Pipeline | C | Concurrent path bypasses 5 of 7 stages |
| L5: Permission System | B | Classifier Deny ≠ true deny, missing programs |
| L6: Compaction | D | No Flash model, no L1 state, irreversible |
| L7: Cancel | C | TOCTOU race, 250ms polling gap |
| L8: GUI Events | B | Narrative loss, cross-session leaks, unbounded memory |

## Red Team Findings

3 independent reviewers challenged the Phase 1-2 findings. Key outcomes:

### New P0 Issues Found

| # | Source | Issue | Impact |
|---|---|---|---|
| RT-P0-1 | Architecture | **AgentKernel is a hollow pass-through** — `run_turn()` at `kernel.rs:203-237` does nothing with its 10 service fields; delegates entirely to monolithic loop. §1 compliance foundation is invalid. | Invalidates §1.1-§1.8 compliance |
| RT-P0-2 | Architecture | **`visible_text_looks_like_transition_statement` IS the rejected `visible_finalizer_answer` pattern** — same string-matching loop-control logic with negated condition. Should be CONFLICT, not COMPLIANT. | doc39 drift not actually clean |
| RT-P0-3 | Robustness | **Convergence never self-terminates** (was P2-20, should be P0) — `ConvergenceEnforcer` produces only SoftWarning/EscalateToCodeEdit, never Stop. Only 6 identical tool errors can terminate. Explains 70-iteration plateaus. | All non-error plateaus are infinite |
| RT-P0-4 | Robustness | **`stream_event_handler` emits dirty events BEFORE HTTP status confirmed** — structural tool events from failed Anthropic attempt leak to turn controller. Root cause of "duplicate tool_call_id in ledger" bug. | Dual-protocol fallback corrupts tool state |

### Severity Re-Ratings

| Issue | Original | Correct | Rationale |
|---|---|---|---|
| P2-20 Convergence not self-terminating | P2 | **P0** | Only mechanism producing Stop is 6 identical errors |
| Agent 05 json_object_complete accepts {} | P2 | **P1** | Budget consumption + model confusion |
| P2-13 Narrative text swallowing | P2 | **P1** | Corrupts conversation context for continuation/compaction |
| §12.1 No Flash model (in-process compaction) | P0 | **P2** | In-process satisfies semantic intent; Flash is cost optimization |
| §1.1 AgentKernel compliance | Compliant | **Conflict** | Kernel is pass-through, not orchestrator |
| §15 visible_finalizer_answer status | Compliant | **Conflict** | Pattern reincarnated as `visible_text_looks_like_transition_statement` |
| §10.1/§10.2 Permission resume | Compliant | **Partial** | P1-04 + P1-05 together invalidate full compliance |

### New Edge Cases Found

| # | Severity | Issue |
|---|---|---|
| EC-1 | P0 | stream_event_handler fires before HTTP status confirmed — dirty events leak |
| EC-2 | P1 | merge_events side-effect applier bypasses `can_transition` validation |
| EC-3 | P1 | Sidecar interrupt kills child and discards already-parsed valid tool calls |
| EC-4 | P1 | Evidence clearing (`last_tool_batch.clear()` + `evidence_ledger.clear()`) destroys evidence before convergence analysis |
| EC-5 | P1 | ObservationCache (never cleared) vs EvidenceLedger (cleared per iteration) produce mixed global/local counts that distort plateau thresholds |
| EC-6 | P2 | `ToolProgressState.reset_repeated_error_streak` resets on ANY successful tool call — model can interleave cheap success with repeated failure to defeat the only Stop mechanism |
| EC-7 | P1 | Complete inversion of OpenClaudeCode reference: services on AgentKernel struct but monolithic loop accesses them directly, bypassing kernel mediation |

### Classification Challenges

- **§12.1 Flash model:** Red team recommends reclassifying from P0 Conflict to P2 Partial — in-process compaction satisfies the semantic intent of model-driven context management; separate Flash is a cost optimization, not a correctness requirement.
- **EscalateToCodeEdit:** More destructive than reported — no model notification of manifest change, silent route/tool schema mismatch, no de-escalation path, permanently neutered convergence after 2 escalations.
- **TurnBudget §7.6:** Technically compliant (event-only, no tool disabling) but retains hard-cap struct design with sentinel values (0=unlimited) rather than a true progress-based architecture.

### Security Red Team

| # | Severity | Issue |
|---|---|---|
| RED-01 | **P0** | `sh`/`bash`/`zsh` interpreters pass through all three defense layers unblocked — not in any deny list |
| RED-02 | P1 | Unicode whitespace bypass: DENY_SUBSTRINGS uses ASCII spaces but `tokenize_command` uses Unicode-aware `char.is_whitespace()` |
| RED-03 | P1 | Native agent loop path (`execute_tool_with_permission_gate`) completely skips Layer A (`command_contains_hard_deny`) — dual security architecture with different guarantees |
| RED-04 | P2 | `is_sensitive_path` (file reads) covers far fewer patterns than `check_dangerous_path` (file writes) |
| RED-05 | P2 | No rate limiting on permission submissions — malicious client can flood and exploit race windows |
| RED-06 | P2 | `artifact.export` has no security classifier or secret scan |

**Severity upgrades:** P2-17 (sudo not in Layer A) → P1; P2-19 (missing dangerous programs) → P1.

**No false positives found** — all Issue Matrix findings are factually accurate.

## Recommended Action Plan

### Week 1: Critical Fixes (Grade A Quick Wins + Top P0s)
1. Fix tool manifest filtering (A2, A3)
2. Fix UTF-8 string boundary panic (A5)
3. Reduce plateau threshold (A6)
4. Fix classifier Deny mapping (A11)
5. Fix compatible Anthropic path (A1)
6. Add HTTP 400 exponential backoff (A7)

### Week 2-3: Architectural Fixes
7. Add active_turns generation counter (A12)
8. Add max-consecutive-error limit (A4)
9. Remove TurnRoute→exposure mapping (A15)
10. Route concurrent path through TCML (A16)

### Month 2: Design Decisions
11. Decide: Flash model for compaction or document in-process as intentional
12. Implement L1 state object or document markdown blob as sufficient
13. Consolidate loop ownership (B1)

### Backlog: Grade B & C
14-36. Remaining Grade B and C issues prioritized by user impact

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| DeepSeek API instability causes mass session failure | Medium | High | Exponential backoff, circuit breaker |
| Compatible provider path broken for all non-native models | High | High | Fix before any compatible provider launch |
| UTF-8 panic in production | Low | High | Fix string boundary check |
| Shell command executed with inadequate permission check | Low | Critical | Fix classifier Deny, add sudo/rmdir to deny lists |
| TCML bypass allows unmediated tool execution | Medium | Medium | Route concurrent path through full TCML |
| Active turn corruption on rapid stop/start | Medium | Medium | Add generation counter |

---

*Audit conducted 2026-06-03. All findings based on code at commit f5c859ad and runtime session data from May 5 - June 2, 2026.*
