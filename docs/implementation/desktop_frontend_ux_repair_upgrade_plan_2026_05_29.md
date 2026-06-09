# Desktop Frontend UX Repair / Upgrade Plan - 2026-05-29

## Scope

This is a deep frontend/runtime UX audit for the active root `desktop/` app, not
the placeholder `apps/desktop/` scaffold. The focus is:

- approval and governance details;
- tool/function visibility and backend wiring;
- runtime state release and "looks stuck" behavior;
- frontend responsiveness and long-session rendering cost;
- testable repair slices.

No product code is changed by this document.

## Current Wiring Snapshot

- `desktop/README.md` defines the real desktop path as Tauri-first:
  `tauri` direct Rust `RuntimeFacade` invoke plus event stream, with HTTP only
  as browser debug fallback.
- `desktop/src/components/AppShell.tsx` owns almost all live session state:
  runtime bootstrap, session creation, event polling/push subscription,
  approval state, transcript state, token counters, local run persistence, and
  composer disablement.
- `desktop/src-tauri/src/main.rs` is the primary product bridge. It invokes
  `RuntimeFacade`, emits `runtime://event`, and continues sessions after plan
  or permission decisions.
- `scripts/local_api_server.py` and `crates/runtime/src/local_api_server.rs`
  are still fallback/debug bridges with behavior differences from the Tauri
  path.

## Highest Priority Findings

### P0-1. Permission approval blocks are not treated as full input blockers

Evidence:

- `BottomComposer` is disabled only when `!bootstrap || pendingPlanApprovals.length > 0`
  in `desktop/src/components/AppShell.tsx:2596-2613`.
- Pending tool permissions still leave the composer active, even though the
  runtime state can be `WaitingForToolApproval`.
- The backend has explicit `runtime_turn_in_progress` handling for a second
  prompt while a turn is active (`desktop/src/components/AppShell.tsx:1693-1703`),
  which means the UI can invite the user into an avoidable error path.

Repair:

- Treat `pendingPermissions.length > 0 || pendingPlanApprovals.length > 0` as
  a single `blockingApprovalActive` UI state.
- Disable composer, Continue, and Retry while any blocking approval is pending.
- Show the active blocker in the composer placeholder: "先处理权限审批" vs
  "先处理计划审批".
- Keep a small "open approvals" action so the user has one obvious next step.

Acceptance:

- A pending `permission.requested` makes typing/submitting impossible until a
  decision is submitted or the session is cancelled.
- A pending permission cannot produce a visible `runtime_turn_in_progress`
  caused by normal composer usage.

### P0-2. Approval content is too thin for safe decisions

Evidence:

- The floating permission banner hardcodes "需要 Shell 命令审批" even though the
  payload can be `file.write`, `patch.apply`, or other gated tools
  (`desktop/src/components/AppShell.tsx:2523-2534`).
- The right inspector shows only `tool_id`, `request_type`, and `permission_id`
  (`desktop/src/components/RightInspector.tsx:312-352`).
- The planned GUI flow requires command preview details: raw command, parsed
  segments, working directory, environment, file/network effects, and matched
  policy rule (`docs/agent_architecture_planning/23_gui_user_flows.md:109-146`).
- Current session permission events only carry `permission_id`, `request_type`,
  and optional `tool_id` (`crates/runtime/src/session.rs:343-366`).

Repair:

- Frontend quick fix: replace hardcoded Shell label with a generic risk-aware
  label from `tool_id` and `request_type`.
- Runtime contract fix: add an approval preview payload with:
  `tool_id`, `request_type`, `summary`, `cwd`, `paths`, `command_preview`,
  `network_intent`, `writes_files`, `request_hash`, and `policy_reason`.
- Display the same preview in the floating blocker and inspector tab.
- Keep PlanApproval and Permission separate: plan approval never authorizes
  shell/file/network/package actions.

Acceptance:

- File write, patch, shell, and package/network approvals render different
  labels and risk summaries.
- Every approval decision references a stable request hash.
- The user can decide without opening raw event logs.

### P0-3. Plan approval UX can degrade to a goal string

Evidence:

- `PlanApprovalBanner` falls back to `plan.plan_preview || plan.goal ||
  "计划内容等待 runtime 同步"` (`desktop/src/components/AppShell.tsx:2647-2705`).
- `plan.approval_requested` from `AgentSession::request_plan_approval` carries
  only `plan_approval_id` and `goal` (`crates/runtime/src/session.rs:299-318`).
- `AppShell` tries to recover `plan_preview` from `plan.mode_entered`, but it
  is best-effort only (`desktop/src/components/AppShell.tsx:1402-1428`).

Repair:

- Promote plan preview into the canonical `plan.approval_requested` payload:
  steps, affected areas, expected tools/commands, verification plan, denied
  paths, and rollback criteria.
- Keep the floating plan banner for the blocking state, but make the inspector
  the detailed review surface.
- Rename "取消" to "要求修改" to match the actual `request_revision` action.

Acceptance:

- A plan approval never shows only an opaque id or one-line goal unless the
  runtime explicitly reports a malformed/missing preview error.

### P0-4. Event vocabulary still has split-brain risk

Evidence:

- Frontend transcript/progress reacts to `tool.call_requested`,
  `tool.call_completed`, `tool.completed`, and `tool.result_recorded`
  (`desktop/src/components/AppShell.tsx:1277-1368`).
- Native turn controller emits ledger events `agent.tool.pending` and
  `agent.tool.completed` (`crates/runtime/src/native_turn_controller.rs:120-205`).
- Replay state counts canonical `tool.call_requested` and `tool.call_completed`
  (`crates/runtime/src/replay.rs:97-148`).

Repair:

- Add a small frontend event-normalization layer before `applyRuntimeEvents`:
  map `agent.tool.pending` to non-transcript diagnostic progress, and map
  `agent.tool.completed` to diagnostic completion unless a canonical
  `tool.call_completed` exists.
- Long term: keep `tool.call_*` as the user-visible canonical tool lifecycle;
  keep `agent.tool.*` as internal turn-ledger diagnostics.
- Add a fixture asserting both vocabularies can arrive without duplicate or
  missing progress rows.

Acceptance:

- Tool activity is visible even if a native turn emits only ledger events.
- Duplicate rows do not appear when both ledger and canonical events are present.

### P0-5. Async permission resume can look successful before it actually resumes

Evidence:

- Tauri `runtime_submit_permission_decision` returns `ok: true` immediately and
  resumes in a background task (`desktop/src-tauri/src/main.rs:322-424`).
- The real outcome is later emitted through synthetic
  `runtime.permission_submission.*` and `runtime.error` events
  (`desktop/src-tauri/src/main.rs:337-411`, `1180-1245`).
- Frontend appends "权限审批已提交，等待 runtime 恢复工具" after the immediate ack
  (`desktop/src/components/AppShell.tsx:1890-1909`).

Repair:

- Split approval state into `decision_submitted`, `runtime_accepted`,
  `tool_resumed`, and `continuation_started`.
- In the UI, immediate ack should say "已提交，等待运行时接收", not imply success.
- Add a watchdog: if no `permission.decided`,
  `runtime.permission_submission.accepted`, or `runtime.error` arrives within a
  fixed window, keep the approval visible with a retry/export diagnostic action.

Acceptance:

- The user can distinguish "click received" from "tool actually resumed".
- Missed synthetic events cannot leave the UI permanently spinning.

## Backend / Function Wiring Gaps

### HTTP fallback is not product-parity

Evidence:

- `scripts/local_api_server.py` identifies itself as a local API server stub
  (`scripts/local_api_server.py:1-34`).
- Its permission approval path executes a limited local tool and completes the
  session (`scripts/local_api_server.py:881-917`); for `shell.command` it falls
  back to `repo.map` arguments instead of real shell resume
  (`scripts/local_api_server.py:904-907`).
- Rust HTTP fallback returns JSON polling, not SSE, and still includes raw
  `jsonl` in the response (`crates/runtime/src/local_api_server.rs:762-816`).

Repair:

- Product label: make Tauri the only "real runtime" path in user-facing copy.
- Browser HTTP mode should show a "debug fallback" badge.
- Either remove real-chat claims from HTTP mode or make it invoke the same
  resume/continuation semantics as Tauri.
- Drop `jsonl` from HTTP polling responses unless explicitly requested with
  `include_jsonl=1`.

Acceptance:

- Smoke tests clearly separate `tauri-real-runtime` from `http-debug`.
- Browser debug cannot be mistaken for the product bridge.

### Several UI controls are visible but not connected deeply enough

Evidence:

- Artifact rows and "查看更改" render as buttons but do not open files/diffs
  (`desktop/src/components/RightInspector.tsx:394-418`,
  `desktop/src/components/Transcript.tsx:630-661`).
- The composer has a disabled microphone with "尚未接入本地 runtime"
  (`desktop/src/components/BottomComposer.tsx:287-294`).
- `Topbar` Continue/Retry remain generally enabled for any current run
  (`desktop/src/components/Topbar.tsx:68-92`).

Repair:

- Wire artifact open/export to Tauri commands.
- Wire diff open to a read-only diff panel backed by artifacts.
- Hide unfinished controls or move them behind disabled tooltips in a
  "coming later" area.
- Disable Continue/Retry while blocking approval is active.

Acceptance:

- Every visible command either works, has a precise blocked reason, or is not
  rendered.

## Frontend Speed / "Feels Stuck" Findings

### P1-1. State updates are too granular during event bursts

Evidence:

- `applyRuntimeEvents` loops through every event and calls `setMessages`,
  `setProgressItems`, `setTokenUsage`, and status setters inside the loop
  (`desktop/src/components/AppShell.tsx:1089-1536`).
- Push events are buffered for 48ms in `subscribeRuntimeEvents`
  (`desktop/src/runtime/localRuntimeClient.ts:436-475`), and adjacent stream
  deltas are coalesced (`desktop/src/components/AppShell.tsx:641-679`), but the
  post-coalesced event batch still performs many independent React state updates.

Repair:

- Convert event application to a reducer that consumes a batch and returns one
  next UI state object.
- Keep `model.stream_delta` on a separate lightweight streaming buffer so
  transcript/progress/sidebar do not re-render for every token group.
- Measure event batch size and render time in dev builds.

Acceptance:

- A 1,000-event replay applies in bounded chunks without freezing input.
- React commits per 100 runtime events drop sharply.

### P1-2. Polling heartbeat is safer than before but still expensive

Evidence:

- When a session is active, the frontend polls every 400ms normally and every
  1200ms even when Tauri push is active (`desktop/src/components/AppShell.tsx:2287-2298`).
- Each poll calls both `streamRuntimeEvents` and `getRuntimeSnapshot`
  (`desktop/src/components/AppShell.tsx:1590-1622`).

Repair:

- Adaptive polling:
  - 250-400ms while no push subscription exists;
  - 1000-1500ms heartbeat while running with push;
  - 3000-5000ms once completed/failed/waiting for approval;
  - immediate poll after submit/approval click.
- Skip snapshot if event batch included a terminal `session.state_changed`.
- Add `event_count` lightweight heartbeat before full event fetch when possible.

Acceptance:

- Push mode avoids continuous double round-trips.
- Missed final events are still recovered within about 1.5s.

### P1-3. Transcript paging is not true virtualization

Evidence:

- Transcript renders a fixed tail window of 80 messages with manual "show older"
  paging (`desktop/src/components/Transcript.tsx:202-245`).
- Completed agent messages are parsed through `ReactMarkdown` on render
  (`desktop/src/components/Transcript.tsx:567-628`).
- Previous P4 status already called out lack of transcript virtualization and a
  missing long-stream performance harness
  (`docs/runtime/p3_p4_completion_status_2026_05_19.md:121-156`).

Repair:

- Add real list virtualization for transcript rows.
- Cache markdown render by message id + content hash.
- Keep live stream rendering as plain text until finalized, then schedule
  markdown parse in idle time.
- Cap expanded tool groups and thinking blocks by default.

Acceptance:

- 500 messages and large markdown/code blocks remain scrollable.
- Markdown parsing does not run during token streaming.

### P1-4. Persistence work can compete with streaming

Evidence:

- Run state is serialized to localStorage after message/progress changes
  (`desktop/src/components/AppShell.tsx:2107-2175`).
- A second session mirror writes JSON through Tauri after message/progress
  changes (`desktop/src/components/AppShell.tsx:2177-2235`).
- During streaming, message content changes frequently via
  `requestAnimationFrame` (`desktop/src/components/AppShell.tsx:959-999`).

Repair:

- Persist only on stable boundaries: user message submitted, approval state
  changes, tool lifecycle event, stream completed, session state changed.
- Use `requestIdleCallback` where available, with a max-age flush.
- Store large transcript/event data in the runtime artifact/session store, not
  localStorage.

Acceptance:

- Long streaming does not continuously stringify the whole run snapshot.
- Restored sidebar/run history still works after refresh.

### P1-5. Event pagination is inconsistent

Evidence:

- Tauri `runtime_stream_events` fetches all events since cursor with no page
  size (`desktop/src-tauri/src/main.rs:204-221`).
- Rust HTTP local API clamps `max_events` to 1..200
  (`crates/runtime/src/local_api_server.rs:788-816`).
- The smoke script knows how to page `max_events=200`
  (`desktop/gui_three_round_smoke.mjs:769-785`).

Repair:

- Add `maxEvents` to Tauri `runtime_stream_events`.
- Frontend should drain pages until `has_more=false`, yielding between chunks.
- Treat cursor rewind or large catch-up as a background replay, not a single
  render-blocking operation.

Acceptance:

- A stale cursor cannot return an unbounded payload to the webview.

## Upgrade Roadmap

### Slice A - Approval UX and blocker correctness (P0)

Files:

- `desktop/src/components/AppShell.tsx`
- `desktop/src/components/RightInspector.tsx`
- `desktop/src/components/Topbar.tsx`
- `desktop/src/components/BottomComposer.tsx`
- runtime approval payload producers in `crates/runtime/src/session.rs` and
  Tauri DTO/event conversion if the contract is expanded.

Tasks:

1. Add `blockingApprovalActive` and use it to disable composer/Continue/Retry.
2. Replace hardcoded permission labels with tool/risk-aware labels.
3. Rename plan "取消" to "要求修改".
4. Add approval lifecycle states: submitted, accepted, resumed, failed.
5. Add frontend tests or GUI smoke probes for file-write, shell, and plan
   approval blockers.

### Slice B - Event normalizer and backend parity (P0/P1)

Files:

- `desktop/src/runtime/localRuntimeClient.ts`
- `desktop/src/components/AppShell.tsx`
- `desktop/src-tauri/src/main.rs`
- `crates/runtime/src/local_api_server.rs`
- `scripts/local_api_server.py` only if keeping browser debug parity.

Tasks:

1. Normalize `agent.tool.*` and `tool.call_*` before rendering.
2. Add Tauri event pagination.
3. Remove raw `jsonl` from normal frontend event responses.
4. Make HTTP fallback visibly debug-only or align its resume semantics.

### Slice C - Performance hardening (P1)

Files:

- `desktop/src/components/AppShell.tsx`
- `desktop/src/components/Transcript.tsx`
- `desktop/src/runtime/localRuntimeClient.ts`

Tasks:

1. Convert event application to a reducer/batched state update.
2. Add transcript virtualization and markdown render caching.
3. Move persistence to idle/stable-boundary writes.
4. Add adaptive polling and snapshot skipping.
5. Add long-stream/long-transcript GUI performance smoke.

### Slice D - Functional completion polish (P2)

Files:

- `desktop/src/components/RightInspector.tsx`
- `desktop/src/components/Transcript.tsx`
- `desktop/src/components/BottomComposer.tsx`
- Tauri commands for artifact/diff open.

Tasks:

1. Wire artifact buttons to open/export actions.
2. Wire "查看更改" to a diff panel.
3. Hide or properly explain unimplemented voice/input controls.
4. Add user-readable recovery actions for runtime errors: retry, export event
   log, open diagnostics.

## Test / Verification Matrix

Required after Slice A:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run build
npm run gui:incident-fixture
```

Required after Tauri bridge changes:

```bash
cd /Users/gongyuxuan/Documents/deep-code
cargo check --manifest-path desktop/src-tauri/Cargo.toml
```

Required after backend approval/resume changes:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run gui:real-runtime
```

Live-provider gated check, only when API keys/network are intentionally enabled:

```bash
cd /Users/gongyuxuan/Documents/deep-code/desktop
npm run gui:incident-live
```

New checks to add:

- `gui:approval-blockers`: plan approval, shell permission, file-write
  permission, deny path, missed-event recovery.
- `gui:long-stream-performance`: replay synthetic 5k events, 1k stream deltas,
  500 transcript messages; assert no UI timeout and no dense stream warning over
  threshold.
- `gui:event-vocabulary`: mixed `agent.tool.*` plus `tool.call_*` fixture.

## Risks

- Expanding permission/plan payloads touches event schema and Product Kernel
  surfaces. That needs an implementation TaskContract before code changes.
- Making HTTP fallback product-parity may duplicate Tauri work. Prefer marking
  it debug-only unless browser mode remains a product requirement.
- Transcript virtualization can subtly break scroll-to-bottom behavior during
  streaming; test this explicitly.
- Approval preview must avoid leaking secrets while still being specific enough
  for a safe decision.

## Recommended Next Task

Start with Slice A. It is the highest user-experience return: it fixes the
"why can I type while it needs approval?" confusion, removes misleading Shell
copy, makes plan/permission blockers explicit, and creates the right foundation
for the later performance work.
