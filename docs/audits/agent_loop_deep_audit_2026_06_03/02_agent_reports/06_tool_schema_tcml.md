# Agent 6: Tool Schema / TCML Audit

## Conclusion

TCML mediation is correctly structured for the main (non-concurrent) execution path with proper alias → schema → repair → permission ordering. However, the **concurrent read-only execution path bypasses TCML entirely**, losing relational defaults, quoted integer repair, markdown link repair, and argument alias normalization. The `file.write.content` and `shell.command.command` fields are correctly blocked from repair (doc39 compliant). Schema errors are returned as recoverable observations.

**Severity:** P1 (concurrent path TCML bypass causes silent parameter loss)

## Files Involved

- `crates/runtime/src/tcml/contract.rs` — main mediation pipeline (238-490)
- `crates/runtime/src/tcml/repair_catalog.rs` — never-repair field lists (15-35)
- `crates/runtime/src/tcml/alias_registry.rs` — tool name resolution
- `crates/runtime/src/tcml/issue_guided_repairer.rs` — repair application
- `crates/runtime/src/tcml/relational_resolver.rs` — offset/limit defaults
- `crates/runtime/src/tcml/manifest.rs` — manifest building (66-97, 141-155)
- `crates/runtime/src/native_agent_loop.rs` — lines 1719-1789 (concurrent bypass), 1891-1949 (shell recovery)
- `crates/runtime/src/native_agent_loop_tools.rs` — lines 882-909, 1105-1196
- `crates/runtime/src/tool_execution.rs` — execution pipeline
- `crates/kernel/src/tool.rs` — ToolSpec definitions

## Key Findings

### Finding 1: Concurrent Path Bypasses TCML (P1)

In `native_agent_loop.rs:1719-1789`, concurrent read-only tool execution constructs `ToolExecutionArgs` using raw `parse_tool_arguments` and `normalize_tool_id`, **bypassing the full `mediate_tool_call` pipeline**. This means:
- No relational defaults (offset=0 when limit given, etc.)
- No quoted integer repair (string "2000" → integer 2000)
- No markdown link repair
- No argument alias normalization (root → path, etc.)
- No TCML events recorded
- `ToolExecutionArgs` only copies 6 fields, dropping `offset`, `limit`, `max_bytes`, etc.

### Finding 2: Orchestration Uses Raw Tool IDs (P2)

The concurrent path uses `pt.tool_id.clone()` (raw model-provided name) in `OrchestrationToolCall`, not the normalized canonical ID. Alias tools like "ls" or "read" would fail lookup in `execute_tool`.

### Finding 3: Stage Order Cosmetic Mismatch (P3)

The event declares stages as `["parse","alias","repair","schema_validate","manifest"]` but actual execution order is `alias → manifest → parse → repair → schema_validate`. Cosmetic only — code order is correct.

### Finding 4: Repair Safety Compliant (P0 ✓)

- `file.write.content`: NEVER repaired (confirmed by `is_never_repair_field` and test)
- `shell.command.command`: NEVER repaired (confirmed)
- `file.write.path`, `file.write.base_hash`: NEVER repaired
- `file.edit.old_string`, `file.edit.new_string`: NEVER repaired

### Finding 5: Schema Errors Are Recoverable (P0 ✓)

When schema validation fails, the function returns `Rejected` with `ModelReadableToolError` where `retryable: true`. Converted to tool result artifact, model sees error and can retry. No loop state pollution.

## doc39 Conflict

**Yes for concurrent bypass** (§5, §7): TCML must be the sole mediation path for ALL tool calls. The concurrent path creates a second, incomplete mediation path. **No** for repair safety (§9): content/command fields correctly blocked.

## Suggested Fix

Replace the ad-hoc `parse_tool_arguments` + manual `ToolExecutionArgs` construction in the concurrent path with a full `mediate_tool_call` call.

## Handoff Needed

- Agent 1 (Agent Loop) — concurrent path is in main loop body
