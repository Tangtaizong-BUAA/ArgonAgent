# 21 Product Kernel v0

Product Kernel v0 is the smallest durable architecture core that must be correct before GUI richness, multi-agent orchestration, research workspace expansion, skills, automation, or team/cloud features. It is not an MVP feature list; it is the invariant substrate.

## Kernel Scope

Kernel v0 contains only:

1. Runtime API
2. Event Log
3. Agent State Machine
4. ToolSpec
5. Permission Request/Decision
6. ContextBundle
7. PatchProposal
8. Artifact Store
9. ModelAdapter
10. EvalEvent

Everything else must integrate through these interfaces.

## Non-Kernel Modules for Now

| Module | Reason It Is Not Kernel v0 | Allowed v0 Prototype? |
|---|---|---|
| GUI advanced board | UI richness can change without changing runtime invariants. | Thin session/log/approval UI only. |
| Multi-agent | Requires worktree isolation, scheduling, conflict policy; too much before kernel safety. | Manual two-session simulation only. |
| Worktree manager | Important, but patch/event/permission kernel must work first. | Prototype with git fixture after kernel. |
| Research workspace | Full workspace IA depends on Research Worker artifacts and privacy model. | Single ResearchJob flow only. |
| Skills | Powerful prompt/tool extension, high injection risk. | Disabled or static internal test skill only. |
| Automation | Scheduled unattended action needs mature permission policy. | None in kernel v0. |
| Team/cloud | Requires auth, sharing, policy admin, retention controls. | Architecture notes only. |
| MCP/plugins | External tool boundary is a major threat surface. | Disabled behind experimental gate. |

## Runtime API

### Responsibility

Expose a versioned local API for creating sessions, streaming events, submitting user messages, requesting tool execution, approving/denying permissions, applying patches, and querying artifacts/eval runs.

### Minimal Endpoints

| API | Input | Output | Notes |
|---|---|---|---|
| `POST /v0/projects/open` | path, policy | project id | Validates path and creates/opens per-project DB. |
| `POST /v0/sessions` | project id, model profile, task | session id | Emits `session.created`. |
| `GET /v0/sessions/{id}/events` | cursor | event stream | WebSocket/SSE compatible. |
| `POST /v0/sessions/{id}/messages` | user message | accepted event id | Starts or resumes loop. |
| `POST /v0/plan_approvals/{id}/decision` | approve/reject/request revision | decision event id | Task-governance decision, not a security permission. |
| `POST /v0/permissions/{id}/decision` | allow/deny, scope | decision event id | Must match pending request hash. |
| `POST /v0/patches/{id}/apply` | patch id, approval id | apply result | Validates base hash. |
| `POST /v0/sessions/{id}/cancel` | reason | cancel event | Cooperative cancellation. |
| `GET /v0/artifacts/{id}` | artifact id | metadata + file handle | Enforces privacy policy. |
| `POST /v0/evals/run` | eval case ids, profiles | eval run id | Runs fixture-based eval. |

### Invariants

- All mutating calls emit events.
- All API payloads have schema versions.
- Privileged operations require active project policy and local auth token.

## Event Log

### Responsibility

Provide the canonical append-only record for replay, audit, UI rendering, eval fixtures, and debugging.

### Event Envelope

```ts
type KernelEvent = {
  event_id: string
  schema_version: string
  project_id: string
  session_id?: string
  task_id?: string
  sequence: number
  event_type: string
  actor: "user" | "agent" | "runtime" | "tool" | "model" | "research_worker"
  created_at: string
  payload_json: unknown
  prev_hash?: string
  hash: string
}
```

### Required Event Types

- `session.created`
- `session.state_changed`
- `message.user_created`
- `model.call_started`
- `model.delta`
- `model.call_completed`
- `tool.call_requested`
- `permission.requested`
- `permission.decided`
- `tool.call_completed`
- `context.bundle_created`
- `plan.proposed`
- `plan.approval_requested`
- `plan.approval_decided`
- `patch.proposed`
- `patch.applied`
- `artifact.created`
- `eval.event`
- `session.completed`
- `session.failed`

## Agent State Machine

### Responsibility

Ensure agent execution is observable and interruptible.

### States

| State | Entry Condition | Allowed Actions | Exit Events |
|---|---|---|---|
| `Created` | Session inserted. | Accept task, select model. | `Planning`, `Cancelled` |
| `Planning` | User task available. | Model planning call, context read. | `WaitingForPlanApproval`, `RetrievingContext`, `Failed` |
| `WaitingForPlanApproval` | Task governance requires plan approval. | User approve/reject/request revision through PlanApprovalDecision. | `RetrievingContext`, `WaitingForUser`, `Cancelled` |
| `RetrievingContext` | Plan needs files/repo info. | read, rg, repo map. | `Executing`, `Failed` |
| `Executing` | Context bundle ready. | model call, tool call proposal. | `WaitingForToolApproval`, `ApplyingPatch`, `RunningCommand`, `Reviewing`, `Failed` |
| `WaitingForToolApproval` | Tool requires approval. | approve/deny/scope. | `RunningCommand`, `ApplyingPatch`, `Executing`, `WaitingForUser` |
| `ApplyingPatch` | Patch approved or auto-allowed. | validate base hash, apply. | `RunningCommand`, `Reviewing`, `DiagnosingFailure` |
| `RunningCommand` | Command approved. | execute command with timeout. | `Executing`, `DiagnosingFailure`, `Reviewing` |
| `DiagnosingFailure` | Tool/build/test failed. | collect failure context, model diagnosis. | `Executing`, `WaitingForUser`, `Failed` |
| `Reviewing` | Candidate result ready. | reviewer model/static checks. | `Completed`, `Executing`, `WaitingForUser` |
| `WaitingForUser` | Need input or blocked. | user message/decision. | previous resumable state, `Cancelled` |
| `Completed` | Success summary emitted. | read-only query/export. | none |
| `Failed` | Terminal failure. | read/export/retry new session. | none |
| `Cancelled` | User/runtime cancel. | read/export. | none |

## ToolSpec

### Responsibility

Define all model-callable tools in a typed, permission-aware registry.

```ts
type ToolSpec = {
  tool_id: string
  display_name: string
  version: string
  description: string
  input_schema: JsonSchema
  output_schema?: JsonSchema
  read_only: boolean | "dynamic"
  destructive: boolean | "dynamic"
  required_capabilities: string[]
  permission_policy: "always" | "never" | "dynamic"
  model_exposure: {
    deepseek: "native" | "hidden" | "wrapped"
    qwen: "native" | "hidden" | "wrapped"
  }
  output_budget: {
    max_inline_bytes: number
    spill_to_artifact: boolean
  }
}
```

### Invariants

- Tool ids are namespaced and stable.
- Model-facing schema bytes are deterministic per profile version.
- Tool output is capped and large output becomes an artifact.

## Permission Request / Decision

### Responsibility

Represent human or policy approval for shell, file writes, network, package install, cloud model data use, protected path access, and sensitive artifact export.

Plan approval is deliberately excluded from `PermissionRequest`. Plan approval is task governance; permission approval is a safety boundary. They can share a GUI drawer, but they must use separate event types and schemas.

```ts
type PermissionRequest = {
  permission_id: string
  session_id: string
  request_type: "command" | "file_write" | "network" | "package_install" | "cloud_model" | "protected_path" | "artifact_export"
  raw_request: unknown
  normalized_summary: string
  affected_paths: string[]
  data_privacy_classes: PrivacyClass[]
  risk_level: "low" | "medium" | "high" | "blocked"
  matched_policy_rules: string[]
  expires_at?: string
  request_hash: string
}

type PermissionDecision = {
  decision_id: string
  permission_id: string
  decision: "allow_once" | "allow_session" | "allow_project_rule" | "deny" | "modify"
  decided_by: "user" | "policy"
  scope?: unknown
  reason?: string
  request_hash: string
}
```

### Invariants

- Decision must reference exact request hash.
- GUI must render raw and normalized forms for risky actions.
- Deny decisions are also context for the model.
- Plan approval must not be represented as `request_type = "plan"`.

## PlanApproval Request / Decision

### Responsibility

Represent task-governance approval for an agent plan before execution. This is part of the Agent State Machine, not the Permission Manager.

```ts
type PlanApprovalRequest = {
  plan_approval_id: string
  session_id: string
  plan_id: string
  plan_summary: string
  planned_steps: PlanStep[]
  expected_paths: string[]
  expected_commands: string[]
  model_profile_id: string
  governance_risk: "low" | "medium" | "high"
  request_hash: string
}

type PlanApprovalDecision = {
  decision_id: string
  plan_approval_id: string
  decision: "approve" | "reject" | "request_revision"
  decided_by: "user" | "policy"
  revision_request?: string
  reason?: string
  request_hash: string
}
```

### Invariants

- Plan approval can authorize the task direction, but it never authorizes shell/file/network/package/cloud/protected-path actions.
- Any risky action inside an approved plan still requires `PermissionRequest`.
- Rejected or revised plans become context for the next planning turn.

## ContextBundle

### Responsibility

Control what enters the model call.

```ts
type ContextBundle = {
  bundle_id: string
  session_id: string
  model_profile_id: string
  purpose: "planning" | "execution" | "review" | "compaction" | "research"
  token_budget: number
  items: ContextItem[]
  omitted_items: ContextOmission[]
  privacy_summary: PrivacyClass[]
  prefix_hash?: string
}

type ContextItem = {
  item_id: string
  kind: "user_task" | "system_policy" | "repo_map" | "file_snippet" | "tool_result" | "plan" | "patch" | "memory" | "research_profile" | "artifact_ref"
  source_uri: string
  trust_level: "system" | "user" | "repo" | "generated" | "tool" | "external"
  content_ref?: string
  inline_text?: string
  token_estimate: number
  privacy_class: PrivacyClass
}
```

### Invariants

- Repo and generated content are never system instructions.
- DeepSeek mode tracks prefix stability.
- Qwen mode tracks parser/tool template version.

## PatchProposal

### Responsibility

Represent model-originated file changes before apply.

```ts
type PatchProposal = {
  patch_id: string
  session_id: string
  base_snapshot: {
    commit?: string
    file_hashes: Record<string, string>
  }
  target_paths: string[]
  intent: string
  hunks: PatchHunk[]
  rendered_diff_artifact_id: string
  risk_level: "low" | "medium" | "high"
  requires_approval: boolean
  validation: {
    base_hash_ok: boolean
    paths_allowed: boolean
    ambiguous: boolean
    conflicts: string[]
  }
}
```

### Invariants

- Apply fails on base hash mismatch.
- Creation/deletion paths are explicit.
- Formatter/test changes after patch are separate events.

## Artifact Store

### Responsibility

Store large or durable outputs with content hashes and privacy classification.

```ts
type ArtifactRecord = {
  artifact_id: string
  project_id: string
  session_id?: string
  kind: "diff" | "command_output" | "chart" | "report" | "notebook" | "script" | "data_profile" | "dataset" | "manifest" | "log"
  sha256: string
  size_bytes: number
  mime_type: string
  logical_name: string
  source_event_id: string
  privacy_class: PrivacyClass
  retention_policy: "keep" | "delete_on_close" | "manual_review" | "expires"
}
```

### Invariants

- No large artifact is embedded directly in model context.
- Every artifact can be traced to a source event.
- Sensitive artifacts require export approval.

## ModelAdapter

### Responsibility

Provide the minimum model interface required by native DeepSeek and Qwen modes.

```ts
type ModelAdapter = {
  adapter_id: string
  profile_id: "deepseek-v4-native" | "qwen3.6-27b-native"
  capabilities: {
    max_context_tokens: number
    supports_tools: boolean
    supports_reasoning_channel: boolean
    supports_prefix_cache_telemetry: boolean
    supports_streaming: boolean
  }
  buildPrompt(bundle: ContextBundle, tools: ToolSpec[]): ModelRequest
  parseStream(event: unknown): ModelDelta
  parseToolCalls(output: ModelOutput): ParsedToolCall[]
  classifyError(error: unknown): ModelErrorClass
}
```

### Invariants

- DeepSeek and Qwen have separate prompt builders and parsers.
- Generic OpenAI-compatible mode cannot override native profile behavior.
- Parser repairs are logged and evaled.

## EvalEvent

### Responsibility

Attach runtime behavior to reproducible eval cases.

```ts
type EvalEvent = {
  eval_run_id: string
  eval_case_id: string
  fixture_hash: string
  model_profile_id: string
  prompt_template_version: string
  parser_version: string
  event_id: string
  metric_name: string
  metric_value: number | string | boolean
  verdict?: "pass" | "fail" | "warning"
}
```

### Invariants

- Profile promotion requires eval run ids.
- Security failures are first-class eval metrics.
- Eval artifacts are immutable.

## Kernel v0 Acceptance Criteria

1. A coding session can be replayed from event log to final patch and test result.
2. A denied command is visible to the model as denial context without executing.
3. A stale patch is rejected deterministically.
4. A large shell output spills to artifact and gives the model a bounded summary.
5. DeepSeek and Qwen adapters can run the same fixture with different prompt/parser profiles.
6. Eval run records model profile version, parser version, and fixture hash.
