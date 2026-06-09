Now I have a comprehensive view of both codebases. Here is my architecture-level review.

---

## Architecture-Level Review: deep-code Tool Execution & Permission Pipeline vs. OpenClaudeCode TypeScript Reference

---

### 1. TOOL REGISTRATION & DISCOVERY

**OpenClaudeCode (TypeScript):**
- Each tool is a class implementing the `Tool<Input, Output, P>` interface (`/src/Tool.ts`, lines 362-695). Tools have `name`, `aliases`, `call()`, `description()`, `checkPermissions()`, `validateInput()`, `isEnabled()`, `isReadOnly()`, `isConcurrencySafe()`, `prompt()`, and many rendering methods.
- `getAllBaseTools()` in `/src/tools.ts` (line 193) returns the full tool array. Tools are conditionally included via `feature()` flags (e.g., `WebBrowserTool`, `PowerShellTool`, etc.).
- `assembleToolPool()` (line 345) merges built-in + MCP tools with dedup. `filterToolsByDenyRules()` (line 262) strips tools disabled by permission rules before the model sees them.
- `ToolSearchTool` defers tool schemas to reduce prompt size, allowing the model to search for tools dynamically.

**deep-code (Rust):**
- Tools are declarative `ToolSpec` structs in a `LazyLock<Vec<ToolSpec>>` static (`/crates/kernel/src/tool.rs`, lines 163-597). All tools are hardcoded at compile time.
- `core_tool_specs()` returns `&[ToolSpec]`. `find_tool_spec()` does an O(n) lookup.
- No MCP integration exists. No dynamic/external tool loading.
- No `ToolSearchTool` equivalent. Tool exposure is controlled by `ToolManifestExposure` variants (`ReadOnly`, `FastAutoWrite`, `CodeEdit`) that filter `core_tool_specs()` by `risk`/`capability_status`/`enabled_by_default`.

**Architectural Divergence:** deep-code's static compile-time catalog is architecturally simpler but misses three things OpenClaudeCode has: (1) feature-flagged conditional tool inclusion at startup, (2) MCP server tool integration at runtime, (3) deferred tool loading with ToolSearch.

**Gap Severity:**
- No MCP integration: **HIGH** (blocks external tool ecosystems)
- No dynamic/feature-gated tool registration: **MEDIUM** (acceptable for current phase but will need it later)
- No ToolSearch tool: **MEDIUM** (relevant when tool count exceeds context budget)

---

### 2. TOOL EXECUTION FLOW

**OpenClaudeCode:**
- Single unified flow: API returns `tool_use` blocks -> validation (`validateInput()`) -> permission check (`checkPermissions()`) -> execution (`tool.call()`) -> result formatting (`mapToolResultToToolResultBlockParam()`). All tools share this path.

**deep-code:**
- **Three separate execution paths** that duplicate significant logic:

  **Path A - Streaming** (`native_agent_loop_tools.rs:467-824`, `execute_streamed_native_tool_call_collect()`): Handles `CompletedStreamingToolCall` from DeepSeek streaming. Entry called from `handle_native_stream_tool_event()` (line 399). Branches into:
  - Not-in-manifest check -> `execute_model_readable_error_collect()` (line 528)
  - Mediation error -> `execute_model_readable_error_collect()` (line 553)
  - Gated tools (shell/patch/write/edit) -> `tool_permission_decision()` then `execute_tool_with_permission_gate()` (line 635)
  - FastAutoWrite/CodeEdit tools -> `execute_fast_auto_write_collect()` (line 724)
  - Duplicate observation -> `execute_duplicate_observation_collect()` (line 749)
  - Read-only tools -> `execute_read_only_collect()` (line 788)

  **Path B - Parsed** (`native_agent_loop_tools.rs:1294-1578`): Handles parsed tool calls from model text output. Separate `execute_permissioned_command_collect()` and `execute_permissioned_write_collect()`. Repeats the same permission check, artifact writing, and event recording logic as Path A.

  **Path C - Concurrent Batch** (`tool_execution.rs:290-390`, `execute_tool_batch_concurrent()`): Uses `thread::spawn` with a `Mutex<Vec<ToolExecutionResult>>`. Separate from Paths A and B entirely.

- The core engine (`tool_execution.rs:141-190`, `execute_tool_inner()`) dispatches by `tool_id` via a match arm on string literals. This is effectively a hardcoded switch statement -- adding a tool requires editing 3+ separate files.

**Architectural Divergence:** deep-code has 3 parallel execution paths where OpenClaudeCode has 1. The streaming path (Path A, ~357 lines) and parsed path (Path B, ~284 lines) duplicate: permission checking, artifact writing, event recording (tool_call_requested, tool_call_completed, tool_result_artifact), PostToolUse hook dispatch, and `ensure_executing()` checks.

**Missing Abstractions:**
- No `Tool` trait with a uniform `execute()` method -- execution is scattered across hardcoded match arms
- No `ToolCallContext` abstraction to carry shared state (session, store, workspace_root, hooks, etc.) -- each function takes 10-15 parameters
- No pluggable tool registration -- adding a new tool touches `tool_execution.rs` match arms, `native_agent_loop_tools.rs` stream candidate lists (`is_stream_candidate_provider_tool`, `is_stream_executable_tool`), and `tool.rs` specs

**Gap Severity:** **HIGH** (the duplication between streaming and parsed paths is a maintenance hazard; the hardcoded dispatch makes extensibility difficult)

---

### 3. PERMISSION SYSTEM ANALYSIS

**OpenClaudeCode:** Single unified system:
- `PermissionMode`: `default`, `acceptEdits`, `bypassPermissions`, `plan`, `auto`
- Each tool's `checkPermissions()` returns `PermissionResult` (`{behavior: 'allow'|'deny', ...}`)
- Rule-based filtering: `alwaysAllowRules`, `alwaysDenyRules`, `alwaysAskRules` keyed by source (session/project/global)
- Bash command classifier for security
- Denial tracking with fallback-to-prompting threshold
- All permission checks go through `canUseTool()` hook

**deep-code:** Three apparent "parallel" systems that are actually layered:

| System | File | Role | Lines |
|--------|------|------|-------|
| PermissionGate | `agent_kernel/permission_gate.rs` | Orchestrator: command classification + mode policy + rule matching + tool-specific checks | ~900 |
| PermissionPolicy | `agent_kernel/permission_policy.rs` | Mode-based default fallback (Default/Plan/AcceptEdits/DontAsk/BypassPermissions -> Allow/Ask/Deny) | ~100 |
| PermissionRuleStore | `runtime/src/permission_policy.rs` | File-backed persistent rule storage (session/project/global scopes, pattern matching) | ~250+ |

The flow in `native_agent_loop_execution.rs:714-810` (`tool_permission_decision()`) shows how they layer:
1. PermissionGate.evaluate() runs first (which internally calls PermissionPolicy as fallback)
2. If Allow -> return Allow
3. If Deny -> return Denied error
4. If Ask -> check provided_permission_decisions from user

**Analysis:** These are NOT truly 3 parallel systems. They form a layered architecture:
- `PermissionRuleStore` is the data layer (load/save rules from files)
- `PermissionPolicy` is the default policy layer (mode-based fallback)
- `PermissionGate` is the orchestrator (combines rules + policy + classifier + tool-specific checks)

The naming collision (`permission_policy.rs` exists in TWO locations with different contents -- `agent_kernel/permission_policy.rs` for `PermissionPolicy`/`PermissionMode` and `runtime/src/permission_policy.rs` for `PermissionRule`/`PermissionRuleSet`) IS confusing and a legitimate architectural smell. The two files should be renamed:
- `agent_kernel/permission_policy.rs` -> `agent_kernel/permission_mode_policy.rs`
- `runtime/src/permission_policy.rs` -> `runtime/src/permission_rules.rs`

**Missing Abstractions:**
- No `canUseTool` hook equivalent (the HookDispatcher exists but only handles PreToolUse/PostToolUse, not permission decisions)
- No per-tool `checkPermissions()` method on ToolSpec -- permission logic is centralized in PermissionGate with hardcoded tool IDs
- No `PermissionResult` intermediate type between PermissionGate and the caller (uses PermissionResolution directly)

**Gap Severity:** **MEDIUM** (the layered architecture is sound but the naming collision and lack of per-tool permission hooks will cause problems as tool count grows)

---

### 4. TOOL RESULT HANDLING

**OpenClaudeCode:**
- Each tool's `call()` returns `ToolResult<Output>` with typed data
- `mapToolResultToToolResultBlockParam()` converts typed output to API-compatible block
- `maxResultSizeChars` per-tool for result persistence threshold
- `contentReplacementState` for tool result budget management
- `renderToolResultMessage()` for UI rendering
- `extractSearchText()` for transcript indexing
- Rich text formatting (diffs, colors, etc.)

**deep-code:**
- `ToolExecutionResult` struct with flat fields: `preview: String`, `detail_json: String`, `ok: bool`, `exit_code: Option<i32>`
- Results are written to `ArtifactStore` via `write_tool_result_artifact()` -- every single execution path includes the same artifact-writing boilerplate (~15 lines repeated 7+ times)
- Preview is a human-readable one-liner; detail_json is the structured machine-readable version
- No per-tool `maxResultSizeChars` enforcement at execution time (the ToolSpec has the field but it is only used for schema generation, not for actual truncation)
- No content replacement / tool result budget mechanism
- No result rendering abstraction (the TUI/GUI layer would need to parse detail_json)

**Architectural Divergence:** deep-code's preview+detail_json model is simpler and arguably more suitable for a headless kernel. However, the result writing boilerplate is repeated in every execution path instead of being extracted into a single `record_tool_result()` function.

**Missing Abstractions:**
- No centralized result recording function -- the same artifact writing pattern appears in `execute_streamed_native_tool_call_collect()` (lines 695-715), `execute_read_only_collect()` (lines 1205-1225), `execute_permissioned_command_collect()` (lines 1411-1431), `execute_permissioned_write_collect()` (lines 1554-1574), `execute_fast_auto_write_collect()`, `execute_duplicate_observation_collect()`, `execute_model_readable_error_collect()`
- No result size budget or content replacement
- No typed result output per tool (all tools share the same `ToolExecutionResult`)

**Gap Severity:** **MEDIUM** (functional for current use; the boilerplate duplication is a maintenance concern)

---

### 5. PARALLEL TOOL EXECUTION

**OpenClaudeCode:**
- Natively supports parallel tool use blocks at the API level (Anthropic/OpenAI)
- `isConcurrencySafe(input)` per-tool (defaults to `false`)
- The API handles ordering of parallel results

**deep-code:**
- `execute_tool_batch_concurrent()` in `tool_execution.rs:290-390` uses `thread::spawn` with `Arc<Mutex<Vec<ToolExecutionResult>>>`
- `SiblingAbortController` with `Arc<Mutex<bool>>` for abort propagation
- `MAX_TOOL_CONCURRENCY = 10`, but threads are joined in batches of 10
- `concurrency_safe` flag in ToolSpec (derived from `ToolRisk::ReadOnly`)
- Only used via `execute_concurrent_read_only_batch()` in `native_agent_loop_tools.rs:882-949`

**Architectural Divergence:** deep-code uses OS threads instead of async tasks. The thread-per-tool pattern is heavier weight (each thread gets its own stack). An async runtime using tokio tasks would be more appropriate for a tokio-based system. However, the safety design (only read-only tools, sibling abort) is well-considered.

**Gap Severity:** **LOW** (OS threads work correctly; migration to async tasks is an optimization, not a correctness gap)

---

### 6. SEARCH TOOL

**OpenClaudeCode (GrepTool, `/src/tools/GrepTool/GrepTool.ts`):**
- Full ripgrep binary wrapper with all rg features
- Regex patterns
- Output modes: `content`, `files_with_matches`, `count` (line 52-56)
- Pagination: `head_limit` + `offset` (lines 80-83)
- Context lines: `-A`, `-B`, `-C` / `context` (lines 58-67)
- `-i` (case insensitive), `-n` (line numbers) (lines 68-73)
- `glob` filtering (lines 47-51)
- `type` file type filtering (lines 74-79)
- `multiline` mode (lines 86-89)
- VCS directory exclusion (.git, .svn, etc., line 95-102)
- Result sorting by file modification time
- `applyHeadLimit()` with explicit `0 = unlimited` escape hatch

**deep-code (search_tool.rs):**
- Pure Rust, no external binary
- Simple `String::contains()` -- substring matching only, NOT regex (line 162)
- No output modes -- always returns lines containing substring
- No pagination (`head_limit`/`offset` not in SearchRequest struct)
- No context lines
- No case sensitivity toggle
- No glob/type filtering
- No multiline support
- Hardcoded skip list at `should_skip()` (line 177-185): `.git`, `target`, `node_modules`, `.venv`, `__pycache__`, `.DS_Store` -- much shorter than OpenClaudeCode's list

**Architectural Divergence:** This is the single largest functional gap. deep-code's search is `grep -F` (fixed strings), while OpenClaudeCode's is full `ripgrep` (regex, complex filtering, pagination). The model will attempt regex patterns like `fn\s+\w+` that silently return zero results instead of matching. The model also expects `head_limit` pagination to manage output size.

**Gap Severity:** **CRITICAL** -- the model generates regex patterns which will silently fail. This will cause wasted turns, incorrect "no matches" conclusions, and ultimately wrong answers.

---

### 7. SUB-AGENT / SPAWN

**OpenClaudeCode (`spawnMultiAgent.ts`, 1094 lines):**
- Three backends: in-process (same Node.js process), tmux split-pane, tmux separate window
- `handleSpawnSplitPane()` (line 305), `handleSpawnSeparateWindow()` (line 545), `handleSpawnInProcess()` (line 840)
- iTerm2 integration for native split panes
- Mailbox-based IPC for tmux agents via `writeToMailbox()`
- Team file persistence for agent discovery
- Permission mode inheritance from parent
- Model resolution ("inherit" alias, default fallback)
- `registerOutOfProcessTeammateTask()` for background task tracking
- `TaskOutputTool` for receiving results
- Agent definitions from `.claude/agents/` directory

**deep-code:**
- `task.dispatch` tool in `tool_execution.rs:1348-1403` (`execute_task_dispatch_preview()`)
- Deterministic local child: runs `repo.map` + `file.read` + `search.ripgrep` + `git.status` and returns a synthesized summary
- No actual sub-agent process spawning
- No agent definition system
- No multi-agent communication
- Records `subagent.spawned` and `subagent.completed` events but as event summaries only (line 1252-1280)
- The `collect_task_dispatch_evidence()` function (line 1411-1462) is essentially a smart context gathering helper, not a sub-agent

**Architectural Divergence:** This is the largest architectural gap. deep-code's "task.dispatch" is a read-only context collection tool masquerading as a sub-agent system. OpenClaudeCode's multi-agent spawning is a core platform capability (swarms, teams, background tasks).

**Gap Severity:** **CRITICAL** -- the model has been trained to use sub-agents for complex multi-file tasks. deep-code's current implementation returns fabricated completion events without actually running an agent.

---

### 8. ADDITIONAL FINDINGS

**8.1. Duplicate Boilerplate: Artifact Writing**

The pattern below repeats verbatim (with minor naming variations) in at least **7 separate functions**:

```rust
// Pattern repeated in:
// execute_streamed_native_tool_call_collect (line 695)
// execute_read_only_collect (line 1205)
// execute_duplicate_observation_collect (line 1031)
// execute_model_readable_error_collect (line 224)
// execute_permissioned_command_collect (line 1411)
// execute_permissioned_write_collect (line 1554)
// execute_fast_auto_write_collect (line 1665)
// execute_concurrent_read_only_batch (line 917)
// execute_pending_tool_after_decision (line 683)

let artifact = write_tool_result_artifact(
    artifact_store,
    &format!("native_loop_v2_*_{index}"),
    &ToolResultRecord::new(
        &tool_call_id, tool_id, result.ok,
        result.preview.clone(), result.detail_json.clone(),
    ),
).map_err(|error| error.to_string())?;
```

This should be extracted into a single `record_tool_result()` function.

**8.2. Missing Tool Abstractions Compared to OpenClaudeCode**

| OpenClaudeCode Feature | deep-code Equivalent | Status |
|------------------------|---------------------|--------|
| `validateInput()` per tool | Hardcoded validation in each execute_* function | Missing abstraction |
| `checkPermissions()` per tool | Centralized in PermissionGate | Different approach, acceptable |
| `isSearchOrReadCommand()` | Not present | Missing |
| `getPath()` per tool | `path_argument_keys` in ToolSpec | Partial |
| `renderToolUseMessage()` etc. | No rendering layer | Out of scope for kernel |
| `toAutoClassifierInput()` | `classify_command_with_reasons()` in permission_gate.rs | Partial (bash only, no other tools) |
| `isResultTruncated()` | Not present | Missing |
| `strict` mode for API | Not present | Missing |
| `backfillObservableInput()` | Not present | Missing |
| `defer_loading` / `shouldDefer` | Not present | Missing |
| MCP tool integration (`isMcp`, `mcpInfo`) | Not present | Missing |

**8.3. Over-Engineering in deep-code**

1. **`replayed_tool_completion_state()`** (`native_agent_loop_tools.rs:120-136`): Scans the entire JSONL event log per tool call to check if a call was already completed. This is O(n*m) where n = event count and m = tool calls per turn. A simple in-memory HashMap would be more appropriate.

2. **`append_stream_mismatch_error_results()`** (`native_agent_loop_tools.rs:194-258`): Handles a corner case where the model produces parsed tool calls that the streaming executor didn't match. The entire function could be replaced with a simpler approach: just skip mismatched calls or log a single warning.

3. **Three separate `execute_*_collect()` functions** (`execute_permissioned_command_collect`, `execute_permissioned_write_collect`, `execute_fast_auto_write_collect`) that share ~80% of their logic. A single parametrized function would suffice.

4. **`execute_fast_auto_write_create_repair()`** (`native_agent_loop_tools.rs:1739-1788`): Implements an elaborate file-creation retry mechanism with auto-incrementing filenames that seems disproportionate to the problem it solves. The `next_fast_auto_write_create_path()` function (line 1791) iterates from 2..100 trying filenames.

---

### SUMMARY TABLE

| Focus Area | Gap | Severity | Recommendation |
|------------|-----|----------|----------------|
| Search Tool | Substring-only, no regex | **CRITICAL** | Integrate ripgrep binary or add regex crate. Add output modes, pagination, glob filtering. Files: `search_tool.rs`, `tool_execution.rs:1552-1678` |
| Sub-agent/Spawn | Fake sub-agent (read-only context gathering only) | **CRITICAL** | Either implement real sub-agent spawning or rename tool to `context.gather` and remove fabricated completion events. File: `tool_execution.rs:1348-1504` |
| Execution path duplication | 3 parallel paths (streaming/parsed/concurrent) with duplicated logic | **HIGH** | Unify into a single `execute_tool_in_context()` function. Extract artifact-writing boilerplate. Files: `native_agent_loop_tools.rs:467-824`, `1294-1578` |
| No MCP integration | No support for external MCP tools | **HIGH** | Add MCP client + tool registration. Files: `tool.rs` (add MCP tool variant to ToolSpec), new `mcp_tool.rs` |
| Hardcoded tool dispatch | Match-on-string tool dispatch across 3+ files | **HIGH** | Implement a `Tool` trait with `execute()`, register tools via registry. Files: `tool_execution.rs:175-189`, `native_agent_loop_tools.rs:826-880` |
| Permission file naming collision | Two `permission_policy.rs` files with different concerns | **MEDIUM** | Rename `agent_kernel/permission_policy.rs` -> `permission_mode_policy.rs`, `runtime/src/permission_policy.rs` -> `permission_rules.rs` |
| Artifact writing boilerplate | Same ~15 lines repeated in 7+ functions | **MEDIUM** | Extract `record_tool_result(session, store, id, tool_id, result)` |
| No tool result budget | Results can grow unbounded | **MEDIUM** | Implement per-tool `max_result_size_chars` enforcement at execution time |
| Thread-based parallelism | Uses `thread::spawn` instead of tokio tasks | **LOW** | Migrate to tokio::spawn for consistency with the async runtime |
| No per-tool validation abstraction | Validation is hardcoded in execute_* functions | **LOW** | Add `validate(args) -> Result` to a Tool trait |

The two **CRITICAL** items (search tool regex capability, sub-agent system) are the most urgent to address, as they directly affect the model's ability to produce correct results. The search gap will cause silent failures (regex patterns returning 0 matches). The sub-agent gap will cause the model to believe background work was done when nothing actually happened.