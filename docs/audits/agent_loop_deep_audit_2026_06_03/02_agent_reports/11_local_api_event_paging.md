Now I have all the data needed for the comprehensive audit. Let me compile the findings.

---

## Audit Report: Local API and Event Paging System

**Date:** 2026-06-03
**Files audited:**
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/local_api_server.rs`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/event_log.rs`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/session.rs`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/runtime_facade.rs`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/runtime_facade_impl.rs`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/runtime/session_store.rs`
- `/Users/gongyuxuan/Documents/deep-code/desktop/src/runtime/localRuntimeClient.ts`
- `/Users/gongyuxuan/Documents/deep-code/desktop/src/hooks/useRuntimeEventSubscription.ts`
- `/Users/gongyuxuan/Documents/deep-code/desktop/src/runtime/runtimeEventStream.ts`
- `/Users/gongyuxuan/Documents/deep-code/crates/kernel/src/lib.rs`

---

### Finding 1: Event streaming under high volume -- O(n) serialization per poll

**Conclusion:** Under high event volumes (1k, 5k, 10k events), `/runtime/stream-events` serializes the **entire event log** to a single JSONL string on every poll, even when the caller only needs a delta. This is a performance concern that grows linearly with total event count.

**Severity:** P2

**Files involved:**
- `runtime_facade_impl.rs` lines 185-225 (`stream_agent_events_since`)
- `session.rs` line 176 (`export_events_jsonl`)
- `event_log.rs` lines 79-86 (`export_jsonl`)

**Events involved:** All events in a session, on every poll.

**State involved:** The entire `SessionStore` mutex is held during the serialization and cursor-based slicing.

**Reproduction clues:** On a session with 10k events, every call to `stream_agent_events_since` internally calls `record.session.export_events_jsonl()` which iterates ALL events, serializes each to JSON, joins with newlines, returns a string. Then `all_jsonl.lines().collect::<Vec<_>>()` splits the entire string into lines, then the `cursor..end` range is sliced.

**Evidence fragments:** From `runtime_facade_impl.rs` lines 198-212:
```rust
let all_jsonl = record.session.export_events_jsonl();  // O(n) EVERY poll
let lines = all_jsonl.lines().collect::<Vec<_>>();       // allocates full vec
```

**Root cause:** The event log is an in-memory `Vec<KernelEvent>` which has no indexed access by sequence number. The only way to stream from a cursor is to serialize everything and split.

**Hidden risks:** Under very high volumes, the serialization of 10k+ events with large payloads (e.g., long model output deltas) could generate multi-megabyte strings on every poll. The `max_events` clamp of 1..200 limits the response but does NOT limit the serialization work. The `SessionStore` mutex is held during the entire export_jsonl call, blocking ALL other operations (approval, permission, export) on that session.

**doc39 conflict:** No

**Suggested fix:** Implement indexed access by sequence number on `EventLog`, avoiding full serialization on every poll. Maintain a pre-built JSONL string cache that is extended on each `append` rather than rebuilt from scratch.

**Not suggested:** Do not remove the mutex. The cursor-based approach is correct; the scope of work under the lock is the issue.

**Handoff needed:** No

---

### Finding 2: UTF-8 multi-byte character truncation at event boundaries

**Conclusion:** Multi-byte UTF-8 characters cannot be truncated at JSONL event boundaries. The newline delimiter (`0x0A`) is always a standalone ASCII byte that never appears inside a multi-byte UTF-8 sequence, and the JSON serializer in `event_log.rs` escapes actual `\n` characters to `\\n` in the event payload.

**Severity:** P3 (informational -- no vulnerability found)

**Files involved:**
- `event_log.rs` lines 79-86 (JSONL export joins with `\n`)
- `event_log.rs` lines 227-240 (`escape` function replaces `\n` with `\\n`)
- `runtime_facade_impl.rs` line 199 (`.lines()` split on newline)

**Events involved:** Any event with multi-byte UTF-8 content in its payload.

**State involved:** Event JSON serialization.

**Reproduction clues:** The UTF-8 encoding never embeds 0x0A in continuation bytes (those always have the high bit set, i.e., >= 0x80). Additionally, the `escape()` function explicitly replaces real newlines with the escaped `\\n` sequence before writing to JSONL. The `lines()` method in Rust splits on `\n` byte boundaries and always produces valid `&str`.

**Evidence fragments:** `event_log.rs` lines 231-233 match `'\n'` and replace it with `"\\n"`. The `export_jsonl()` method at line 83 appends `output.push('\n')` only as the delimiter between distinct events.

**Root cause:** N/A -- no issue exists. The design is sound.

**Hidden risks:** None identified on the Rust/server side. On the client side, the TypeScript `response.text()` + `JSON.parse()` path in `localRuntimeClient.ts` is equally safe.

**doc39 conflict:** No

**Suggested fix:** None required.

**Not suggested:** Do not change the JSONL newline delimiter strategy.

**Handoff needed:** No

---

### Finding 3: Cursor reliability under concurrent event appends

**Conclusion:** The cursor is reliable. Underneath, `stream_agent_events_since` locks the `SessionStore` mutex for the entire read, so the cursor-to-array mapping is atomic and consistent. Events are append-only, so cursor N always maps to the Nth event.

**Severity:** P3 (informational)

**Files involved:**
- `runtime_facade_impl.rs` lines 191-225
- `session_store.rs` lines 34-36 (Mutex)
- `runtime_facade_impl.rs` lines 247-272 (`ingest_native_loop_event_jsonl_line`)

**Events involved:** All events in a session.

**State involved:** `SessionStore.sessions: Mutex<HashMap<String, RuntimeSessionRecord>>`.

**Reproduction clues:** The `stream_agent_events_since` method locks `self.sessions.lock()` at line 193, holds it through the entire `export_events_jsonl()`, `lines()`, and slice operations, then drops it at method return. Meanwhile `ingest_agent_event_jsonl_line` at line 236 acquires the same mutex. This ensures no concurrent modification during cursor-based reads.

**Evidence fragments:** `session_store.rs` lines 34-36: `pub struct SessionStore { sessions: Mutex<HashMap<String, RuntimeSessionRecord>> }`. The `stream_agent_events_since` takes the raw `lock()` (not `with_ref`) to hold the lock across multiple operations.

**Root cause:** The design is sound -- cursor-based pagination with mutex-held reads.

**Hidden risks:** The cursor can fall behind significantly if the client does not poll frequently enough. The desktop client polls every 400ms (HTTP mode) or 1200ms (Tauri push mode), so drift is bounded. If the cursor falls behind by more than `usize::MAX`, overflow would occur, but this is not practically reachable.

**doc39 conflict:** No

**Suggested fix:** None required.

**Not suggested:** Do not relax the mutex. The atomicity is intentional.

**Handoff needed:** No

---

### Finding 4: Approval API can be blocked by event export operations

**Conclusion:** Yes, the approval/submission API can be blocked by event export, because all operations share the same `SessionStore` mutex.

**Severity:** P2

**Files involved:**
- `local_api_server.rs` lines 1293-1326 (`submit_permission_decision_when_ready`)
- `local_api_server.rs` lines 1445-1475 (`handle_export_events`)
- `runtime_facade_impl.rs` lines 1424-1431 (`export_events`)
- `session_store.rs` lines 34-36 (`Mutex`)

**Events involved:** Any event export operation.

**State involved:** `SessionStore` global mutex.

**Reproduction clues:** `export_events` calls `stream_agent_events` which calls `sessions.with_ref` which acquires the mutex, serializes ALL events to JSON, imports them back as validation, and writes to disk. During this time, the mutex is held, blocking `submit_permission_decision`, `submit_plan_decision`, `stream_agent_events_since`, and all other session operations. The `submit_permission_decision_when_ready` function retries up to 60 times with 50ms delays (3 seconds total), but a large export could exceed this.

**Evidence fragments:** `runtime_facade_impl.rs` lines 1424-1431: `export_events` calls `stream_agent_events(session_id)` which internally calls `with_ref(session_id, ...)` which acquires the mutex. The import + file write is a synchronous operation. `session_store.rs` lines 66-76: `with_ref` acquires and holds the mutex for the closure's duration.

**Root cause:** Single global mutex protecting all session state. A long-running export operation blocks all other operations on the same session.

**Hidden risks:** Under very high event volumes, export_events could take seconds to serialize, import-validate, and write to disk. During this window, the GUI cannot submit permission decisions, which would appear as a hung UI. The `submit_permission_decision_when_ready` retries for only ~3 seconds (60 * 50ms), so if export takes longer, the permission submission fails permanently.

**doc39 conflict:** Yes, with doc39 §14 which defines tool result formatting and by extension the expected latency for permission decision round-trips.

**Suggested fix:** Move the disk I/O out of the mutex-critical section. Re-validate after acquiring the file path, or perform the export on a cloned event log. Specifically, clone the session's event log reference under the lock, but perform the file write outside it.

**Not suggested:** Do not remove the mutex entirely or introduce a read-write lock without careful analysis of event ingestion paths.

**Handoff needed:** Yes, to a performance engineer to measure actual export latency at scale.

---

### Finding 5: Unbounded thread-per-connection model with single listener thread

**Conclusion:** The Rust server uses a single listener thread with `set_nonblocking(true)` and 100ms sleep polling, spawning one OS thread per accepted connection. There is no thread pool, connection limit, or backpressure mechanism.

**Severity:** P2

**Files involved:**
- `local_api_server.rs` lines 108-150 (listener loop)
- `local_api_server.rs` lines 136-138 (`thread::spawn` per connection)

**Events involved:** All incoming TCP connections.

**State involved:** OS thread scheduling.

**Reproduction clues:** The listener loop at line 130 iterates `listener.incoming()`. For each `Ok(stream)`, a new `thread::spawn` is created (line 138). There is no semaphore, thread pool, or connection limit. Under high concurrency (e.g., many rapid polls from the GUI plus export operations), this creates unbounded threads.

**Evidence fragments:** `local_api_server.rs` lines 136-138:
```rust
Ok(stream) => {
    let state = Arc::clone(&runtime_state);
    let root = static_root.clone();
    thread::spawn(move || handle_connection(stream, state, &root));
}
```

**Root cause:** The comment at the top of the file says "Uses only std::net::TcpListener + std::thread -- no async deps." The threading model is a deliberate simplicity choice rather than a production-grade design.

**Hidden risks:** Under sustained load, spawning thousands of threads could exhaust OS resources. Each connection handler holds a `TcpStream` read with a 30-second timeout and write with a 60-second timeout. A slow client that opens connections and reads slowly could cause thread exhaustion.

**doc39 conflict:** No direct conflict.

**Suggested fix:** Add a `ThreadPool` with a configurable max size (default 8-16). Alternatively, use `std::thread::available_parallelism` as a bound. This can be done while staying within `std` only, using a simple semaphore or a bounded channel.

**Not suggested:** Do not convert to async/tokio unless a broader async migration is planned.

**Handoff needed:** No

---

### Finding 6: Desktop client polling reconnection -- no missed events but no SSE support

**Conclusion:** The desktop client uses periodic HTTP polling (400ms interval for HTTP transport, 1200ms for Tauri push). It drains up to 25 pages per poll (paging through `has_more` responses with `next_cursor`). If a poll fails, it reports an error and retries on the next interval. There is no EventSource/SSE reconnection because the Rust server does not implement SSE (it explicitly chooses single JSON responses with `Connection: close`).

**Severity:** P3 (informational)

**Files involved:**
- `localRuntimeClient.ts` lines 340-379 (`streamRuntimeEvents`)
- `useRuntimeEventSubscription.ts` lines 114-172 (polling loop)
- `useRuntimeEventSubscription.ts` lines 174-183 (interval timer)
- `local_api_server.rs` lines 1016-1068 (handle_runtime_stream_events contract note)

**Events involved:** All runtime events.

**State involved:** `cursorRef` on the client side.

**Reproduction clues:** The polling loop at `useRuntimeEventSubscription.ts` line 124 iterates up to 25 pages `for (let page = 0; page < 25; page += 1)`. If any call fails (10s timeout), the entire poll is aborted and retried on next interval. The cursor ensures no events are missed between retries.

**Evidence fragments:** `useRuntimeEventSubscription.ts` lines 114-172: The `pollRuntimeEvents` function tracks `nextCursor` based on `streamed.next_cursor`. Events are drained in a paging loop until `!streamed.has_more`. The cursor is persisted in a ref and can survive component re-renders.

**Root cause:** The Rust server deliberately chooses not to implement SSE (see the contract note at `local_api_server.rs` lines 1010-1014: "The desktop GUI expects SSE via `new EventSource(url)` and will fall back to polling on error. This incompatibility is deliberate until SSE support is implemented").

**Hidden risks:** The maximum polling interval is 400ms for HTTP mode. With 25 pages per poll and 200 events per page, the client can drain up to 5000 events per poll cycle. However, each page is a separate HTTP request, creating burst traffic. The Rust server's thread-per-connection model means these 25 sequential requests each create a thread (though they are sequential, so at most one is active at a time per client). If the GUI is lost and reopens, the cursor resets to 0 and the client re-fetches all events from the beginning.

**doc39 conflict:** No

**Suggested fix:** Track the cursor in persistent storage (localStorage) to survive full page reloads. Consider adding a `sequence_endpoint` field or similar in the bootstrap response so the client can detect event log truncation events.

**Not suggested:** Do not force SSE on the Rust server without verifying that the thread-per-connection model can support long-lived SSE connections (it cannot -- each SSE connection would pin an OS thread indefinitely).

**Handoff needed:** No

---

### Finding 7: No maximum event response size -- single oversized event risk

**Conclusion:** There is no explicit maximum response body size for `/runtime/stream-events`. The `max_events` query parameter is clamped to 1..200, but a single event's `payload_json` can be arbitrarily large (e.g., a model stream delta with a multi-megabyte concatenated content preview). The response is sent via `write_all()` with a 60-second write timeout, but no chunked transfer encoding.

**Severity:** P2

**Files involved:**
- `local_api_server.rs` lines 1037-1040 (`max_events` clamp, but no per-event size limit)
- `local_api_server.rs` lines 514-545 (`write_response` with `Connection: close`, no `Content-Length`)
- `local_api_server.rs` lines 168-169 (read timeout 30s, write timeout 60s)

**Events involved:** Events with large payloads, particularly `model.stream_delta`, `tool.result_recorded`, and `assistant.text_delta`.

**State involved:** TCP send buffer.

**Reproduction clues:** If a single event's JSON serialization is, say, 50MB (a very long model response), the response body would be 50MB sent as a single `write_all()` call. The 60-second write timeout at line 169 would apply. If the TCP buffer cannot drain fast enough, the `write_all()` will fail with a timeout error, the connection is silently dropped (after `Shutdown::Write` at line 544), and the client receives a truncated response.

**Evidence fragments:** `local_api_server.rs` lines 1046-1061: The response builds `events_json` as a string from individual event serializations, then wraps it all in a single JSON response. There is no streaming or progressive encoding. `write_response` does not use `Content-Length` (intentionally, per the comment at lines 532-537), so the client reads until EOF. If the write fails partway, the client sees a truncated JSON document.

**Root cause:** No per-event size budget at the API response layer. The `event_line_to_http_json` function at line 1077 has a fallback for non-JSON events that truncates to 500 characters, but valid JSON events pass through without any size check.

**Hidden risks:** The `max_result_size_chars` in the tool catalog (20k-80k chars, line 1557) limits individual tool results, but model stream deltas can accumulate across multiple `model.stream_delta` events. The `export_jsonl()` function builds the full event log string in memory with no streaming, so even a single call to `stream_agent_events_since` can allocate a large string.

**doc39 conflict:** No

**Suggested fix:** Add a `max_response_bytes` parameter (default 10MB) to `stream_agent_events_since`. When the accumulated response exceeds this limit, stop adding events and set `has_more = true`. Also add per-event size tracking to reject individual events that exceed a threshold.

**Not suggested:** Do not switch to chunked transfer encoding until the connection model is reconsidered (chunked encoding requires keeping the connection open for multiple writes).

**Handoff needed:** No

---

### Finding 8: Python `local_api_server.py` status -- removed, not maintained

**Conclusion:** The Python `local_api_server.py` at `scripts/local_api_server.py` has been deleted from the working tree. It exists only in git history (last substantive version from commit `3b41a72a`). The Rust `local_api_server.rs` is the active and maintained implementation. Both are NOT maintained in parallel.

**Severity:** P3 (informational)

**Files involved:**
- `scripts/local_api_server.py` (git history only, 2337 lines at `3b41a72a`)
- `local_api_server.rs` (active, 2014 lines)

**Events involved:** N/A

**State involved:** N/A

**Reproduction clues:** `find ~/Documents/deep-code/scripts -name "local_api_server*"` returns no results. The file is absent from the filesystem. Git log shows the last commit affecting this file was `3b41a72a` "checkpoint: make mission frontend usable".

**Evidence fragments:** The Rust file header at line 1-3 states: "Sync HTTP/1.1 server for GUI/runtime integration. Replaces the Python `local_api_server.py` mock layer with real RuntimeFacade calls."

**Root cause:** The Python server was a stub/mock written during the initial GUI prototype phase. The Rust implementation replaced it with real RuntimeFacade calls as part of the native runtime convergence.

**Hidden risks:** If any deployment script or docker configuration still references `scripts/local_api_server.py`, it will fail. The file should be explicitly removed in git or flagged with a deprecation notice.

**doc39 conflict:** No

**Suggested fix:** Remove all references to `scripts/local_api_server.py` from any documentation, deployment scripts, or CI configurations. Confirm that the desktop client's `FALLBACK_BOOTSTRAP` (line 119-126 in `localRuntimeClient.ts`) points to the correct Rust server port (8765).

**Not suggested:** Do not resurrect the Python server. The Rust implementation is feature-complete and performance-superior.

**Handoff needed:** No, but the docs/scripts teams should clean up stale references.

---

### Summary of Findings

| Finding | Severity | Summary |
|---------|----------|---------|
| 1. O(n) serialization per poll | P2 | Entire event log re-serialized on every `stream_agent_events_since` call |
| 2. UTF-8 truncation safety | P3 (no issue) | Multi-byte chars cannot contain `\n` byte; JSON serializer escapes `\n` |
| 3. Cursor reliability | P3 (no issue) | Mutex ensures atomic cursor-to-array mapping; append-only log |
| 4. Approval blocked by export | P2 | Global `SessionStore` mutex blocks permission decisions during export |
| 5. Unbounded thread creation | P2 | Thread-per-connection, no pool or limit |
| 6. Client polling/reconnection | P3 (no issue) | 400ms polling with cursor; no events lost, but no SSE |
| 7. No max response size | P2 | No per-response size budget; single oversized event can exceed write timeout |
| 8. Python server status | P3 (info) | Python server removed from disk; Rust is the sole maintained implementation |