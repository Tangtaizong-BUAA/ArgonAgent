# Audit 12: Permission Cross-Cutting Wiring

**Date:** 2026-05-19 | **Scope:** All permission types and decision paths

## Verdict: Unified Pipeline, 100% Converged

The three old parallel permission systems are fully unified. `NativeAgentPermissionMode` is **completely removed** (0 references).

## 1. Complete Decision Flow

```
=== PATH A: Native Agent Loop V2 ===

tool_permission_decision()                    [execution.rs:429]
  └── permission_gate.evaluate(request, tool) [execution.rs:451]
        └── PermissionResolver::evaluate_request()
              ├── [1a] Deny rules → Deny
              ├── [1b] Ask rules → Ask(force)
              ├── [1c] tool.check_permissions() → ToolPermissionResult
              ├── [1d] Result::Deny → Deny
              ├── [1e] requires_user_interaction + Ask → Ask
              ├── [1f] Result::Ask → Ask
              ├── [1g] SafetyCheck → Ask(force, safety)
              ├── [2a] BypassPermissions → Allow
              ├── [2b] Allow rules → Allow
              └── [3] resolve_by_mode() → PermissionPolicy::evaluate()

=== PATH B: RuntimeFacade ===
apply_permission_policy() → evaluate_permission_request() → same pipeline

=== PATH C: FastAutoWrite ===
fast_auto_write_permission_gate_result() → BypassPermissions → Allow
```

## 2. Permission Type Inventory

### ACTIVE — Unified
- **PermissionMode** (5 variants): agent_kernel/permission_policy.rs — ~40 production usages
- **PermissionGate**: agent_kernel/permission_gate.rs — ~25 usages, all tool paths
- **PermissionResolver**: permission_resolver.rs — 8-step pipeline
- **PermissionRuleStore**: permission_policy.rs — file-backed TSV persistence
- **PermissionCheck trait**: 6 implementations (ShellCommand, FileWrite, FileRead, FileEdit, PatchApply, Default)

### REMOVED
- **NativeAgentPermissionMode**: 0 references — fully deleted

### ACTIVE — Backward compat
- `type PermissionPolicy = PermissionRuleSet` (alias, line 244)
- `type PermissionPolicyStore = PermissionRuleStore` (alias, line 245)

## 3. Dead Code

- **tool_argument_policy.rs**: `ToolArgumentReplayMode`, `replay_mode_for_tool()`, `safe_side_effect_argument_summary_json()`, all private helpers — **128 lines, all dead**

## 4. Manifest Keyword Cutting — STILL ACTIVE

`NativeAgentToolExposure` (3 variants: ReadOnly, FastAutoWrite, CodeEdit) still controls tool manifest visibility. Phase 2 goal ("delete manifest turn-state cutting") **not implemented**.

## 5. PermissionGate Checkpoint Position

**Correct.** All tool execution paths pass through `permission_gate.evaluate()` before execution. Checkpoint order: PreToolUse hook → PermissionGate::evaluate() → execute_tool().

## 6. Migration Progress Summary

| Metric | Status |
|---|---|
| NativeAgentPermissionMode removal | 100% |
| PermissionPolicy::evaluate() as fallback | 100% integrated (Step 3) |
| PermissionGate at execution checkpoint | 100% |
| 8-step unified pipeline | 100% |
| File-backed rule persistence | 100% integrated (Steps 1a,1b,2b) |
| Tool-specific checks | 100% integrated (Steps 1c-1g) |
| Manifest cutting removal | 0% (Phase 2 not done) |

**Overall: Permission system conceptually converged.** Remaining work is Phase 2 manifest cutting removal.
