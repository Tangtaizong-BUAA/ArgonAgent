export function buildUrl(baseUrl, path, params = {}) {
  const url = new URL(path, normalizeBaseUrl(baseUrl));
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined && value !== null && value !== "") {
      url.searchParams.set(key, String(value));
    }
  }
  return url.toString();
}

export function parseJsonl(text) {
  return text
    .trim()
    .split(/\n+/)
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

export function parseSseEvents(text) {
  return text
    .trim()
    .split(/\n\n+/)
    .filter(Boolean)
    .map((chunk) => {
      const event = {};
      for (const line of chunk.split(/\n/)) {
        const index = line.indexOf(":");
        if (index === -1) {
          continue;
        }
        const key = line.slice(0, index);
        const value = line.slice(index + 1).trimStart();
        event[key] = key === "data" ? JSON.parse(value) : value;
      }
      return event;
    });
}

export async function getHealth(fetchImpl, baseUrl) {
  return readJson(fetchImpl, buildUrl(baseUrl, "/health"));
}

export async function getLatestRun(fetchImpl, baseUrl) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/latest-run"));
  return payload.latest_run || null;
}

export async function getEvents(fetchImpl, baseUrl, eventPath) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/events", { path: eventPath }));
  return payload.events || [];
}

export async function getSummary(fetchImpl, baseUrl, eventPath, sessionId = undefined) {
  const payload = await readJson(
    fetchImpl,
    buildUrl(baseUrl, "/summary", { path: eventPath, session_id: sessionId }),
  );
  return payload.summary || null;
}

export async function getModelTimeline(fetchImpl, baseUrl, eventPath) {
  const payload = await readJson(
    fetchImpl,
    buildUrl(baseUrl, "/model-timeline", { path: eventPath }),
  );
  return payload.model_timeline || {
    call_count: 0,
    stream_count: 0,
    calls: [],
    streams: [],
  };
}

export async function getSessionSnapshot(fetchImpl, baseUrl, eventPath) {
  const payload = await readJson(
    fetchImpl,
    buildUrl(baseUrl, "/session-snapshot", { path: eventPath }),
  );
  return payload.session_snapshot || {
    event_count: 0,
    last_event_type: null,
    state: "Created",
    health: "empty",
    pending_permissions: [],
    pending_plan_approvals: [],
    counts: {},
    can_resume_without_user: false,
  };
}

export async function getRepoMap(fetchImpl, baseUrl, root = ".", maxFiles = 160, maxDepth = 4) {
  const payload = await readJson(
    fetchImpl,
    buildUrl(baseUrl, "/repo-map", {
      root,
      max_files: maxFiles,
      max_depth: maxDepth,
    }),
  );
  return payload.repo_map || {
    file_count: 0,
    omitted_count: 0,
    tech_stack: [],
    important_files: [],
    tree: [],
  };
}

export async function getToolCatalog(fetchImpl, baseUrl) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/tool-catalog"));
  return payload.tool_catalog || {
    version: "unknown",
    tool_count: 0,
    permission_required_count: 0,
    concurrency_safe_count: 0,
    categories: {},
    tools: [],
  };
}

export async function getApprovalQueue(fetchImpl, baseUrl, eventPath = undefined) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/approval-queue", { path: eventPath }));
  return payload.approval_queue || {
    pending_count: 0,
    pending: [],
    permissions: [],
    patches: [],
    tools: [],
    latest_pending: null,
  };
}

export async function getArtifacts(fetchImpl, baseUrl, artifactDir) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/artifacts", { dir: artifactDir }));
  return payload.artifacts || [];
}

export async function previewTool(fetchImpl, baseUrl, toolId, args = {}) {
  const payload = await readJson(fetchImpl, buildUrl(baseUrl, "/tool/preview"), {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ tool_id: toolId, arguments: args }),
  });
  if (payload.ok === false) {
    throw new Error(payload.error || "tool preview failed");
  }
  return payload.result;
}

export async function createPendingPackageFromSession(fetchImpl, baseUrl, args = {}) {
  const payload = await readJson(
    fetchImpl,
    buildUrl(baseUrl, "/native-loop/pending-package-from-session"),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ arguments: args }),
    },
  );
  if (payload.ok === false) {
    throw new Error(payload.error || "pending package creation failed");
  }
  return payload;
}

export function buildCommandCenterViewModel(events, summary = null, repoMap = null, toolCatalog = null) {
  const latestEvent = summary?.latest_event ?? events.at(-1) ?? null;
  const eventCount = summary?.event_count ?? events.length;
  const toolCounts = summary?.tool_counts ?? countByStatus(events);
  const modelCounts = summary?.model_counts ?? countModelCalls(events);
  const artifactCounts = summary?.artifact_counts ?? {};
  const modelTimeline = buildModelStreamTimeline(summary?.recent_events ?? events);
  const workflowPanels = buildWorkflowPanels(summary?.recent_events ?? events);
  return {
    eventCount,
    latestEventType: latestEvent?.event_type ?? "none",
    latestSequence: latestEvent?.sequence ?? 0,
    toolCounts,
    modelCounts,
    artifactCounts,
    modelTimeline,
    workflowPanels,
    repoMap,
    toolCatalog,
    recentEvents: summary?.recent_events ?? events.slice(-20),
  };
}

export function buildWorkflowPanels(events) {
  const tools = new Map();
  const permissions = new Map();
  const patches = new Map();
  const commands = [];
  for (const event of events) {
    const payload = event.payload ?? {};
    if (event.event_type === "tool.call.assembled") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        status: current.status ?? "assembled",
        assembledSequence: event.sequence,
        argumentsReplayable: payload.arguments_replayable === true,
        argumentsSummary: payload.arguments_summary ?? current.argumentsSummary,
        lifecycle: appendLifecyclePhase(current.lifecycle, "assembled", event, payload),
      });
    } else if (event.event_type === "tool.permission.evaluated") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        status: current.status ?? "permission_evaluated",
        permissionDecision: payload.decision,
        permissionMode: payload.mode,
        lifecycle: appendLifecyclePhase(current.lifecycle, "permission_evaluated", event, payload),
      });
    } else if (event.event_type === "tool.call_requested") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolCallId: payload.tool_call_id,
        toolId: payload.tool_id,
        status: "requested",
        sequence: event.sequence,
        lifecycle: appendLifecyclePhase(current.lifecycle, "requested", event, payload),
      });
    } else if (event.event_type === "tool.dispatched") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        status: "running",
        dispatcher: payload.dispatcher,
        dispatchedSequence: event.sequence,
        lifecycle: appendLifecyclePhase(current.lifecycle, "dispatched", event, payload),
      });
    } else if (event.event_type === "tool.call_completed") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        status: payload.ok === false ? "failed" : "completed",
        completedSequence: event.sequence,
        lifecycle: appendLifecyclePhase(
          current.lifecycle,
          payload.ok === false ? "failed" : "completed",
          event,
          payload,
        ),
      });
      if (payload.tool_id === "shell.command") {
        commands.push({
          toolCallId: payload.tool_call_id,
          status: payload.ok === false ? "failed" : "completed",
          sequence: event.sequence,
        });
      }
    } else if (event.event_type === "tool.completed") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        status: payload.ok === false ? "failed" : "completed",
        completedSequence: event.sequence,
        lifecycle: appendLifecyclePhase(
          current.lifecycle,
          payload.ok === false ? "failed" : "completed",
          event,
          payload,
        ),
      });
    } else if (event.event_type === "tool.result_recorded") {
      const current = tools.get(payload.tool_call_id) ?? { toolCallId: payload.tool_call_id };
      tools.set(payload.tool_call_id, {
        ...current,
        toolId: payload.tool_id ?? current.toolId,
        artifactId: payload.artifact_id,
        contentHash: payload.content_hash,
        preview: payload.preview,
        lifecycle: appendLifecyclePhase(current.lifecycle, "result_recorded", event, payload),
      });
    } else if (event.event_type === "permission.requested") {
      permissions.set(payload.permission_id, {
        permissionId: payload.permission_id,
        requestType: payload.request_type,
        status: "requested",
        sequence: event.sequence,
      });
    } else if (event.event_type === "permission.decided") {
      const current = permissions.get(payload.permission_id) ?? {
        permissionId: payload.permission_id,
      };
      permissions.set(payload.permission_id, {
        ...current,
        requestType: payload.request_type ?? current.requestType,
        status: payload.decision,
        decidedSequence: event.sequence,
      });
    } else if (event.event_type === "patch.proposal_created") {
      patches.set(payload.patch_id, {
        patchId: payload.patch_id,
        path: payload.path,
        status: "created",
        sequence: event.sequence,
      });
    } else if (event.event_type === "patch.proposal_validated") {
      const current = patches.get(payload.patch_id) ?? { patchId: payload.patch_id };
      patches.set(payload.patch_id, {
        ...current,
        validation: payload.validation,
        status: payload.validation === "pass" ? "validated" : "invalid",
        validatedSequence: event.sequence,
      });
    } else if (event.event_type === "patch.applied") {
      const current = patches.get(payload.patch_id) ?? { patchId: payload.patch_id };
      patches.set(payload.patch_id, {
        ...current,
        path: payload.path ?? current.path,
        status: "applied",
        appliedSequence: event.sequence,
      });
    }
  }
  return {
    tools: [...tools.values()],
    permissions: [...permissions.values()],
    patches: [...patches.values()],
    commands,
  };
}

function appendLifecyclePhase(phases = [], phase, event, payload) {
  return [
    ...phases,
    {
      phase,
      eventType: event.event_type,
      sequence: event.sequence,
      decision: payload.decision,
      mode: payload.mode,
      dispatcher: payload.dispatcher,
      ok: payload.ok,
    },
  ];
}

export function buildModelStreamTimeline(events) {
  const calls = new Map();
  const streams = new Map();
  for (const event of events) {
    const payload = event.payload ?? {};
    if (event.event_type === "model.call_started") {
      calls.set(payload.call_id, {
        callId: payload.call_id,
        provider: payload.provider,
        adapterId: payload.adapter_id,
        actualModelName: payload.actual_model_name,
        role: payload.role,
        live: payload.live === true,
        scaffold: {
          level: payload.scaffold_level,
          promptTokensEstimate: payload.prompt_tokens_estimate ?? 0,
          promptHash: payload.prompt_hash,
          toolCatalogHash: payload.tool_catalog_hash,
          maxContextTokens: payload.max_context_tokens ?? 0,
          promptScaffoldBudget: payload.prompt_scaffold_budget ?? 0,
          dynamicContextBudget: payload.dynamic_context_budget ?? 0,
          protectedReserveTokens: payload.protected_reserve_tokens ?? 0,
          budgetWarningCount: payload.budget_warning_count ?? 0,
        },
        status: "running",
      });
    } else if (event.event_type === "model.call_completed") {
      const current = calls.get(payload.call_id) ?? { callId: payload.call_id };
      calls.set(payload.call_id, {
        ...current,
        provider: payload.provider ?? current.provider,
        status: payload.ok === false ? "failed" : "completed",
        artifactId: payload.artifact_id,
        contentHash: payload.content_hash,
      });
    } else if (event.event_type === "model.call_blocked") {
      const current = calls.get(payload.call_id) ?? { callId: payload.call_id };
      calls.set(payload.call_id, {
        ...current,
        provider: payload.provider ?? current.provider,
        status: "blocked",
        gate: payload.gate,
      });
    } else if (event.event_type === "model.stream_delta") {
      const stream = ensureStream(streams, payload.stream_id, payload.provider);
      stream.deltas.push({
        kind: payload.delta_kind,
        preview: payload.preview,
        sequence: event.sequence,
      });
    } else if (event.event_type === "model.stream_completed") {
      const stream = ensureStream(streams, payload.stream_id, payload.provider);
      stream.status = "completed";
      stream.artifactId = payload.artifact_id;
      stream.contentHash = payload.content_hash;
      stream.tokens = {
        prompt: payload.prompt_tokens ?? 0,
        completion: payload.completion_tokens ?? 0,
        reasoning: payload.reasoning_tokens ?? 0,
        promptCacheHit: payload.prompt_cache_hit_tokens ?? 0,
        promptCacheMiss: payload.prompt_cache_miss_tokens ?? 0,
      };
    }
  }
  return {
    callCount: calls.size,
    streamCount: streams.size,
    calls: [...calls.values()],
    streams: [...streams.values()],
  };
}

async function readJson(fetchImpl, url, init = undefined) {
  const response = await fetchImpl(url, init);
  if (!response.ok) {
    throw new Error(`local api request failed: ${response.status}`);
  }
  return response.json();
}

function normalizeBaseUrl(baseUrl) {
  return baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
}

function countByStatus(events) {
  const counts = {};
  for (const event of events) {
    if (event.event_type !== "tool.call_completed") {
      continue;
    }
    const status = event.payload?.ok === false ? "failed" : "completed";
    counts[status] = (counts[status] ?? 0) + 1;
  }
  return counts;
}

function countModelCalls(events) {
  const counts = {};
  for (const event of events) {
    if (event.event_type !== "model.call_started") {
      continue;
    }
    const provider = event.payload?.provider ?? "unknown";
    counts[provider] = (counts[provider] ?? 0) + 1;
  }
  return counts;
}

function ensureStream(streams, streamId, provider) {
  const key = streamId ?? "unknown";
  if (!streams.has(key)) {
    streams.set(key, {
      streamId: key,
      provider,
      status: "running",
      deltas: [],
      tokens: {
        prompt: 0,
        completion: 0,
        reasoning: 0,
        promptCacheHit: 0,
        promptCacheMiss: 0,
      },
    });
  }
  return streams.get(key);
}
