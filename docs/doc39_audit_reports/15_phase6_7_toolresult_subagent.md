# Audit 15: Phase 6+7 — ToolResult Format + Subagent

**Date:** 2026-05-19 | **Files:** tool_result_format.rs, error_factory.rs, runtime_facade.rs (subagent)

## Phase 6: ToolResult Format + Error Catalog

### 1. ResultFormatter — No Trait Exists

Standalone formatting functions in `tool_result_format.rs`. Implemented but **not all wired**:

| Tool | Formatter | Wired? | Live output matches spec? |
|---|---|---|---|
| file.read | format_file_read_preview (line numbers) | NO | FAIL — inline format without line numbers |
| file.edit | format_file_edit_preview (unified diff) | NO | FAIL — inline format without diff |
| file.write | format_file_write_preview | YES | PASS |
| file.multi_edit | format_file_multi_edit_preview | YES | PASS |
| shell.command | format_shell_command_preview (timing) | NO | FAIL — no elapsed time |
| file.list_directory | format_list_directory_preview | YES | PASS |
| file.list_tree | format_list_tree_preview | YES | PASS |

**Key gap:** file.read, file.edit, shell.command — the 3 most important tools — have spec formatters implemented but unwired.

### 2. ToolErrorCode: 5/9 Variants

| Spec Variant | Implemented? |
|---|---|
| UnknownTool | Yes |
| PlanModeRequired | **NO** |
| PermissionDenied | Yes |
| SafetyDenied | **NO** |
| SchemaValidationFailed | Yes |
| MalformedJson | Yes (as MalformedToolJson) |
| RelationalInvariantFailed | **NO** |
| ToolExecutionFailed | Yes (as ToolFailed) |
| BudgetExhausted | **NO** |

Extra (not in spec): SensitivePath, PathEscapesWorkspace, ToolTimeout

### 3. ModelReadableToolError: 3/7 Fields

| Field | Status |
|---|---|
| error_code | Present (as String, not enum) |
| tool_name | Present |
| short_message | Present |
| retryable | Present |
| **field_errors** | **MISSING** |
| **retry_hint** | **MISSING** |
| **retry_example** | **MISSING** |
| **counts_against_budget** | **MISSING** |

Has extra: `suggested_replacement: Option<String>` (not in spec)

---

## Phase 7: Subagent (task.dispatch)

### 4. task.dispatch Tool — DOES NOT EXIST

- No entry in `core_tool_specs()`
- No entry in tool manifest
- No tool dispatcher handler
- No schema definition

### 5. run_subagent_task — Hardcoded, No LLM

Current implementation (`runtime_facade.rs:1618-1776`):
1. repo.map (root=".")
2. Optionally file.read or repo.map from message path
3. Optionally search.ripgrep from message pattern
4. git.status

All tools run in `ReadOnlyPreview` mode. No LLM turn. No tool choice by model.

### 6. Subagent Isolation — Missing

| Requirement | Status |
|---|---|
| Independent EventLog/AgentSession | NO — events in parent session |
| write_scope enforcement | NO — only validated, never applied (ReadOnlyPreview only) |
| Cancellation (abort handle) | NO — just flips status enum |
| Worktree isolation (git worktree) | NO — plan-only, never executed |
| Flash model for subagent | N/A — no LLM call |

### 7. Phase 6+7 Summary

| Phase | Completion |
|---|---|
| Phase 6 (ToolResult + Error Catalog) | ~30% |
| Phase 7 (Subagent) | ~5% |

**Critical missing:** task.dispatch tool, LLM-driven subagent, subagent isolation, complete ModelReadableToolError.
