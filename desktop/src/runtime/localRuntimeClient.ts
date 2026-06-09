import type { RuntimeBootstrap } from "@/types/runtime";

export interface RuntimeEvent {
  event_id?: string;
  sequence?: number;
  event_type: string;
  payload?: Record<string, unknown>;
  created_at?: string;
}

export interface RuntimeSessionSnapshot {
  session_id: string;
  state: string;
  event_count: number;
  model_mode: string;
  autonomy_mode: string;
  workspace_root: string;
  pending_permission_count: number;
  pending_plan_approval_count: number;
}

export interface RuntimeStartSessionResult {
  session_id: string;
  task_id: string;
  workspace_root: string;
  model_mode: string;
  autonomy_mode: string;
  state: string;
}

export interface RuntimeStreamResult {
  session_id: string;
  from_cursor: number;
  next_cursor: number;
  has_more: boolean;
  events: RuntimeEvent[];
  jsonl?: string;
}

export interface PendingPermission {
  permission_id: string;
  request_type: string;
  tool_id: string;
  args_preview?: string;
  path_preview?: string;
  risk_level?: "low" | "medium" | "high" | "critical" | string;
}

export interface PendingPlanApproval {
  plan_approval_id: string;
  goal: string;
  plan_preview?: string;
}

export interface RuntimeConfigureProviderResult {
  ok: boolean;
  env_path: string;
  updated_keys: string[];
}

export interface RuntimeProviderHealthResult {
  ok: boolean;
  provider: "deepseek" | "qwen" | string;
  status: "healthy" | "unhealthy" | "skipped" | string;
  reason?: string;
  http_status_code?: number;
  target_kind?: string;
}

export interface RuntimeExportEventsResult {
  ok: boolean;
  session_id: string;
  path: string;
}

export interface RuntimeSessionRecordWriteResult {
  ok: boolean;
  path: string;
}

export interface RuntimePermissionDecisionResult {
  ok: boolean;
  session_id: string;
  error_code?: string;
  permission_id?: string;
  tool_call_id?: string;
  provider_tool_call_id?: string;
  tool_id?: string;
  resume_strategy?: string;
  tool_executed?: boolean;
  model_continuation_required?: boolean;
}

interface RuntimeEventEnvelope {
  session_id: string;
  event: RuntimeEvent;
  raw_jsonl?: string;
}

type TauriInvoke = <T = unknown>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

type TauriListen = <T = unknown>(
  event: string,
  callback: (event: { payload: T }) => void,
) => Promise<() => void>;

interface TauriBridge {
  core?: {
    invoke?: TauriInvoke;
  };
  event?: {
    listen?: TauriListen;
  };
}

const FALLBACK_BOOTSTRAP: RuntimeBootstrap = {
  transport: "http",
  baseUrl: "http://127.0.0.1:8765",
  token: "",
  workspaceRoot: ".",
  port: 8765,
  logPath: "",
};

function authHeaders(token: string): Record<string, string> {
  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (token) {
    headers.Authorization = `Bearer ${token}`;
  }
  return headers;
}

async function readJson<T>(response: Response): Promise<T> {
  const text = await response.text();
  const payload = text.trim() ? (JSON.parse(text) as T) : ({} as T);
  if (!response.ok) {
    const errorMessage =
      (payload as { error?: string; message?: string }).error ??
      (payload as { message?: string }).message ??
      `HTTP ${response.status}`;
    throw new Error(errorMessage);
  }
  return payload;
}

function toObjectRecord(value: unknown): Record<string, unknown> | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }
  return value as Record<string, unknown>;
}

function normalizeRuntimeEvent(event: Partial<RuntimeEvent> | undefined): RuntimeEvent {
  return {
    event_id: event?.event_id,
    sequence: typeof event?.sequence === "number" ? event.sequence : undefined,
    event_type: event?.event_type ? String(event.event_type) : "runtime.unknown",
    payload: toObjectRecord(event?.payload),
    created_at: event?.created_at ? String(event.created_at) : undefined,
  };
}

function tauriBridge(): TauriBridge | undefined {
  return window.__TAURI__;
}

function tauriInvoke(): TauriInvoke | undefined {
  return tauriBridge()?.core?.invoke;
}

function tauriListen(): TauriListen | undefined {
  return tauriBridge()?.event?.listen;
}

function hasTauriRuntime(): boolean {
  return typeof tauriInvoke() === "function";
}

async function invokeTauri<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const invoke = tauriInvoke();
  if (!invoke) {
    throw new Error("Tauri runtime bridge unavailable");
  }
  return await invoke<T>(command, args);
}

function isTauriBootstrap(bootstrap: RuntimeBootstrap): boolean {
  return bootstrap.transport === "tauri";
}

export async function resolveRuntimeBootstrap(): Promise<RuntimeBootstrap> {
  if (hasTauriRuntime()) {
    const payload = await invokeTauri<{
      workspace_root?: string;
      artifact_root?: string;
    }>("runtime_bootstrap");
    return {
      transport: "tauri",
      baseUrl: "",
      token: "",
      workspaceRoot: payload.workspace_root ?? ".",
      port: 0,
      logPath: payload.artifact_root ?? "",
    };
  }
  if (window.__ARGON_RUNTIME_BOOTSTRAP__) {
    const payload = window.__ARGON_RUNTIME_BOOTSTRAP__;
    return {
      ...payload,
      transport: payload.transport ?? "http",
    };
  }
  return FALLBACK_BOOTSTRAP;
}

export async function startRuntimeSession(
  bootstrap: RuntimeBootstrap,
  args: {
    workspace: string;
    model_mode: "deepseek" | "qwen";
    autonomy_mode?: string;
  },
): Promise<RuntimeStartSessionResult> {
  if (isTauriBootstrap(bootstrap)) {
    return normalizeStartSession(
      await invokeTauri<Partial<RuntimeStartSessionResult>>("runtime_start_session", {
        workspace: args.workspace,
        modelMode: args.model_mode,
        autonomyMode: args.autonomy_mode,
      }),
    );
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/start-session`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  const payload = await readJson<{ ok?: boolean; session?: RuntimeStartSessionResult }>(response);
  if (!payload.session) {
    throw new Error("runtime/start-session missing session payload");
  }
  return payload.session;
}

function normalizeStartSession(payload: Partial<RuntimeStartSessionResult>): RuntimeStartSessionResult {
  return {
    session_id: String(payload.session_id ?? ""),
    task_id: String(payload.task_id ?? ""),
    workspace_root: String(payload.workspace_root ?? "."),
    model_mode: String(payload.model_mode ?? "deepseek"),
    autonomy_mode: String(payload.autonomy_mode ?? "fast_auto"),
    state: String(payload.state ?? "Executing"),
  };
}

export async function submitRuntimeUserMessage(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string; text: string },
): Promise<{ ok: boolean; session_id: string; error_code?: string }> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<{
      ok?: boolean;
      session_id?: string;
      error_code?: string;
    }>("runtime_submit_user_message", {
      sessionId: args.session_id,
      text: args.text,
    });
    return {
      ok: payload.ok !== false,
      session_id: payload.session_id ?? args.session_id,
      error_code: payload.error_code,
    };
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/submit-user-message`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  return await readJson<{ ok: boolean; session_id: string; error_code?: string }>(response);
}

export async function interruptRuntimeSession(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string },
): Promise<{ ok: boolean; session_id: string; state?: string }> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<{ ok?: boolean; session_id?: string; state?: string }>(
      "runtime_interrupt_session",
      { sessionId: args.session_id },
    );
    return {
      ok: payload.ok !== false,
      session_id: payload.session_id ?? args.session_id,
      state: payload.state,
    };
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/interrupt-session`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  const payload = await readJson<{ ok?: boolean; session_id?: string; state?: string }>(response);
  return {
    ok: payload.ok !== false,
    session_id: payload.session_id ?? args.session_id,
    state: payload.state,
  };
}

export async function setRuntimeAutonomyMode(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string; autonomy_mode: string },
): Promise<{ ok: boolean; session_id: string; autonomy_mode?: string; state?: string }> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<{
      ok?: boolean;
      session_id?: string;
      autonomy_mode?: string;
      state?: string;
    }>("runtime_set_autonomy_mode", {
      sessionId: args.session_id,
      autonomyMode: args.autonomy_mode,
    });
    return {
      ok: payload.ok !== false,
      session_id: payload.session_id ?? args.session_id,
      autonomy_mode: payload.autonomy_mode,
      state: payload.state,
    };
  }
  return { ok: false, session_id: args.session_id };
}

export async function streamRuntimeEvents(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string; cursor: number },
): Promise<RuntimeStreamResult> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<Partial<RuntimeStreamResult>>("runtime_stream_events", {
      sessionId: args.session_id,
      cursor: args.cursor,
    });
    return {
      session_id: String(payload.session_id ?? args.session_id),
      from_cursor: Number(payload.from_cursor ?? args.cursor),
      next_cursor: Number(payload.next_cursor ?? args.cursor),
      has_more: Boolean(payload.has_more),
      events: Array.isArray(payload.events)
        ? payload.events.map((event) => normalizeRuntimeEvent(event as RuntimeEvent))
        : [],
      jsonl: String(payload.jsonl ?? ""),
    };
  }
  const query = new URLSearchParams({
    session_id: args.session_id,
    cursor: String(args.cursor),
  });
  const response = await fetch(`${bootstrap.baseUrl}/runtime/stream-events?${query.toString()}`, {
    headers: bootstrap.token ? { Authorization: `Bearer ${bootstrap.token}` } : {},
  });
  const payload = await readJson<RuntimeStreamResult>(response);
  return {
    ...payload,
    session_id: String(payload.session_id ?? args.session_id),
    from_cursor: Number(payload.from_cursor ?? args.cursor),
    next_cursor: Number(payload.next_cursor ?? args.cursor),
    has_more: Boolean(payload.has_more),
    events: Array.isArray(payload.events)
      ? payload.events.map((event) => normalizeRuntimeEvent(event))
      : [],
    jsonl: String(payload.jsonl ?? ""),
  };
}

export async function getRuntimeSnapshot(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string },
): Promise<RuntimeSessionSnapshot> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<Partial<RuntimeSessionSnapshot>>("runtime_get_snapshot", {
      sessionId: args.session_id,
    });
    return normalizeSnapshot(payload, args.session_id);
  }
  const query = new URLSearchParams({ session_id: args.session_id });
  const response = await fetch(`${bootstrap.baseUrl}/runtime/get-snapshot?${query.toString()}`, {
    headers: bootstrap.token ? { Authorization: `Bearer ${bootstrap.token}` } : {},
  });
  const payload = await readJson<
    Partial<RuntimeSessionSnapshot> | { snapshot?: Partial<RuntimeSessionSnapshot> }
  >(response);
  const wrappedPayload = payload as { snapshot?: Partial<RuntimeSessionSnapshot> };
  const snapshot = wrappedPayload.snapshot ?? (payload as Partial<RuntimeSessionSnapshot>);
  return normalizeSnapshot(snapshot, args.session_id);
}

function normalizeSnapshot(
  payload: Partial<RuntimeSessionSnapshot>,
  sessionId: string,
): RuntimeSessionSnapshot {
  return {
    session_id: String(payload.session_id ?? sessionId),
    state: String(payload.state ?? "Executing"),
    event_count: Number(payload.event_count ?? 0),
    model_mode: String(payload.model_mode ?? "deepseek"),
    autonomy_mode: String(payload.autonomy_mode ?? "fast_auto"),
    workspace_root: String(payload.workspace_root ?? "."),
    pending_permission_count: Number(payload.pending_permission_count ?? 0),
    pending_plan_approval_count: Number(payload.pending_plan_approval_count ?? 0),
  };
}

export async function submitRuntimePermissionDecision(
  bootstrap: RuntimeBootstrap,
  args: {
    session_id: string;
    permission_id: string;
    decision: "allow_once" | "allow_session" | "allow_project_rule" | "deny" | "modify";
    feedback?: string;
  },
): Promise<RuntimePermissionDecisionResult> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<Partial<RuntimePermissionDecisionResult>>(
      "runtime_submit_permission_decision",
      {
        sessionId: args.session_id,
        permissionId: args.permission_id,
        decision: args.decision,
        feedback: args.feedback ?? "",
      },
    );
    return {
      ok: payload.ok !== false,
      session_id: payload.session_id ?? args.session_id,
      error_code: payload.error_code ?? undefined,
      permission_id: payload.permission_id ?? args.permission_id,
      tool_call_id: payload.tool_call_id ?? undefined,
      provider_tool_call_id: payload.provider_tool_call_id ?? undefined,
      tool_id: payload.tool_id ?? undefined,
      resume_strategy: payload.resume_strategy ?? undefined,
      tool_executed: Boolean(payload.tool_executed),
      model_continuation_required: Boolean(payload.model_continuation_required),
    };
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/submit-permission-decision`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  const payload = await readJson<{ ok?: boolean; session_id?: string; error_code?: string }>(
    response,
  );
  const ok = payload.ok !== false;
  return {
    ok,
    session_id: payload.session_id ?? args.session_id,
    error_code: ok ? undefined : String(payload.error_code ?? "runtime_permission_decision_failed"),
    permission_id: args.permission_id,
    tool_executed: ok,
    model_continuation_required: ok,
  };
}

export async function submitRuntimePlanDecision(
  bootstrap: RuntimeBootstrap,
  args: {
    session_id: string;
    plan_approval_id: string;
    decision: "approve" | "request_revision";
    feedback?: string;
  },
): Promise<{ ok: boolean; session_id: string; error_code?: string; plan_approval_id?: string }> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<{
      ok?: boolean;
      session_id?: string;
      error_code?: string;
      plan_approval_id?: string;
    }>(
      "runtime_submit_plan_decision",
      {
        sessionId: args.session_id,
        planApprovalId: args.plan_approval_id,
        decision: args.decision,
        feedback: args.feedback,
      },
    );
    return {
      ok: payload.ok !== false,
      session_id: payload.session_id ?? args.session_id,
      error_code: payload.ok === false ? String(payload.error_code ?? "runtime_plan_decision_failed") : undefined,
      plan_approval_id: payload.plan_approval_id ?? args.plan_approval_id,
    };
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/submit-plan-decision`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  const payload = await readJson<{
    ok?: boolean;
    session_id?: string;
    error_code?: string;
    error?: string;
    plan_approval_id?: string;
  }>(response);
  const ok = payload.ok !== false;
  return {
    ok,
    session_id: payload.session_id ?? args.session_id,
    error_code: ok ? undefined : String(payload.error_code ?? payload.error ?? "runtime_plan_decision_failed"),
    plan_approval_id: payload.plan_approval_id ?? args.plan_approval_id,
  };
}

export async function subscribeRuntimeEvents(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string },
  onEvents: (events: RuntimeEvent[]) => void,
): Promise<() => void> {
  if (!isTauriBootstrap(bootstrap)) {
    return () => {};
  }
  const listen = tauriListen();
  if (!listen) {
    return () => {};
  }
  let bufferedEvents: RuntimeEvent[] = [];
  let flushTimer: ReturnType<typeof setTimeout> | null = null;
  const flushBufferedEvents = () => {
    if (flushTimer) {
      clearTimeout(flushTimer);
      flushTimer = null;
    }
    if (bufferedEvents.length === 0) {
      return;
    }
    const events = bufferedEvents;
    bufferedEvents = [];
    onEvents(events);
  };
  const unlisten = await listen<RuntimeEventEnvelope>("runtime://event", (event) => {
    const payload = event.payload;
    if (!payload || payload.session_id !== args.session_id) {
      return;
    }
    bufferedEvents.push(normalizeRuntimeEvent(payload.event));
    if (!flushTimer) {
      flushTimer = setTimeout(flushBufferedEvents, 48);
    }
  });
  return () => {
    flushBufferedEvents();
    unlisten();
  };
}

export async function configureRuntimeProvider(args: {
  provider: "deepseek" | "qwen";
  apiKey?: string;
  baseUrl?: string;
  modelId?: string;
}): Promise<RuntimeConfigureProviderResult | null> {
  if (!hasTauriRuntime()) {
    return null;
  }
  return await invokeTauri<RuntimeConfigureProviderResult>("runtime_configure_provider", {
    provider: args.provider,
    apiKey: args.apiKey ?? "",
    baseUrl: args.baseUrl ?? "",
    modelId: args.modelId ?? "",
  });
}

export async function healthCheckRuntimeProvider(args: {
  provider: "deepseek" | "qwen";
}): Promise<RuntimeProviderHealthResult | null> {
  if (!hasTauriRuntime()) {
    return null;
  }
  return await invokeTauri<RuntimeProviderHealthResult>("runtime_health_check_provider", {
    provider: args.provider,
  });
}

export async function markDesktopReady(): Promise<void> {
  if (!hasTauriRuntime()) {
    return;
  }
  await invokeTauri("desktop_mark_ready");
}

export async function exportRuntimeEvents(
  bootstrap: RuntimeBootstrap,
  args: { session_id: string; path?: string },
): Promise<RuntimeExportEventsResult> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<Partial<RuntimeExportEventsResult>>("runtime_export_events", {
      sessionId: args.session_id,
      path: args.path ?? "",
    });
    return {
      ok: payload.ok !== false,
      session_id: String(payload.session_id ?? args.session_id),
      path: String(payload.path ?? ""),
    };
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/export-events`, {
    method: "POST",
    headers: authHeaders(bootstrap.token),
    body: JSON.stringify(args),
  });
  return await readJson<RuntimeExportEventsResult>(response);
}

export async function listRuntimeCommands(bootstrap: RuntimeBootstrap): Promise<string[]> {
  if (isTauriBootstrap(bootstrap)) {
    const payload = await invokeTauri<{ commands?: string[] } | string[]>("runtime_list_commands");
    if (Array.isArray(payload)) {
      return payload.map((item) => String(item));
    }
    if (Array.isArray(payload?.commands)) {
      return payload.commands.map((item) => String(item));
    }
    return [];
  }
  const response = await fetch(`${bootstrap.baseUrl}/runtime/list-commands`, {
    headers: bootstrap.token ? { Authorization: `Bearer ${bootstrap.token}` } : {},
  });
  const payload = await readJson<{ commands?: string[] }>(response);
  return Array.isArray(payload.commands) ? payload.commands.map((item) => String(item)) : [];
}

export async function pickRuntimeProjectFolder(defaultPath?: string): Promise<string | null> {
  if (!hasTauriRuntime()) {
    return null;
  }
  const payload = await invokeTauri<{ path?: string | null }>("runtime_pick_project_folder", {
    defaultPath: defaultPath ?? "",
  });
  const path = String(payload?.path ?? "").trim();
  return path ? path : null;
}

export async function writeRuntimeSessionRecord(args: {
  workspaceRoot: string;
  runId: string;
  sessionId?: string | null;
  contentJson: string;
}): Promise<RuntimeSessionRecordWriteResult | null> {
  if (!hasTauriRuntime()) {
    return null;
  }
  const payload = await invokeTauri<Partial<RuntimeSessionRecordWriteResult>>(
    "runtime_write_session_record",
    {
      workspaceRoot: args.workspaceRoot,
      runId: args.runId,
      sessionId: args.sessionId ?? "",
      contentJson: args.contentJson,
    },
  );
  return {
    ok: payload.ok !== false,
    path: String(payload.path ?? ""),
  };
}

export async function revealRuntimePath(path: string): Promise<boolean> {
  if (!hasTauriRuntime()) {
    return false;
  }
  const payload = await invokeTauri<{ ok?: boolean }>("runtime_reveal_path", { path });
  return payload.ok !== false;
}
