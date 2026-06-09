# Audit 07: DeepSeek Stream + Reasoning vs doc39

**Date:** 2026-05-19 | **Files:** stream.rs, stream_processor.rs, reasoning.rs, deepseek/mod.rs

## 1. StreamProcessor State Machine

### doc39 §4 requires:
- `state: StreamProcessorState` (inside_dsml, partial_calls, finish_reason, last_chunk_at)
- `dsml_filter: DsmlChunkFilter`
- `accumulator: ToolCallAccumulator`
- `reasoning_buffer: String`
- `visible_buffer: String`
- `ingest(SseChunk) -> Vec<StreamEvent>` with 6 event types

### Actual implementation:
- `DsmlChunkFilter` IS integrated into `StreamProcessor` (field at line 55)
- `ToolCallPipeline` wraps `StreamingToolCallAssembler` instead of bare `ToolCallAccumulator`
- `reasoning_buffer` replaced by `state.pending_thinking_chars: usize` (simpler char count)
- `visible_buffer` exists as `state.pending_content: String`
- **Missing:** `finish_reason`, `last_chunk_at` fields
- **Missing:** `ContentToolCallCandidate` emission at finish_reason=stop
- **Missing:** `ContentToolCallExtractor::scan()` call

### Event coverage:
| doc39 StreamEvent | Actual | Status |
|---|---|---|
| VisibleDelta(String) | VisibleDelta { chars } | PASS |
| ReasoningDelta(String) | ReasoningDelta { chars } | PASS |
| ToolCallPartial { index, name } | ToolCallPartial | PARTIAL (no fields) |
| ToolCallAssembled | ToolCallAssembled { count } | PASS |
| ContentToolCallCandidate | **ABSENT** | FAIL |
| StreamCompleted { finish_reason } | StreamCompleted | PARTIAL (no reason) |

### Verdict: PARTIALLY COMPLIANT
Core pipeline works but ContentToolCallCandidate extraction and finish_reason handling are missing.

## 2. DsmlChunkFilter — Integrated

Two-phase filtering in `ingest_visible_delta`:
1. `dsml_filter.filter()` — strips `<tool_calls>` tags
2. `content_suppression_heuristic()` — suppresses preamble/post-tool content

Works correctly for B6 (DSML leak prevention).

## 3. ReasoningReplayManager — Fully Wired

- `capture()` — called from main loop (native_agent_loop.rs:1283)
- `inject_if_required()` — wraps via `deepseek_reasoning_replay_for_tool_continuation()`
- `compact_old_reasoning()` — called at both compaction sites (lines 580, 1048)
- B1 (reasoning replay) — PASS
- B2 (separate budget tracking) — PASS

## 4. B1-B6 Compliance

| Behavior | Status |
|---|---|
| B1: reasoning replay in tool chains | PASS |
| B2: reasoning separate budget | PASS |
| B3: Pro/Flash gradient | FAIL (RoleSplit not wired) |
| B4: temperature sensitivity | FAIL (temperature always None) |
| B5: tool_calls.delta cross-chunk | PASS (StreamingToolCallAssembler) |
| B6: DSML/content leak filtering | PASS (DsmlChunkFilter + content suppression) |

## 5. Dead Code

- `stream.rs` `StreamEvent` enum (lines 22-39) — doc39-conformant variants but **never constructed**
- `StreamProcessorState` in stream.rs — **unused** by stream_processor.rs

## Key Gap
No `ContentToolCallExtractor::scan()` — the visible buffer is never scanned for DSML tool calls at finish_reason=stop. B6 content extraction fallback is missing.
