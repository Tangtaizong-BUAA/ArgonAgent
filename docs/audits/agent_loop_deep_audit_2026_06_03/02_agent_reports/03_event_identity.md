Now I have all the information needed. Here is the complete audit report.

---

## Event Identity Chain Audit Report

### Conclusion

The event identity chain in deep-code has a **dual-ID architecture** (internal `tool_call_id` and external `provider_tool_call_id`) that is **mostly consistent within a single loop invocation**, but relies on **string-based guessing** for cross-ID mapping between `permission_id`/`plan_approval_id` and `tool_call_id`. The merge-time `_loop_` suffix rewrite is thorough for tool/model IDs but **omits `permission_id` and `plan_approval_id`**, creating a latent collision risk. The EventLog invariant checker (`event_invariants.rs`) provides good detection of orphan tool events within a single EventLog but does not catch cross-invocation or cross-ID-type mismatches.

---

### Question 1: Every Distinct ID Type in the Tool Call Lifecycle

There are **7 distinct ID types** involved:

| ID Type | Created | Consumed | Stable Across Pipeline? |
|---|---|---|---|
| **`tool_call_id`** | `native_agent_loop.rs:2140` — `format!("native_loop_v2_tool_{iteration}_{tool_index}")` | `session.rs` recording methods, `tool_execution.rs`, `event_invariants.rs` | Yes within one loop; rewritten during merge |
| **`provider_tool_call_id`** | Model stream response or synthetic fallback `format!("toolu_v2_{iteration}_{tool_index}")` at `native_agent_loop_tools.rs:138-146` (via `model_provider_tool_call_id()`) | Stored alongside `tool_call_id` in event payloads; used in `conversation_history.rs` for OpenAI projection | Potentially not stable if derived from iteration/tool_index pattern rather than true provider ID |
| **`permission_id`** | `runtime_facade_impl.rs:1154` — `format!("{tool_call_id}_fast_auto_permission")`; `runtime_facade_impl.rs:1218` — `format!("{tool_call_id}_permission")` | `session.rs` `request_permission()` and `decide_permission()` | Yes, but reverse lookup to `tool_call_id` is by string suffix stripping (fragile) |
| **`plan_approval_id`** | `native_agent_loop.rs:2141` — `format!("{tool_call_id}_plan_approval")`; also `runtime_facade_impl.rs:2295` — `format!("{}_approval", plan.plan_id)` | `session.rs` `request_plan_approval()`, `decide_plan()` | Yes, but constructed from `tool_call_id` by string concat; `runtime_facade_impl.rs:637-639` reverses via `strip_suffix("_plan_approval")` |
| **`ledger_tool_call_id`** | `native_agent_loop.rs:1890` — `format!("native_loop_v2_ledger_{iteration}_{tool_index}")` | `turn_controller.record_tool_pending()`, `record_tool_completed()` | Internal to turn controller; **distinct from the event-log `tool_call_id`** |
| **`call_id`** (model) | `session.rs:record_model_call_started()` | `session.rs:record_model_call_completed()`, `record_model_call_blocked()` | Yes; rewritten during merge |
| **`stream_id`** (model) | `session.rs:record_model_stream_delta()` (caller-provided) | `session.rs:record_model_stream_completed()`, assistant text block tracking | Yes; rewritten during merge |

**Hidden risk**: `ledger_tool_call_id` and `tool_call_id` have different formats (`native_loop_v2_ledger_*` vs `native_loop_v2_tool_*`) but refer to the same logical tool call. If any code path confuses them, the turn controller's state machine and the event log will disagree about which tools are pending/completed.

---

### Question 2: Is `tool_call_id` the Same as `provider_tool_call_id`?

**No, they are fundamentally different.**

- `tool_call_id` is the **runtime-internal ID**, e.g., `"native_loop_v2_tool_0_0"`
- `provider_tool_call_id` is the **model/provider's ID** from the API response, or a synthetic fallback if the provider did not assign one

**Mapping mechanism**: There is **no explicit mapping table**. Both IDs are stored together in the event payload. The `RuntimeEventPayload` variants (`ToolCallRequested`, `ToolCallCompleted`, `ToolCallAssembled`, `ToolResultRecorded`) all carry both fields:

```rust
// session.rs (payload.rs types):
ToolCallRequested {
    tool_call_id: String,
    tool_id: String,
    provider_tool_call_id: Option<String>,  // <-- optional
},
```

**Where the mapping is consumed**: In `conversation_history.rs` lines 73-83, when projecting events to OpenAI format, `provider_tool_call_id` is preferred over `tool_call_id`:

```rust
// conversation_history.rs:83
id: provider_tool_call_id.unwrap_or(internal_tool_call_id),
```

**Fallback mechanism**: If no provider ID exists, `model_provider_tool_call_id()` at `native_agent_loop_tools.rs:138-146` constructs a synthetic one:
```rust
// If parsed_tool.provider_tool_call_id is None, fallback is the pattern:
// "toolu_v2_{iteration}_{tool_index}"
// or for concurrent mode: "toolu_v2_conc_{iteration}_{idx}"
// or for loop guard: "toolu_v2_loop_guard_{iteration}_{tool_index}"
```

This means the `provider_tool_call_id` stored in the event log may NOT be a real provider ID at all, but a synthetic runtime construct. This is indistinguishable later.

---

### Question 3: `_loop_` Suffix on IDs

**Meaning**: When events from one loop invocation are merged into another session's EventLog (`session.rs:988-1029`, `merge_events_with_id_suffix()`), all known ID fields get `_loop_{seq}` appended to avoid collisions.

**The rewrite set** (`REWRITABLE_ID_KEYS` at `session.rs:1125-1131`):
```rust
const REWRITABLE_ID_KEYS: &[&str] = &[
    "id",
    "call_id",
    "stream_id",
    "tool_call_id",
    "provider_tool_use_id",
];
```

**Consistency assessment**: The rewrite is recursive (handles nested JSON objects/arrays), which is thorough. However, the following IDs are **NOT rewritten**:
- **`permission_id`** — NOT in the list
- **`plan_approval_id`** — NOT in the list
- **`patch_id`** — NOT in the list

**Evidence** (`session.rs:1448` test assertion):
```rust
// merge_events_preserves_pending_permission_identity_and_state test
// The test explicitly ASSERTS that permission_id is NOT rewritten:
assert!(!merged_jsonl.contains("native_loop_v2_write_perm_120_loop_99"));
```

**Severity**: This is correct behavior for the current code since `permission_id` is constructed from `tool_call_id` by string concat (so if `tool_call_id` gets rewritten, `permission_id` would become stale), but only works because `permission_id` is carried in a non-rewritten field. If a merge results in two events with the same `permission_id` (unlikely but possible), the `pending_permission` tracking in `session.rs` would break.

---

### Question 4: String-Based Guesses for IDs

**Yes, there are three critical string-based guesses:**

**(A) `permission_id` to `tool_call_id` reverse lookup** (`runtime_facade_impl.rs:3899-3912`):
```rust
let tool_call_id = permission_id
    .strip_suffix("_fast_auto_permission")
    .or_else(|| permission_id.strip_suffix("_permission"))
    .map(str::to_string)
    .or_else(|| {
        permission_id
            .strip_prefix("native_loop_command_perm_")
            .map(|index| format!("native_loop_tool_{index}"))
    })
    .or_else(|| {
        permission_id
            .strip_prefix("native_loop_patch_perm_")
            .map(|index| format!("native_loop_tool_{index}"))
    });
```
This function has a **primary path** that searches the event log for the permission.requested event and then scans backward for the most recent `tool.call_requested` with a matching `tool_id`. The string suffix strip is only a **fallback** if the event-log search fails. However, the event-log search matches on `tool_id`, NOT on `tool_call_id`, which means if multiple tool calls with the same `tool_id` are pending, the lookup may return the wrong one.

**(B) `plan_approval_id` from `tool_call_id` construction** (`native_agent_loop.rs:2141`, `runtime_facade_impl.rs:927`):
```rust
let plan_approval_id = format!("{tool_call_id}_plan_approval");
```
**Reverse** (`runtime_facade_impl.rs:637-639`):
```rust
let plan_tool_call_id = plan_id
    .strip_suffix("_plan_approval")
    .unwrap_or(plan_id)
    .to_string();
```

**(C) `permission_id` from `tool_call_id` construction** (`runtime_facade_impl.rs:1154`):
```rust
let permission_id = format!("{tool_call_id}_fast_auto_permission");
```

**Severity**: P2. If any code path generates a `permission_id` or `plan_approval_id` that does not follow these exact patterns, the reverse lookup will silently return `None`, and the `tool_call_completed` / `tool_result_artifact` events will never be recorded for that tool. The event invariant checker (`event_invariants.rs:278-281`) would then flag `"tool completed without recorded result: ..."` as a warning.

---

### Question 5: Orphan Tool Results

**Detection mechanism**: `event_invariants.rs` tracks three disjoint sets and validates:

| Check | Lines | Logic | Severity |
|---|---|---|---|
| `tool.call_completed` without `tool.call_requested` | 90-93 | Errors if not in `tool_requested` set | Error |
| `tool.result_recorded` without `tool.call_completed` | 108-110 | Errors if not in `tool_completed` set | Error |
| `tool.call_completed` without `tool.result_recorded` | 278-281 | Warns if in `tool_completed` but not in `tool_result_recorded` | Warning |

**Real-world orphan scenario in native_agent_loop.rs**: At line 1555, there is a test named `native_agent_loop_v2_streamed_parsed_mismatch_gets_synthetic_tool_result`. This handles the case where streamed tool calls and parsed tool calls diverge. `append_stream_mismatch_error_results()` (`native_agent_loop_tools.rs:194`) detects parsed tool calls not found in the streamed batch and generates synthetic error results for them. If this mismatch logic misses a case, the tool call is requested in the event log but never completed or given a result.

**Additional risk**: In `conversation_history.rs:115`, `tool.permission.denied` and `tool.permission.resolved` events also generate tool-result messages. But these are **runtime events** (`record_runtime_event`), not structured tool lifecycle events. The event invariant checker does NOT cross-reference these with `tool.call_requested` events, so a `permission.denied` for a tool that was never `tool.call_requested` would go undetected.

---

### Question 6: Conversation History Mapping for OpenAI Format

`conversation_history.rs:66-89` processes `"tool.call_requested"` events:

```rust
"tool.call_requested" => {
    // ...
    let provider_tool_call_id =
        extract_json_string(&event.payload_json, "provider_tool_call_id");
    let arguments_json = assembled_arguments
        .remove(&internal_tool_call_id)  // looked up by INTERNAL tool_call_id
        .unwrap_or_else(|| "{}".to_string());
    messages.push(ConversationMessage {
        tool_calls: vec![ConversationToolCall {
            id: provider_tool_call_id.unwrap_or(internal_tool_call_id),
            // ^^^^ provider_tool_call_id preferred over tool_call_id
            tool_id,
            arguments_json,
        }],
    });
}
```

For tool results (`"tool.result_recorded"`, lines 91-106), the same preference applies:
```rust
tool_call_id: Some(provider_tool_call_id.unwrap_or(internal_tool_call_id)),
```

**Critical detail**: The `assembled_arguments` map at lines 34 and 54-64 collects `arguments_json` from `"tool.call.assembled"` events keyed by `tool_call_id`. Later, `"tool.call_requested"` **removes** the entry by the same `tool_call_id`. This means the assembled args are keyed by **internal** `tool_call_id`, not `provider_tool_call_id`. If a tool call is assembled but never requested, its args remain leaked in the map.

**Impact**: When projected to OpenAI format via `conversation_messages_to_openai_json()`, the `id` field in `tool_calls[]` uses `provider_tool_call_id` (or fallback), and the `tool_call_id` in tool role messages also uses it. This is correct for OpenAI's API which expects the provider's ID. **This mapping is stable and correct.**

---

### Question 7: `permission_id` Linked to `tool_call_id`

**Yes, but through a fragile event-log search, not a stored mapping.**

There are two paths:

**(A) Direct path** — `infer_permission_tool_hint()` at `runtime_facade_impl.rs:3867`:
1. Iterates events in reverse looking for the `"permission.requested"` event matching the given `permission_id`
2. From that position, scans backward for `"tool.call_requested"` with a matching `tool_id`
3. Returns the `tool_call_id` and `provider_tool_call_id` from that event

**(B) Fallback path** — static string manipulation at `runtime_facade_impl.rs:3899-3912` (described in Q4).

**The trace from permission decision back to specific tool call**:

`decide_permission()` (runtime_facade_impl.rs:309-363):
1. Validates `permission_id` matches session's `pending_permission`
2. Calls `infer_permission_tool_hint()` to find tool_call_id
3. Calls `session.decide_permission(decision)` which updates state
4. Returns `RuntimePermissionDecisionOutcome` with both `tool_call_id` and `provider_tool_call_id`

`resume_native_loop_after_permission_decision()` (runtime_facade_impl.rs:365):
1. Uses `PendingNativeToolExecution` which already carries `tool_call_id` and `permission_id` together
2. Calls `infer_pending_native_tool_identity_from_session()` (line 3973) which does the same event-log scan
3. Records `tool.call_completed` and `tool.result_artifact` with both IDs

**The fallback path can produce the WRONG tool_call_id** if:
- Multiple tool calls share the same `tool_id` (e.g., two consecutive `file.read` calls)
- The `*_permission` suffix is used by code that does NOT construct the ID from `tool_call_id`

**Severity**: P2. The primary path (event-log scan) should work in most cases, but it matches on `tool_id` alone, not on `tool_call_id`. If there are multiple pending calls with the same `tool_id`, the lookup returns the most recent one, which could be wrong.

---

### Question 8: EventLog Consistency for Tool Lifecycle Events

**Assessment**: The three event types (`tool.call_requested`, `tool.call_completed`, `tool.result_recorded`) use **consistent `tool_call_id` values** within a single EventLog. The `event_invariants.rs` checker validates:

1. No duplicate `tool.call_requested` for the same ID
2. No `tool.call_completed` without matching `tool.call_requested`
3. No `tool.result_recorded` without matching `tool.call_completed`
4. Warning for `tool.call_completed` without `tool.result_recorded`

**However**, there is a subtle inconsistency: the **suffix-pattern** variants of the recording functions sometimes include `provider_tool_call_id` and sometimes don't:

| Function | Includes provider_tool_call_id? |
|---|---|
| `record_tool_call_requested()` | No (always `None`) |
| `record_tool_call_requested_with_provider_id()` | Yes (optional) |
| `record_tool_call_completed()` | No |
| `record_tool_call_completed_with_provider_id()` | Yes (optional) |
| `record_tool_call_assembled()` | No |
| `record_tool_call_assembled_with_provider_id()` | Yes |

The `*_preserving_provider_id()` helpers in `native_agent_loop_execution.rs:99-168` call the "with" variant when a provider ID is available and the "without" variant when not. This means **the same tool call can have `provider_tool_call_id` present in one event and absent in another** if the helper is not used consistently. In practice, the helper is used consistently within the native agent loop, but other callers (e.g., `runtime_facade_impl.rs` tool preview/apply paths) use the "without" variants and lose the provider ID.

---

### Hidden Risks

1. **Legacy ID format collision risk**: The `_loop_{seq}` rewrite only handles 5 field names. If new ID fields are added (e.g., for permission IDs or plan approval IDs), they must be added to `REWRITABLE_ID_KEYS` or collisions will occur during merge. Currently, `permission_id` and `plan_approval_id` are correctly NOT rewritten because they are derived from `tool_call_id` by string operations — but this implicit coupling is fragile.

2. **String-based reverse mapping fragility**: `infer_permission_tool_hint()` at line 3899 has a fallback path that does `strip_suffix("_fast_auto_permission")` and `strip_suffix("_permission")`. If a new permission ID format is introduced without updating these patterns, the reverse lookup silently returns `None`.

3. **Same-tool_id collision in permission lookup**: The primary event-log search in `infer_permission_tool_hint()` matches on `tool_id`, not `tool_call_id`. With concurrent tool execution (introduced at `native_agent_loop.rs:1719`), multiple tools with the same `tool_id` could be pending simultaneously. The `tool_id`-based lookup would always return the most recent one.

4. **`ledger_tool_call_id` vs `tool_call_id` divergence**: Both refer to the same tool call but have different ID formats. Any code path that confuses them (e.g., trying to look up a ledger ID in the event log) will silently fail to find a match.

5. **plan_approval_id reverse mapping**: At `runtime_facade_impl.rs:637`, `plan_id.strip_suffix("_plan_approval")` is used to reconstruct the tool_call_id. The `unwrap_or(plan_id)` fallback means if a plan_id does NOT end with `_plan_approval`, the entire plan_id is used as the tool_call_id, which would cause `record_tool_call_completed_with_provider_id` to produce an event with a tool_call_id that was never recorded via `record_tool_call_requested`. The invariant checker would flag this as `"tool.call_completed without request"`.

---

### doc39 Conflict

**Yes, with reference to `docs/doc39_implementation_gap_analysis.md`**.

The doc39 gap analysis confirms at lines 99-106:
- `conversation_history.rs` is 85% aligned but **0% adopted** (no external callers)
- `provider_tool_call_id` priority handling is correct (line 103, 261)

The doc39 architecture specifies that `agent_kernel/conversation_history.rs` should be the single projection boundary. However:
- The runtime still uses its own duplicate `conversation_history.rs` at `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/conversation_history.rs`
- The `native_agent_loop` has its own separate projection logic inline
- The `conversation_messages_from_event_log` function only handles a subset of events compared to the full pipeline

This creates a doc39 compliance gap: the ID mapping logic lives in three places instead of one, increasing the risk of divergence.

---

### Suggested Fixes

1. **Add `permission_id` and `plan_approval_id` to `REWRITABLE_ID_KEYS`**, or document why they are intentionally excluded and add a test that verifies the behavior.

2. **Replace the string-based reverse mapping** in `infer_permission_tool_hint()` with explicit event-log cross-referencing that matches on `tool_call_id` (not `tool_id`), or store the mapping explicitly in a `BTreeMap`.

3. **Replace the `strip_suffix("_plan_approval")` reverse lookup** with an explicit event-log search: scan for the `"plan.mode_entered"` event matching the `plan_approval_id`, extract its `tool_call_id`, then look up the matching `tool.call_requested` event.

4. **Add an invariant check** in `event_invariants.rs` for the `permission_id`-to-`tool_call_id` reverse mapping: when a `permission.decided` event exists, verify that the matching `tool.call_requested` event actually exists with the expected `tool_call_id`.

5. **Route all tool recording through the `*_preserving_provider_id()` helpers** to ensure `provider_tool_call_id` is consistently recorded across all code paths.

6. **Consolidate the ID-mapping logic** into a single module (per doc39's architecture), removing the duplicated logic in `native_agent_loop.rs`.

---

### Not Suggested

- Do NOT create an explicit ID lookup table unless the event-log scan becomes a performance bottleneck. The event-log scan is O(n) and sufficient for current call volumes.
- Do NOT remove the `tool_id`-based fallback from `infer_permission_tool_hint()` without replacing it with an equivalent mechanism. Some code paths (particularly those in `runtime_facade_impl.rs` that handle permission decisions without a full loop context) rely on it.

---

### Handoff Needed

**Yes** — to the team responsible for the `native_agent_loop.rs` refactoring (doc39 Phase 2), specifically:
1. The `ledger_tool_call_id` vs `tool_call_id` duality needs a documented invariant test
2. The `model_provider_tool_call_id()` fallback ID format should be formally specified rather than being a scattered string pattern (`toolu_v2_*`, `toolu_v2_conc_*`, `toolu_v2_loop_guard_*`, `toolu_v2_parsed_*`)
3. The `infer_permission_tool_hint()` function should be reviewed for correctness when concurrent tool execution is enabled
4. The `conversation_history.rs` projection logic should be the single source of truth for OpenAI-format ID mapping, and all inline projection in `native_agent_loop.rs` should route through it