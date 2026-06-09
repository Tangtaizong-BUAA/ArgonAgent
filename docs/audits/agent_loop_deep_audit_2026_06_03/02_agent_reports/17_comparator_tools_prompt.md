Now I have a comprehensive view of all the files. Let me compile the final review.

---

# Line-by-Line Code Review: deep-code Rust Implementation vs OpenClaudeCode TypeScript Reference

## File 1: `tool_execution.rs` (3103 lines) vs `tools.ts` (389 lines)

### Why the 8x size difference?

The size discrepancy is not a quality problem. It comes from:

1. **Architecture**: TypeScript uses OO polymorphism -- each tool is a class in its own file. `tools.ts` is just an index/registry (389 lines). The actual tool implementations (BashTool, FileEditTool, etc.) live in 30+ separate files. Rust inlines ALL tool implementations into one file (read, write, edit, multi-edit, shell, patch, search, repo_map, git_status, todo, ask_user, task_dispatch, csv_profile, lsp_diagnostics), making it the equivalent of 14 class files concatenated.

2. **Protocol adaptation**: Rust handles 3+ wire protocols (Anthropic-compatible, OpenAI-compatible, DSML), streaming event parsing, and multi-provider support. TypeScript targets Anthropic SDK only.

3. **Safety scaffolding**: Rust has explicit path resolution, workspace-boundary checks, TOCTOU prevention, and structured error reporting that TypeScript relies on OS/Node for.

4. **Inline tests**: ~800 lines of unit tests live in the same file.

---

### Issues by Severity

**CRITICAL (3 issues)**

| # | Line Range | Severity | Description | Reference | Fix |
|---|---|---|---|---|---|
| 1 | `tool_execution.rs:290-390` | CRITICAL | **No concurrency safety check during parallel execution**. `execute_tool_batch_concurrent` runs all tools in threads without checking `spec.concurrency_safe`. A `file.write` could race with a `file.edit` on the same path. | `Tool.ts:402` -- `isConcurrencySafe()` is checked before parallel execution in OpenClaudeCode. | Add a pre-flight check: before spawning threads, filter tools by `find_tool_spec(tool.tool_id).concurrency_safe`. Non-safe tools should run sequentially after the batch. |
| 2 | `tool_execution.rs:1-3103` | CRITICAL | **No per-tool execution timeout**. There is no mechanism to kill a runaway tool after N seconds. Shell commands or file operations can hang indefinitely. | `Tool.ts:416` -- `interruptBehavior()` returns `'cancel'` or `'block'` so the UI knows whether to kill the tool or wait. | Add a `timeout_ms` field to `ToolExecutionRequest` and wrap execution in `tokio::time::timeout` or a thread with a deadline. |
| 3 | `tool_execution.rs:895-980` | CRITICAL | **TOCTOU race in `execute_file_read_preview` and `execute_file_write`**. The file is resolved/checked, then later read/written. Between the check and the operation, another concurrent operation can change the file. | OpenClaudeCode uses `readFileState` cache (`Tool.ts:181`) to track file states and detect staleness across tools within a single turn. | The `base_hash` check covers the write path, but the read path has no staleness guard. Use an in-memory content hash cache across reads to detect intra-turn modification. |

**HIGH (4 issues)**

| # | Line Range | Severity | Description | Reference | Fix |
|---|---|---|---|---|---|
| 4 | `tool_execution.rs:337-338` | HIGH | **Shell command sibling abort triggers on failure only**. The abort only fires for `shell.command` failures. Other tool failures (e.g., a `file.edit` that corrupts state) do not trigger sibling abort. | OpenClaudeCode uses `abortController` for all tool types, not just shell. | Generalize the abort condition to any tool that is not `concurrency_safe`, or make it configurable per batch. |
| 5 | `kernel/tool.rs:98-118` | HIGH | **ToolSpec has `enabled_by_default` but no dynamic `isEnabled()` method**. In OpenClaudeCode, `isEnabled()` can check runtime conditions (feature flags, environment variables). The Rust tool catalog is entirely static. | `Tool.ts:403` and `tools.ts:181-182` -- `isEnabled()` per tool is checked at runtime. | Add an `is_enabled()` closure or method to `ToolSpec` that can check runtime state (filesystem capabilities, env vars). |
| 6 | `kernel/tool.rs:120-600` | HIGH | **No MCP tool integration interface**. The tool catalog is a fixed `LazyLock<Vec<ToolSpec>>`. There is no way to dynamically add MCP tools from external servers at runtime. | `tools.ts:383-389` -- `getMergedTools()` merges built-in and MCP tools dynamically. `Tool.ts:436` -- `isMcp` flag and `mcpInfo` metadata. | Add a runtime tool registry alongside the static catalog, with functions to register and deregister MCP tools per session. |
| 7 | `tool_execution.rs:1348-1402` | HIGH | **`execute_task_dispatch_preview` is a fake -- it does not spawn a real subagent**. It collects evidence locally and returns a deterministic summary. The function name says "preview" but it is the actual implementation. | OpenClaudeCode's `AgentTool` and `Task` tools actually spawn isolated subprocesses/subagents with full model interaction. | Either rename to indicate it is the bounded deterministic path, or implement actual subagent spawning. This is a known gap (doc39), but the naming is misleading. |

**MEDIUM (5 issues)**

| # | Line Range | Severity | Description | Reference | Fix |
|---|---|---|---|---|---|
| 8 | `kernel/tool.rs:28-35` | MEDIUM | **No `isDestructive` semantic**. `ToolRisk` has `WritesFiles` and `ExecutesCommand` but no distinction between "writes a new file" (safe-ish) and "deletes files" (destructive). | `Tool.ts:404-406` -- `isDestructive()` (optional, defaults to false) marks irreversible operations. | Add an `is_destructive` flag to `ToolSpec` and use it in permission decisions. |
| 9 | Entire Rust codebase | MEDIUM | **No `defer_loading` optimization**. All tools are sent to the model immediately. OpenClaudeCode can defer tools and require a `ToolSearch` call first -- critical for scaling beyond ~20 tools. | `Tool.ts:442-448` -- `shouldDefer` and `alwaysLoad` properties control when a tool's full schema appears in the prompt. `tools.ts:247-249` -- `ToolSearchTool` enables deferred tool discovery. | Add a `should_defer` field to `ToolSpec` and implement deferred tool loading in the manifest builder. |
| 10 | `native_agent_loop_prompt.rs:335-372` | MEDIUM | **User intent classification via naive substring matching**. `native_prompt_wants_file_generation` checks for Chinese/English substrings to classify intent. False positives/negatives are likely. | OpenClaudeCode relies on the model's own understanding via the system prompt, not text classification. | Consider removing this heuristic entirely and letting the model decide. If it stays, add tests with adversarial examples. |
| 11 | `native_agent_loop_tools.rs:952-991` | MEDIUM | **Duplicate observation suppression uses string comparison, not content hashing**. `tool_calls_are_cached_observations` compares tool ID + arguments as strings. `contains_in_workspace` uses the same approach. | OpenClaudeCode's file state cache (`Tool.ts:181`) uses content hashes and LRU eviction. | Use content hashes (e.g., the `stable_text_hash` already in the codebase) instead of string comparison for cache keys. |
| 12 | `tool_execution.rs:261-263` | MEDIUM | **`canonical_json_text` is fragile**. It round-trips through `serde_json::Value` to canonicalize. This can lose data (e.g., integer precision > f64, duplicate keys). | OpenClaudeCode doesn't need this because tool calls arrive pre-parsed from the API. | Use a streaming canonicalizer or accept that non-canonical JSON is fine as long as it's semantically equivalent. At minimum, log warnings when canonicalization changes the input. |

**LOW (4 issues)**

| # | Line Range | Severity | Description | Reference | Fix |
|---|---|---|---|---|---|
| 13 | `tool_execution.rs:654-657` | LOW | **`execute_file_edit` allows `old_string == new_string` check AFTER line-ending normalization (line 668)**, but checks it only at line 669. A wasteful round-trip: hash check, stale check, then identity check. | N/A -- this is a minor ordering issue. | Move the identity check before the hash check to fail fast. |
| 14 | `native_agent_loop_prompt.rs:473-511` | LOW | **Line count validation only applies to `file.write`, not `file.edit` or `file.multi_edit`**. If the user requests a 30-line file and the model uses `file.edit` to construct it in pieces, the line count guard does not trigger. | OpenClaudeCode doesn't have this guard -- it trusts the model. The Rust guard is extra, but incomplete. | Extend the validation to `file.edit` and `file.multi_edit`, or remove it as over-engineering. |
| 15 | `kernel/tool.rs:767-779` | LOW | **`json_escape` is duplicated**. `tool_execution.rs:2192-2211` has its own `json_string()` function, and `kernel/tool.rs` has `json_escape()`. They differ: the kernel version maps `\u{08}` to `\b` while the execution version does too but with a different control char approach. | OpenClaudeCode uses a single JSON serialization path (via the API). | Consolidate into a single `json_escape` function shared across the crate. |
| 16 | `native_agent_loop_tools.rs:178-191` (approximately) | LOW | **`append_stream_mismatch_error_results` generates errors for every parsed tool call not matched in the streamed batch**. This can produce confusing ghost errors if the model outputs tool calls in text but the streaming executor handled them through a different path. | OpenClaudeCode uses a single code path for tool call ingestion (via the SDK). | Add a debug log to distinguish between "streaming pipeline dropped this call" and "model output this call in non-streaming format". |

---

### Bug Count Summary

| Severity | Count |
|---|---|
| CRITICAL | 3 |
| HIGH | 4 |
| MEDIUM | 5 |
| LOW | 4 |
| **Total** | **16** |

---

### Missing Feature Count

1. **MCP tool integration** -- no runtime dynamic tool registration
2. **Deferred tool loading** -- no `shouldDefer` / `alwaysLoad` mechanism
3. **Per-tool execution timeout** -- no timeout or abort mechanism
4. **`isDestructive()` discrimination** -- no distinction between safe writes and destructive operations
5. **`isSearchOrReadCommand()`** -- no UI collapse optimization
6. **Dynamic `isEnabled()`** -- no runtime feature flag support per tool
7. **`userFacingName()` per input** -- static `display_name` only
8. **`interruptBehavior()`** -- no cancel-vs-block semantics
9. **Real subagent spawning** -- `task_dispatch` is a deterministic local preview
10. **Session-scoped tool result budget** -- no `ContentReplacementState` equivalent
11. **`backfillObservableInput()`** -- no hook/pre-observation input transformation

**Total: 11 missing features** (these are known gaps per doc39, but worth cataloging against the reference).

---

### Overall Assessment Per File

**`tool_execution.rs` (3103 lines)**: B+
Solid execution engine with strong safety properties (TOCTOU checks, workspace containment, stale-hash validation, structured errors). The main gaps are runtime concerns: no tool timeout, no concurrency safety enforcement during parallel batches, and the task dispatch is a placeholder. The file is large but logically coherent -- it contains what would be 14+ separate files in the TypeScript codebase.

**`native_agent_loop_tools.rs` (1855 lines)**: B
Good streaming tool event pipeline with duplicate suppression, stream mismatch detection, and per-protocol event logging. The complexity comes from supporting three protocol formats simultaneously. The permission gate integration is thorough. Main weakness: duplicate observation detection uses string comparison instead of content hashing.

**`native_agent_loop_prompt.rs` (712 lines)**: B-
The system prompt construction is protocol-aware and includes useful governance directives (tool inventory, write directives, continuation hints). The heuristic text classifiers (`native_prompt_wants_file_generation`, `native_prompt_wants_tool_inventory`) are fragile substring matchers that will produce false positives on edge cases. The `validate_fast_auto_write_runtime_constraints` is a creative idea but only covers `file.write`.

**`kernel/src/tool.rs` (954 lines)**: A-
Well-structured tool catalog with clear separation of concerns. The `ToolSpec` struct captures all essential metadata. The schema generation functions for different providers (Anthropic-style, OpenAI-style) and different capability sets (read-only, fast-auto, etc.) are clean. The FNV hash-based catalog hash is a good cache-invalidation primitive. Missing: no dynamic tool addition interface, no `Tool` trait for polymorphism.

**`agent_kernel/tool_inventory.rs` (75 lines)**: A
Clean, simple observation tracking. No issues.