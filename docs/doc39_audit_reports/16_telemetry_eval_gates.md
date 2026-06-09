# Audit 16: Phase 8 Telemetry + Eval Gates R1-R10 (Re-audit)

**Date:** 2026-05-19 | **Re-audit reason:** 纳入 P2-C retry/recovery 事件

## 1. doc39 §19 Metric Coverage (Re-calculated)

**2/17 COVERED, 4/17 PARTIAL, 11/17 MISSING. Coverage: 11.8%** (unchanged from old audit — metrics not implemented since then)

| # | Metric | Status | Field |
|---|---|---|---|
| 1 | cache.zone_a_hit_rate | PARTIAL | merged hits/misses |
| 2 | cache.zone_b_hit_rate | PARTIAL | merged hits/misses |
| 3 | reasoning.tokens_per_turn | PARTIAL | total only |
| 4 | reasoning.replay_count | MISSING | — |
| 5 | reasoning.replay_size_kb | MISSING | — |
| 6 | dsml.leak_chunks_count | PARTIAL | event count only |
| 7 | dsml.leak_recovered | MISSING | — |
| 8 | tool_call.partial_chunks_avg | MISSING | — |
| 9 | tool_call.assembly_latency_ms | MISSING | — |
| 10 | alias.resolution_count_by_alias | **COVERED** | alias_resolutions |
| 11 | repair.rule_applied_count_by_rule | **COVERED** | repair_applications |
| 12 | repair.success_rate | MISSING | — |
| 13 | compaction.triggers_count | PARTIAL | compaction_count |
| 14 | compaction.tokens_freed | MISSING | — |
| 15 | role_split.executor_calls | MISSING | — |
| 16 | role_split.compactor_calls | PARTIAL | compactor_role_calls |
| 17 | role_split.flash_savings_estimate_usd | MISSING | — |

## 2. P2-C Recovery Events (NOT tracked in telemetry)

7 new recovery event types + 13 `agent.loop_recovery` emission sites. None have corresponding fields in `AgentKernelTelemetry`.

## 3. Eval Gates R1-R10

### R1: reasoning_content replay in tool_use chains
**PASS** — ReasoningReplayManager: capture() (native_agent_loop.rs:1283), inject(), compact_old_reasoning() (lines 580, 1048). Wired in main loop.

### R2: DSML cross-chunk filtering
**PASS** — DsmlChunkFilter (`stream.rs:42-110`) stateful cross-chunk filter. 3 marker types: `<tool_calls>`, `<tool_call>`, `<|tool_calls_section_begin|>`. Integrated in StreamProcessor (`stream_processor.rs:55`).

### R3: tool_calls.delta cross-chunk accumulation
**PASS** — StreamingToolCallAssembler (`stream.rs`) with per-index accumulation. `json_object_complete()` check before assembly. Split-marker test passes.

### R4: ContentToolCallExtractor doesn't auto-execute
**FAIL** — `content_extractor.rs` is 3-line wrapper. No `ExtractedContentCall` struct. No `scan()` with confidence. No ContentToolCallCandidate emission at finish_reason=stop.

### R5: AliasRegistry covers 50+ aliases
**PASS** — 116 explicit aliases in `alias_registry.rs`. Case-insensitive. Snake/dot/kebab interconversion. No edit-distance fuzzy suggest (minor).

### R6: RepairCatalog doesn't repair file.write.content
**PARTIAL (70%)** — `never_apply_to` covers file.write.content + shell.command.command (16 tuples). But only 2/5 repair rules implemented.

### R7: CachePrefixPolicy sort stability
**PASS** — Zone A sorted by canonical_tool_id (lexical). Zone B sorted by key. Tests pass.

### R8: RoleSplit uses Flash for Compactor
**FAIL** — RoleSplit defined but never used in production. No Flash model routing exists.

### R9: 192K triggers compaction
**PARTIAL (80%)** — Threshold defined (192K). Guard checks correctly. Compactor.compact() called. But compact() is read-only (doesn't modify EventLog).

### R10: base_hash injected by runtime
**FAIL (30%)** — patch.apply has injection. TCML has no base_hash injection for file.write/edit. Tests confirm base_hash remains None.

## 4. Summary

| Gate | Status |
|---|---|
| R1 | PASS |
| R2 | PASS |
| R3 | PASS |
| R4 | FAIL |
| R5 | PASS |
| R6 | PARTIAL (70%) |
| R7 | PASS |
| R8 | FAIL |
| R9 | PARTIAL (80%) |
| R10 | FAIL (30%) |

**Pass rate: 5/10 pass, 2/10 partial, 3/10 fail.**

## 5. Phase 8 Telemetry + ToolDoctor: ~15%
- Telemetry: 100% dead code (aggregate_from never called in production)
- ToolDoctor: manifest consistency only (missing cache-status, alias-stats, repair-stats)
- P2-C recovery events: zero telemetry tracking
