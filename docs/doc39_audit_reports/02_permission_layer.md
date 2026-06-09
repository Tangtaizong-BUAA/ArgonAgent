# Audit 02: Permission Layer vs doc39

**Date:** 2026-05-19 | **Files:** permission_policy.rs, permission_gate.rs, tool_argument_policy.rs, permission_resolver.rs

## Verdict: Three Systems Unified into One Pipeline

The old 3 parallel permission systems have been unified. `NativeAgentPermissionMode` no longer exists in the main branch.

---

## 1. Permission Type Inventory

### ACTIVE — doc39-conformant
- **PermissionMode** (5 variants): Default, Plan, AcceptEdits, DontAsk, BypassPermissions — `agent_kernel/permission_policy.rs:4`
- **PermissionGate** — checkpoint struct wrapping PermissionResolver — `agent_kernel/permission_gate.rs:14`
- **PermissionPolicy::evaluate()** — 5-mode fallback logic — matches doc39 §3.4 exactly
- **PermissionResolver** — 8-step pipeline engine — `permission_resolver.rs:195`
- **PermissionRequest** — doc39-shaped request struct — `permission_resolver.rs:166`
- **PermissionCheck trait** — 6 implementations (ShellCommand, FileWrite, FileRead, FileEdit, PatchApply, Default)
- **PermissionRuleStore** — file-backed rule persistence

### DEAD CODE
- **ToolArgumentReplayMode** enum — zero production callers
- **replay_mode_for_tool()** — zero production callers
- **safe_side_effect_argument_summary_json()** — zero production callers
- **redacted_argument_keys()** + all private helpers — ~128 lines total dead

## 2. Permission Decision Call Graph

```
NativeAgentLoopRequest { permission_mode: PermissionMode }
  └── KernelServices.permission_gate: PermissionGate

=== Tool Permission Decision ===
tool_permission_decision()                          (execution.rs:429)
  └── permission_gate.evaluate(request, tool)      (execution.rs:451)
        └── PermissionResolver::evaluate_request()  (resolver.rs:256)
              ├── [Step 1a] Deny rules match → Deny
              ├── [Step 1b] Ask rules match → Ask
              ├── [Step 1c] tool.check_permissions() → ToolPermissionResult
              ├── [Step 1d] Result::Deny → Deny
              ├── [Step 1e] requires_user_interaction + Ask → Ask
              ├── [Step 1f] Result::Ask → Ask
              ├── [Step 1g] SafetyCheck → Ask(force:true)
              ├── [Step 2a] BypassPermissions → Allow
              ├── [Step 2b] Allow rules match → Allow
              └── [Step 3] resolve_by_mode()
                    └── PermissionPolicy::evaluate()  ← doc39 Step 7 equivalent
                          ├── BypassPermissions → Allow
                          ├── Plan + state_changing → Deny
                          ├── AcceptEdits + file_edit → Allow
                          ├── AcceptEdits + state_changing → Ask
                          ├── DontAsk + read_only → Allow
                          ├── DontAsk → Deny
                          ├── Default + state_changing → Ask
                          └── else → Allow
```

## 3. Three Entry Points (all unified)

1. **Streaming tool path**: `handle_native_stream_tool_event()` → `tool_permission_decision()`
2. **Batch tool path**: `tool_permission_decision()` same flow
3. **Fast Auto Write path**: `fast_auto_write_permission_gate_result()` — BypassPermissions → Allow

## 4. Gaps vs doc39 §3.4 & §5 Step 7

| Gap | Severity | Detail |
|---|---|---|
| Permission not standalone lifecycle phase | Medium | Not an explicit Step 7 with PermissionEvaluated events |
| ToolArgumentPolicy totally dead | Low | 128 lines of unused argument redaction code |
| Denial fallback inert | Low | `should_fallback()` never called in production |
| Mode fallback is Step 3 not Step 7 | Low | Architectural ordering differs from spec but functionally correct |

## 5. Overall Assessment

**Permission unification: COMPLETE.** Single PermissionMode enum, single PermissionGate checkpoint, single 8-step resolver pipeline. No residual NativeAgentPermissionMode. The mode-based fallback logic is correct. The primary gap is that permission isn't an independently-visible lifecycle phase with structured events.
