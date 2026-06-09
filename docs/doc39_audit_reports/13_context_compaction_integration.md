# Audit 13: Context & Compaction Integration

**Date:** 2026-05-19 | **Files:** runtime_facade.rs, prompt_assembler.rs, context_budget.rs, compactor.rs, conversation_history.rs

## Status Update — 2026-05-20

**Current status: PARTIALLY FIXED.** The original ConversationHistory injection finding is now stale, while the compaction/EventLog findings remain valid.

Fixed after the original audit:

- `conversation_history_openai_json()` is now used in production by the DeepSeek/Qwen facade live-loop prompt builder.
- `build_context_bundle()` intentionally remains project/runtime context only; conversation history is injected separately as `# Conversation History (OpenAI JSON)`.
- Same-turn structured `tool_result` continuation now also routes through `ConversationHistory` projection before provider-block mapping.
- Tests verify that context bundle excludes `conversation_history.*`, while turn 2 requests include prior `tool_call_id`.

Still valid gaps:

- `Compactor::compact()` remains read-only over `EventLog`; it compacts the model request projection, not the persisted event log.
- Compaction still does not call a Flash/role-split model.
- Reasoning preservation during compaction remains partial because older raw reasoning is folded into placeholders.
- Context items still live outside the DeepSeek cache Zone C policy.

## 1. Context Assembly Call Chain

```
RuntimeFacade::build_context_bundle(session_id)
  ├── ContextBundleBuilder
  ├── user_task, project_instructions (AGENTS.md)
  ├── repo_map, git_status
  ├── session_memory (last 12), file_state previews (32)
  ├── discovered_roots (8), path_corrections (16)
  └── build() → ContextBundle

Separately injected by facade native-loop prompt construction:
  conversation_history_openai_json()
    → conversation_messages_from_event_log()
    → conversation_messages_to_openai_json()
```

**Critical: Conversation history is NOT in ContextBundle.** Test explicitly verifies this:
```rust
assert!(!bundle.items.iter().any(|item| item.source.starts_with("conversation_history")));
```

## 2. Compaction Trigger Chain

```
evaluate_native_context_guard()          [native_turn_controller.rs:290]
  ├── estimated_total = estimate_tokens(body) + max_tokens
  ├── If DeepSeek && total > 240K → Blocked (abort)
  └── If DeepSeek && total > 192K → CompactionRequired
        ├── Compactor::compact(event_log) → CompactionResult
        ├── reasoning_replay.compact_old_reasoning()
        └── Rebuild request with compaction summary
```

Two call sites in native_agent_loop.rs:
- Line 555: tool continuation path
- Line 1024: initial request path

## 3. Fake vs Real Compaction

**Compactor::compact() is read-only.** Takes `&EventLog`, returns `CompactionResult`. EventLog is never modified. No events deleted, truncated, or replaced.

**What's actually compacted:** The HTTP request body is rebuilt with old turns replaced by markdown summary. EventLog remains full and append-only.

**Reasoning compaction is real:** `reasoning_replay.compact_old_reasoning()` mutates the in-memory HashMap, replacing raw reasoning with placeholder text.

## 4. ConversationHistory Injection: IMPLEMENTED AS PROMPT SECTION

`RuntimeFacade::build_context_bundle()` now remains project/runtime context only, and `conversation_history_openai_json()` is injected separately into DeepSeek/Qwen facade live-loop prompts as `# Conversation History (OpenAI JSON)`.

Remaining nuance: this is not yet represented as a first-class typed `Vec<OpenAIChatMessage>` field across every provider request builder. It is a structured JSON prompt section plus a ConversationHistory-owned same-turn continuation projection.

## 5. Context Lifecycle

```
BUILD: build_context_bundle() → ContextBundle (no conversation_history)
  │
SEND: assemble_native_prompt() → system + user messages
  │     Cache zones: A (immutable system+tools), B (session metadata), C (tool catalog)
  │     ContextBundle items go to <context> block, NOT into cache zones
  │
GUARD: evaluate_native_context_guard()
  ├── <192K → Send
  └── ≥192K → CompactionRequired → Compactor::compact() (read-only)
                                    → Rebuild request
  │
RECEIVE: New events → EventLog (append only, never truncated)
  │
REBUILD: Next build_context_bundle() — old events remain, EventLog grows unbounded
```

## 6. Key Gaps

| Gap | Status |
|---|---|
| ConversationHistory injected into prompt | DONE |
| Compactor modifies EventLog | NOT DONE (read-only) |
| Compactor uses Flash model | NOT DONE (no LLM call in compaction) |
| 192K threshold | DONE |
| Reasoning preservation during compaction | PARTIAL (placeholder replaces raw content) |
| 3-zone cache prefix | DONE for system prompt; context items NOT in zones |

## 7. Phase 3+4 Completion: ~40%
