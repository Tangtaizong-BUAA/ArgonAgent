# Agent 10: Active Turn / Cancel Audit

## Conclusion

The active turn lifecycle has correct structure on the happy path but contains a critical TOCTOU race condition between cancel cleanup and new-turn startup, plus a window where streaming model output can reach the GUI after the user has pressed stop.

**Severity:** P1 (TOCTOU race can corrupt session event ordering)

## Files Involved

- `desktop/src-tauri/src/main.rs` (260-378, 1382-1386) — active turn management, interrupt, release
- `crates/runtime/src/runtime_facade_impl.rs` (54-79, 149-174) — cancel_session, submit_user_message
- `crates/runtime/src/sidecar_http_transport.rs` (260-443) — run_sidecar_streaming_process
- `crates/runtime/src/native_agent_loop_model_io.rs` (466-571) — observer function
- `crates/runtime/src/local_api_server.rs` (78-83, 694-801, 927) — local API turn management

## Key Findings

### Finding 1: TOCTOU Race — "next question not responding" (P1)

Two scenarios cause the symptom:

**Scenario A — Cancel not yet complete:** User presses stop then immediately submits next question. `runtime_submit_user_message` acquires `active_turns` lock before `cancel_session` sets state to Cancelled. Sees non-terminal state → rejects with `runtime_turn_in_progress`.

**Scenario B — Active turn registration clobbered by old task:**
1. Old turn running; `active_turns = {S}`
2. `runtime_interrupt_session`: cancel_session sets Cancelled; `release_active_turn` clears active_turns
3. User submits new question; active_turns empty → inserts S; spawns blocking task B
4. Old blocking task A finishes, calls `release_active_turn(S)` → `active_turns = {}`
5. Blocking task B is running but ACTIVE_TURNS IS EMPTY → third submission can start concurrent turn for same session

### Finding 2: "Stop flash" — 250ms interrupt polling gap (P2)

`run_sidecar_streaming_process` polls interrupt flag every 250ms. Between flag set and child kill, streaming data arrives and is forwarded to GUI via observer (which has no interrupt check). Model output appears after cancel indicator.

### Finding 3: Active turn released on ALL paths (P3)

All completion paths call `release_active_turn` unconditionally. This is correct in isolation but causes the race in Finding 1 — old task's unconditional release can clobber new task's registration.

### Finding 4: No generation counter on active_turns (P1)

`active_turns: Mutex<HashSet<String>>` uses simple set membership. No generation counter or epoch number to prevent stale release from old task.

### Finding 5: Window close handler incomplete (P2)

`on_window_event(CloseRequested)` only calls `interrupt()` — does NOT call `cancel_session` or `release_active_turn`. No cleanup on window close.

### Finding 6: Permission decision retry loop contention (P2)

`submit_permission_decision_when_ready` spins for up to 3s waiting for turn to go inactive. If turn goes inactive then new turn starts (due to race), permission decision can be submitted against wrong turn's session state.

### Finding 7: `spawn_continue_with_prompt` polls 100 times (P3)

5 second total poll time with 50ms sleep. If cancel delays old task release, continue can fail with `runtime_turn_still_active_after_permission_resume`.

## doc39 Conflict

- **Yes** (§4.5.1): All session mutations must be guarded by single lock — violated by stale release
- **Yes** (§7.2): GUI layer must not release turn belonging to different lifecycle epoch

## Suggested Fix

1. Replace `HashSet<String>` active_turns with `HashMap<String, u64>` generation counter — `release_active_turn` only clears if caller's generation matches stored generation
2. Add interrupt check inside observer function in `send_with_live_visible_stream_events`
3. Reduce sidecar polling interval from 250ms to 50ms or use event-driven approach
4. Add `cancel_session` + `release_active_turn` to window close handler

## Handoff Needed

- GUI team: verify frontend waits for `runtime_interrupt_session` response before enabling "next question" button
- Testing team: add stress test that submits new question immediately after cancel
