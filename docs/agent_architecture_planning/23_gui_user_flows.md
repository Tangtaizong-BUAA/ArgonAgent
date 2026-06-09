# 23 GUI User Flows

This document specifies task flows, not page inventory. Each flow must be backed by Product Kernel events and APIs so that GUI design cannot drift into an untraceable chat surface.

## Common GUI Components

- Project switcher
- Session header with model profile and privacy mode
- Event timeline
- Plan panel
- Context/source panel
- Tool log panel
- Permission drawer
- Diff review panel
- Artifact preview panel
- Failure diagnosis panel

## Flow 1: Coding Task from GUI

### Screen Sequence

1. Home / Project list
2. Project dashboard
3. New task composer
4. Agent session view
5. Plan/context/log split view
6. Completion summary

### Runtime Events

- `project.opened`
- `session.created`
- `message.user_created`
- `session.state_changed: Planning`
- `context.bundle_created`
- `model.call_started`
- `model.call_completed`
- `session.state_changed: RetrievingContext`
- `tool.call_requested`
- `tool.call_completed`
- `session.completed` or failure state

### User Decisions

- Choose project.
- Choose native model mode: DeepSeek or Qwen3.6-27B.
- Choose permission policy: ask all, ask risky, read-only auto.
- Submit task.
- Optionally approve plan or tool actions.

### Failure States

- Project path inaccessible.
- Model profile not configured.
- Sensitive project policy blocks cloud model.
- Context retrieval fails.
- Agent asks for clarification.

### Required Backend APIs

- `POST /v0/projects/open`
- `POST /v0/sessions`
- `POST /v0/sessions/{id}/messages`
- `GET /v0/sessions/{id}/events`
- `GET /v0/artifacts/{id}`

## Flow 2: Approve Plan

### Screen Sequence

1. Agent session view enters plan state.
2. Plan approval panel opens.
3. User reviews steps, affected files/directories, expected commands, model profile.
4. User approves, edits, or rejects.

### Runtime Events

- `session.state_changed: WaitingForPlanApproval`
- `plan.proposed`
- `plan.approval_requested`
- `plan.approval_decided`
- `plan.approved` or `plan.rejected`
- `session.state_changed: RetrievingContext` or `WaitingForUser`

### User Decisions

- Approve plan as-is.
- Edit constraints, e.g. "do not touch package B".
- Reject and ask for a smaller plan.
- Switch model profile before execution.

### Failure States

- Plan omits verification.
- Plan asks for network/package install unexpectedly.
- Plan references nonexistent files.
- Plan conflicts with project policy.

### Required Backend APIs

- `GET /v0/sessions/{id}/events`
- `POST /v0/plan_approvals/{id}/decision`
- `POST /v0/sessions/{id}/messages`

### Governance vs Permission Note

Plan approval is task governance. It does not authorize shell commands, file writes, package installs, network access, cloud model calls, or protected paths. Those still require `PermissionRequest` and `PermissionDecision`.

## Flow 3: Approve Command

### Screen Sequence

1. Permission drawer appears over session view.
2. Command preview shows raw command, normalized command, parsed segments, working directory, environment changes, files/network effects, matched policy rule.
3. User chooses allow once, allow session rule, deny, or modify.
4. Tool log updates with execution status.

### Runtime Events

- `tool.call_requested`
- `permission.requested`
- `permission.decided`
- `session.state_changed: RunningCommand`
- `tool.call_completed`
- `artifact.created` for large output

### User Decisions

- Allow once.
- Allow for session scoped to exact command prefix.
- Deny and send reason to agent.
- Modify command, which becomes a new request.

### Failure States

- Command parser cannot normalize safely.
- Command includes dangerous shell segment.
- Command attempts package install.
- Command writes outside project.
- Command times out.

### Required Backend APIs

- `POST /v0/permissions/{id}/decision`
- `POST /v0/sessions/{id}/cancel`
- `GET /v0/artifacts/{id}`

## Flow 4: Review Diff

### Screen Sequence

1. Diff review panel opens.
2. File list shows additions/modifications/deletions and risk labels.
3. User reviews unified/side-by-side diff.
4. User applies, rejects, requests changes, or opens file context.

### Runtime Events

- `patch.proposed`
- `artifact.created` for rendered diff
- `permission.requested` with `request_type = file_write`
- `permission.decided`
- `patch.applied` or `patch.rejected`
- `session.state_changed: RunningCommand` if tests run

### User Decisions

- Apply all.
- Apply selected files if patch supports splitting.
- Reject with reason.
- Ask agent to revise patch.

### Failure States

- Base hash mismatch.
- Path protected.
- Patch ambiguous.
- Binary file diff unsupported.
- Formatter modifies extra files after apply.

### Required Backend APIs

- `GET /v0/artifacts/{diff_id}`
- `POST /v0/permissions/{id}/decision`
- `POST /v0/patches/{id}/apply`
- `POST /v0/sessions/{id}/messages`

## Flow 5: Reject Patch

### Screen Sequence

1. User clicks reject in diff review.
2. Reject reason dialog shows suggested structured reasons.
3. Session returns to execution/diagnosis state with rejection as context.

### Runtime Events

- `permission.decided: deny`
- `patch.rejected`
- `message.user_created` or `user.feedback_created`
- `session.state_changed: Executing`
- `model.call_started`

### User Decisions

- Reject because wrong file.
- Reject because too broad.
- Reject because style/API unacceptable.
- Reject and cancel task.

### Failure States

- Agent repeats same patch without addressing reason.
- Patch was already partially applied.
- Rejection reason conflicts with original task.

### Required Backend APIs

- `POST /v0/permissions/{id}/decision`
- `POST /v0/sessions/{id}/messages`
- `POST /v0/sessions/{id}/cancel`

## Flow 6: Repair Failed Build

### Screen Sequence

1. Command log shows failed build/test.
2. Failure diagnosis panel summarizes stderr, failing tests, touched files.
3. Agent proposes repair plan.
4. User reviews additional patch/command approvals.
5. Build reruns and result is summarized.

### Runtime Events

- `tool.call_completed` with nonzero exit
- `artifact.created` for full output
- `session.state_changed: DiagnosingFailure`
- `context.bundle_created` for failure context
- `model.call_started`
- `patch.proposed`
- `tool.call_requested`
- `session.completed` or `session.failed`

### User Decisions

- Let agent diagnose.
- Limit retry count.
- Deny risky repair command.
- Stop and export logs.

### Failure States

- Same failure repeats beyond retry budget.
- Repair requires package install.
- Output too large; summary loses key lines.
- Tests flaky.

### Required Backend APIs

- `GET /v0/artifacts/{output_id}`
- `POST /v0/sessions/{id}/messages`
- `POST /v0/permissions/{id}/decision`
- `POST /v0/sessions/{id}/cancel`

## Flow 7: Run Two Agents in Separate Worktrees

### Screen Sequence

1. Project dashboard.
2. Task board with "new isolated task".
3. Worktree/session setup modal.
4. Two agent session views side-by-side or tabs.
5. Merge review panel.

### Runtime Events

- `worktree.requested`
- `worktree.created`
- `session.created` for each task
- independent session event streams
- `merge.plan_proposed`
- `merge.conflict_detected` or `merge.applied`

### User Decisions

- Choose base branch/commit.
- Start tasks in separate worktrees.
- Pause/cancel one task.
- Merge one result, both, or neither.
- Resolve conflicts manually or ask agent.

### Failure States

- Git worktree creation fails.
- Two agents edit same file.
- Tests pass in worktree but fail after merge.
- Branch has uncommitted changes.

### Required Backend APIs

- Future API: `POST /v0/worktrees`
- `POST /v0/sessions`
- `GET /v0/sessions/{id}/events`
- Future API: `POST /v0/worktrees/{id}/merge_plan`

### Kernel Note

This is not Product Kernel v0. Kernel must first prove patch/event/permission invariants with single sessions.

## Flow 8: Research CSV Analysis

### Screen Sequence

1. Project dashboard.
2. Research workspace / data file selector.
3. Data profiling preview.
4. Job plan and privacy review.
5. Worker progress view.
6. Chart/report/artifact viewer.

### Runtime Events

- `research.job_drafted`
- `research.data_classified`
- `research.schema_profile_created`
- `permission.requested` if sensitive/cloud/package needed
- `research.script_proposed`
- `research.worker_started`
- `research.worker_progress`
- `artifact.created`
- `research.lineage_created`
- `research.job_completed`

### User Decisions

- Select files.
- Approve analysis plan.
- Approve or deny cloud model use for data summary.
- Approve package install if needed.
- Accept final report or ask revision.

### Failure States

- File too large for naive pandas read.
- Sensitive columns detected.
- Worker times out.
- Chart validation fails.
- Package missing.

### Required Backend APIs

- `POST /v0/research/jobs`
- `GET /v0/research/jobs/{id}`
- `GET /v0/sessions/{id}/events`
- `GET /v0/artifacts/{id}`
- `POST /v0/permissions/{id}/decision`

## Flow 9: Approve Cloud Model Use for Sensitive Data

### Screen Sequence

1. Privacy approval drawer opens.
2. Shows data classification summary, columns/samples requested, provider, model profile, retention note, and minimization strategy.
3. User allows once, allows masked summary only, switches to local/Qwen endpoint, or denies.

### Runtime Events

- `research.data_classified`
- `context.bundle_created` with sensitive items omitted
- `permission.requested` with `request_type = cloud_model`
- `permission.decided`
- `model.call_started` only after approval
- `model.call_completed`

### User Decisions

- Allow cloud call with masked sample.
- Allow profile/statistics only, no row samples.
- Switch to local model endpoint.
- Deny and request local-only analysis.

### Failure States

- Model call already started before approval. This is a critical bug.
- Classification confidence low.
- User cannot inspect what will be sent.
- Provider endpoint identity is unclear.

### Required Backend APIs

- `POST /v0/permissions/{id}/decision`
- `GET /v0/context_bundles/{id}/preview`
- `POST /v0/sessions/{id}/messages`

## Flow-Level Acceptance Criteria

1. Every visible state transition corresponds to an event-log event.
2. Every approval decision references a request hash.
3. Every diff/research artifact has a hash and source event id.
4. Every sensitive cloud call has an approval event before `model.call_started`.
5. Rejections and denials are fed back into the agent as context.
6. GUI can recover state solely from event log plus artifact store.
