# Audit 04: ConversationHistory + ObservationCache vs doc39

**Date:** 2026-05-19 | **Files:** conversation_history.rs, observation_cache.rs, runtime_facade.rs, prompt_assembler.rs

## Status Update — 2026-05-20

**Current status: PARTIALLY FIXED.** The original critical finding that conversation history was never injected is now stale.

Fixed after the original audit:

- `RuntimeFacade` DeepSeek/Qwen live-loop paths now inject `# Conversation History (OpenAI JSON)` when prior turn history exists.
- `conversation_messages_to_openai_json` is the active cross-turn history projection.
- `build_context_bundle()` remains project/runtime context only and does not add `conversation_history.*` context items.
- Structured same-turn `tool_result` continuations now derive assistant/tool message layout through `ConversationHistory` before mapping to provider-native blocks.
- `conversation_messages_to_context_summary` and the old direct `*_from_batch` continuation block helpers have been removed.
- Regression tests now cover turn 2 seeing turn 1 `tool_call_id`, OpenAI-shape continuation projection, and provider continuation ID preservation.

Still valid gaps:

- ObservationCache coverage remains limited to the known observation tools listed below.
- Error, permission, subagent, and compaction events are still not projected as conversation messages.
- Conversation history is injected as an OpenAI JSON section inside the runtime prompt, not yet as a first-class `Vec<OpenAIChatMessage>` request field all the way down every provider path.

## Original Critical Finding — STALE

**Original finding:** Conversation History NOT Injected.

`conversation_messages_from_event_log()` and `conversation_messages_to_openai_json()` are fully implemented and tested, but **never called in production**. The model sees only system context + current prompt — no previous turns.

---

## 1. Context Assembly Call Chain

```
RuntimeFacade::build_context_bundle(session_id)
  ├── ContextBundleBuilder::new()
  ├── builder.add_user_task()
  ├── builder.add_project_instructions()     ← AGENTS.md
  ├── builder.add_repo_map()
  ├── builder.add_git_status()
  ├── builder.add_memory()                   ← session_memory (last 12)
  ├── builder.add_tool_result_preview()      ← file states
  └── builder.build() → ContextBundle

Separately available in the original audit (now called by facade live-loop prompt construction):
RuntimeFacade::conversation_history_openai_json()
  → conversation_messages_from_event_log()
  → conversation_messages_to_openai_json()
  → Returns OpenAI JSON
```

## 2. Gap vs doc39 Phase 3

| Requirement | Status |
|---|---|
| `session.to_conversation_messages()` | DONE (implemented) |
| assistant includes tool_calls | DONE |
| assistant includes reasoning_content | DONE |
| tool message uses tool_call_id | DONE |
| build_context_bundle = system only | DONE |
| **History injected into model prompt** | DONE |
| "turn 2 sees turn 1 tool_result" test | DONE |
| Same-turn tool_result continuation uses ConversationHistory projection | DONE |
| Provider request history is a first-class typed message vector everywhere | PARTIAL |

## 3. Event Type Coverage

| Event | Produced Message |
|---|---|
| `model.stream_delta` (user input) | user message |
| `model.stream_delta` (content) | appended to assistant |
| `model.stream_delta` (reasoning) | assistant with reasoning_preview |
| `tool.call.assembled` (replayable) | stored in assembled_arguments map |
| `tool.call_requested` | assistant with tool_calls |
| `tool.result_recorded` | tool role with tool_call_id |
| All other events | Silently dropped |

**Gaps:** Error events, permission events, subagent events, compaction events — all silently dropped. Non-replayable tool args produce `"{}"`.

## 4. ObservationCache Coverage

| Tool | Key | Status |
|---|---|---|
| file.read | `file.read:{path}:offset={}:limit={}:max_bytes={}` | Covered (mtime invalidation, range detection, plan-path budget) |
| file.list_directory | `file.list_directory:{path}:hidden={}:max_entries={}` | Covered |
| file.list_tree | `file.list_tree:{path}:depth={}:max_entries={}` | Covered |
| repo.map | `repo.map:{root}:max_files={}:max_depth={}` | Covered |
| git.status | `git.status` (constant) | Covered |
| search.ripgrep | `search.ripgrep:{root}:{pattern}:max_results={}` | Covered |
| **All other tools** | `_ => None` | **NOT COVERED** |

## 5. Doom Loop Prevention (B12)

**For 6 known tools:** Effective — detects exact and range-covered duplicates, suppresses re-execution, terminates loop after 2+ consecutive duplicates.

**For unknown tools:** INEFFECTIVE — `observation_key()` returns `None`, no duplicate detection possible.

**Risks:**
- New read-only tools bypass cache entirely
- Synthetic "already observed" message is misleading for search tools
- Cache is per-TurnState, lost when turn ends
- No persistent storage across sessions
