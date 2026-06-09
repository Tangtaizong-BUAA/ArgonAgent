# doc39 Audit Report Status

Last updated: 2026-05-20

This directory contains audit snapshots against the doc39 architecture plan. Some
reports are still accurate, while others include line references or conclusions
that became stale after the P2-B/P2-C and P3/P4 repair passes. Treat every report
as an audit record, not as automatically current truth. Re-check current code
with `rg` before using a finding as implementation evidence.

## Status Labels

- `valid`: primary finding still matches current code.
- `mostly valid`: main architectural concern still stands, but line references
  or secondary details may be stale.
- `partial`: primary finding was partly fixed; remaining gaps still matter.
- `fixed/stale`: original primary finding no longer matches current code.
- `needs recheck`: related code changed enough that the report should be
  re-audited before planning implementation work from it.

## Current Index

| Report | Current status | Notes |
|---|---|---|
| `01_kernel_core.md` | mostly valid | Kernel service graph now exists and P3 authority routing improved; older line-level claims need recheck. |
| `02_permission_layer.md` | mostly valid | Permission unification improved, but lifecycle/event coverage and policy-surface gaps still need review. |
| `03_compactor.md` | valid | EventLog physical compaction remains open; request projection compaction is not persisted log truncation. |
| `04_conversation_history.md` | partial | Original "history never injected" finding is stale after P2-B; event coverage and first-class provider message replay remain open. |
| `05_turn_control.md` | mostly valid | Evidence/convergence ownership improved through AgentKernel, but loop behavior should be rechecked with current runtime traces. |
| `06_provider_telemetry.md` | needs recheck | P2-C added transient retry/recovery events; aggregation and eval-gate claims need fresh code evidence. |
| `07_deepseek_stream_reasoning.md` | needs recheck | StreamProcessor work changed the implementation surface; verify against current DeepSeek streaming code. |
| `08_deepseek_cache_role_think.md` | mostly valid | Cache prefix support exists, but role-split/Flash/reasoning policy integration remains incomplete. |
| `09_qwen_profile_factory.md` | mostly valid | Qwen native profile remains thinner than DeepSeek; current factory behavior should be rechecked before action. |
| `10_tcml_full.md` | needs recheck | TCML service ownership changed during P3; do a fresh grep audit before using old call-site counts. |
| `11_monomer_analysis.md` | mostly valid | Native loop is still large, though authority boundaries moved into AgentKernel services. |
| `12_permission_crosscutting.md` | mostly valid | Cross-cutting permission concerns remain relevant; direct native-loop call-site counts may be stale. |
| `13_context_compaction_integration.md` | partial | ConversationHistory injection is fixed; EventLog compaction, Flash compactor, and cache-zone integration remain open. |
| `14_profile_rolesplit_integration.md` | valid | RoleSplit is still not a production-owned routing policy. |
| `15_phase6_7_toolresult_subagent.md` | mostly valid | Tool-result projection improved; subagent execution/merge/isolation coverage remains open. |
| `16_telemetry_eval_gates.md` | needs recheck | Retry/recovery telemetry changed; eval-gate and metric completeness claims need re-audit. |

## Current Runtime Status Reference

Use `docs/runtime/p3_p4_completion_status_2026_05_19.md` as the latest
implementation status summary for P2-B, P2-C, P3, and P4 partial work. The audit
reports here remain useful for gap discovery, but this status file is the better
source for what was intentionally changed in the latest repair pass.
