# Agent 12: GUI Event Reducer / Transcript Rendering Audit

## Conclusion

The GUI event reducer and transcript rendering pipeline correctly handles the primary event types but has four distinct mechanisms that swallow narrative text between tool calls, a cross-session state leak in `suppressNextCallCompletedSettle`, and unbounded memory growth in the transcript messages array.

**Severity:** P2 (narrative text loss, cross-session state leak, dedup key buffer limit)

## Files Involved

- `desktop/src/hooks/useRuntimeEventApplication.ts` — main event processing chain
- `desktop/src/hooks/useStreamingTranscript.ts` — streaming transcript state management
- `desktop/src/runtime/runtimeEventReducer.ts` — event-to-message reduction
- `desktop/src/runtime/runtimeEventViewModel.ts` — status/display mappings
- `desktop/src/runtime/streamSanitizer.ts` — markup stripping and content resolution
- `desktop/src/components/Transcript.tsx` — rendering and display windowing

## Key Findings

### Finding 1: Narrative text swallowed via terminalStreamClosedRef (P2)

After `model.stream_completed` triggers scheduleStreamCommit (220ms timer), `terminalStreamClosedRef` is set to `true`. Any `model.stream_delta` events before next `model.call_started` are dropped. If runtime emits narrative text as stream_delta between tool turns, it is silently discarded.

### Finding 2: suppressNextCallCompletedSettleRef leaks across sessions (P2)

When truncation occurs, `suppressNextCallCompletedSettleRef` is set to `true`. It is only consumed on next `model.call_completed`. If loop ends without another call, ref stays `true` and NEXT session's first call completion is incorrectly suppressed. Never reset on `session.state_changed`.

### Finding 3: Narrative dedup suppression (P2)

When `runtime.stream.narration` produces agent text via `appendMessage`, `isDuplicateAgentText` check compares against last agent text. If narration matches streaming content, narration is dropped. The narrator should be authoritative, not deduped.

### Finding 4: Unbounded transcript memory (P2)

`messages` state array grows unbounded. Only display-windowed (last 80 items) but all messages retained in memory. After 5000 events, `seenEventKeysRef` prunes from 5000 to 3000 entries, losing dedup protection.

### Finding 5: Markdown re-renders entire accumulated text (P3)

`applyStreamChunk` uses `requestAnimationFrame` batching, but `ReactMarkdown` re-parses ALL accumulated text on each state change, not just the delta. Computationally wasteful for long streams.

### Finding 6: Recoverable tool failures hidden from user (P3)

`tool.result_recorded` with recoverable observation prefixes triggers `markRecoverableToolFailureRecorded` which silently promotes failure to "completed" phase. Failures are invisible to user by design but violates transparency expectations.

### Finding 7: Sanitize carry buffer creates 1-chunk latency (P3)

`extractTrailingPotentialMarkup` holds up to 80 chars that match potential tool markup prefix. Visible text near markup boundaries is deferred by one chunk.

### Finding 8: No "Thinking" run status (P3)

`RunStatus` type has no "thinking" state. Reasoning is conveyed only via `ThinkingBlock` in transcript body, not in status bar. doc39 §3.2 expects "Thinking"/"Reasoning" indicator in status bar.

## doc39 Conflict

- **Yes** (§4.3.1): Narrative streaming should be gapless and model-authoritative; runtime.narration should override, not be deduped
- **Yes** (§6.1): Transcript storage expects capped size and automatic summarization
- **Yes** (§4.4): Stream session lifecycle requires refs to be reset on session boundaries
- **Yes** (§3.2): Status visibility expects "Thinking"/"Reasoning" indicator in status bar
- **Yes** (§4.5): All tool failures should be visible in transcript

## Suggested Fix

1. Reset `terminalStreamClosedRef` on `model.call_started` before checking, not after
2. Reset `suppressNextCallCompletedSettleRef` on `session.state_changed` and `model.call_started`
3. Prefer `runtime.stream.narration` content over streaming content in `appendMessage`
4. Add cap on `messages` array with older turns collapsed/summarized
5. Change dedup buffer from hard 5000 cap to time-based LRU or increase limit
6. Add `isThinking` status to `RunStatus` type
7. Increase dedup buffer limit or switch to LRU eviction

## Handoff Needed

- Runtime engineer: confirm whether `terminalStreamClosedRef` semantics are intentional or oversight
- Designer: confirm whether recoverable tool failures should be visible in transcript
