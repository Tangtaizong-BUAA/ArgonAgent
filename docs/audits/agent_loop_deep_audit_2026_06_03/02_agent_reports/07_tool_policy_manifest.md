# Agent 7: Tool Policy / Manifest Audit

## Conclusion

The tool manifest system has multiple doc39 violations. Tools are hidden based on exposure mode (ReadOnly hides shell.command, file.write, file.edit, patch.apply). TurnRoute classification uses prompt keywords to determine exposure, making the manifest unstable across turns. `model_compatibility` field is defined but never enforced. Qwen's `max_active_tools` limit silently truncates the tool catalog. The `tui_fastauto` path maintains a separate, inconsistent tool filtering logic.

**Severity:** P0 (shell.command not in manifest under ReadOnly — direct doc39 §2.3 violation)

## Files Involved

- `crates/runtime/src/tcml/manifest.rs` (66-97, 141-155) — `allow_tool_for_manifest`, `build_tool_manifest_for_context`
- `crates/runtime/src/native_agent_loop_prompt.rs` (198-229) — `native_agent_effective_tool_exposure_for_route`
- `crates/runtime/src/agent_kernel/turn_router.rs` (6-57) — keyword-driven route classification
- `crates/runtime/src/native_agent_loop.rs` (458-476, 2814-2837) — manifest construction + EscalateToCodeEdit
- `crates/kernel/src/tool.rs` (81-87, 589, 684-696) — `ToolModelCompatibility`, `tui_fastauto_provider_tool_schema_json`
- `crates/runtime/src/prompt_assembler.rs` (140-172, 325) — `stable_tool_catalog` with max_active_tools limit

## Key Findings

### Finding 1: shell.command Hidden in ReadOnly Exposure (P0)

`allow_tool_for_manifest` excludes `shell.command` under ReadOnly because its risk is `ExecutesCommand`, not `ReadOnly`. **Direct violation of doc39 §2.3**: "shell.command始终在manifest中" (shell.command always in manifest).

### Finding 2: write/edit Tools Hidden in ReadOnly (P0)

Same mechanism excludes `file.write`, `file.edit`, `patch.apply`. **Direct violation of doc39 §2.3**.

### Finding 3: Prompt Keywords Drive Tool Exposure (P2)

`TurnRouter.classify()` uses keyword matching on the user prompt to determine `TurnRoute`, which maps to `NativeAgentToolExposure`. This creates an indirect prompt-keyword → exposure → manifest chain. A prompt with "fix" gets CodeEdit exposure (write tools visible); "explain" gets more restricted. **Violates doc39 §2.4** (explicitly rejects prompt-keyword exposure control).

### Finding 4: Mid-Loop Manifest Change (P1)

`EscalateToCodeEdit` replaces the manifest mid-loop with a new tool set. The model sees new tools without an explicit signal that the policy changed.

### Finding 5: model_compatibility Field Never Enforced (P1)

`ToolModelCompatibility` enum (All/NativeOnly/DeepSeekNative/QwenNative/CompatibleProvider) is defined but `allow_tool_for_manifest` never checks it.

### Finding 6: Qwen Silent Truncation (P2)

`stable_tool_catalog` applies `.take(max_active_tools)` which can silently drop `shell.command` due to budget constraints, not policy.

### Finding 7: Separate tui_fastauto Path (P1)

`tui_fastauto_provider_tool_schema_json()` maintains independent filtering that also excludes `shell.command` and `patch.apply`.

## doc39 Conflict

**Yes, multiple violations:**
- §2.3: shell.command + write/edit tools not always in manifest
- §2.4: Prompt-keyword-based exposure control (indirect via TurnRoute)
- §3.1: model_compatibility not enforced
- §2.1: Manifest changes between turns and mid-loop

## Suggested Fix

1. Always include all registered tools in the manifest regardless of exposure mode
2. Move execution control entirely to PermissionPolicy (Allow/Ask/Deny)
3. Remove TurnRoute → exposure mapping; make manifest stable
4. Unify tui_fastauto path with main manifest builder
5. Enforce model_compatibility in allow_tool_for_manifest

## Handoff Needed

- Agent 14 (Security) — PermissionPolicy needs to handle all tool governance
