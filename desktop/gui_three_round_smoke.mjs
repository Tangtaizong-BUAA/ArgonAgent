#!/usr/bin/env node
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import fs from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const DESKTOP_DIR = path.dirname(__filename);
const REPO_ROOT = path.resolve(DESKTOP_DIR, "..");

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find((arg) => arg.startsWith(prefix))?.slice(prefix.length) || "";
}

const args = new Set(process.argv.slice(2));
const headed = args.has("--headed");
const keepOpen = args.has("--keep-open");
const realDialogue = args.has("--real-dialogue");
const liveProvider = args.has("--live-provider");
const forbidLoopBudget = args.has("--forbid-loop-budget") || liveProvider;
const incidentVerify = args.has("--incident-verify");
const incidentFixtureOnly = args.has("--incident-fixture-only");
const incidentLiveOnly = args.has("--incident-live-only");
const incidentStrictTelemetry = args.has("--incident-strict-telemetry");
const incidentFixtureArg = argValue("incident-fixture");
const backend = argValue("backend") || "rust";
const roundsArg = argValue("rounds");
const promptArgs = process.argv
  .filter((arg) => arg.startsWith("--prompt="))
  .map((arg) => arg.slice("--prompt=".length));
let workspaceArg = argValue("workspace");
const providerArg = argValue("provider") || (incidentVerify ? "deepseek" : "qwen");
const autonomyModeArg = argValue("autonomy-mode") || "conservative";
const expectedToolsArg = argValue("expect-tools") || "";
const expectedToolIdsArg = expectedToolsArg
  .split(",")
  .map((tool) => canonicalToolId(tool.trim()))
  .filter(Boolean);
const expectedWrittenPathArg = argValue("expect-written-path") || "";
const requestedRounds = Math.max(1, Number(roundsArg || promptArgs.length || (incidentVerify ? 2 : 3)));
const started = Date.now();

function envSeconds(name, fallback) {
  const value = Number(process.env[name] || "");
  return Number.isFinite(value) && value > 0 ? value : fallback;
}

const providerIdleTimeoutSeconds = envSeconds(
  "RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS",
  envSeconds("RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS", 60),
);
const liveSessionSettleTimeoutMs = Math.min(
  600_000,
  Math.max(90_000, Math.ceil(providerIdleTimeoutSeconds * 1000) + 45_000),
);

const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", runId);
await fs.mkdir(artifactDir, { recursive: true });

const defaultIncidentFixturePath = path.join(
  REPO_ROOT,
  ".researchcode",
  "runtime_desktop",
  "runtime_session_1778982897464665000",
  "events",
  "runtime_events.jsonl",
);
const incidentFixturePath = incidentFixtureArg || (incidentLiveOnly ? "" : defaultIncidentFixturePath);
if (incidentVerify && !incidentFixtureOnly && !workspaceArg) {
  workspaceArg = path.join(artifactDir, "workspace");
  await fs.mkdir(workspaceArg, { recursive: true });
  await fs.writeFile(
    path.join(workspaceArg, "README.md"),
    [
      "# GUI Runtime Incident Probe",
      "",
      "This workspace is created by gui_three_round_smoke.mjs --incident-verify.",
      "It is safe for file.write and shell approval regression checks.",
      "",
    ].join("\n"),
    "utf8",
  );
}

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];
const apiResponses = [];
const runtimeHttpErrors = [];
let invalidJsonResponseCount = 0;

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[gui-smoke +${elapsed}ms] ${message}`);
}

function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

async function findFreePort() {
  return await new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      const port = typeof address === "object" && address ? address.port : 0;
      server.close(() => resolve(port));
    });
  });
}

function httpGetJson(url, headers = {}, timeoutMs = 10_000) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, { headers }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => {
        body += chunk;
      });
      res.on("end", () => {
        const json = parseJson(body);
        if (!json) {
          invalidJsonResponseCount += 1;
          const filename = `invalid_response_${String(invalidJsonResponseCount).padStart(2, "0")}.json`;
          fs.writeFile(path.join(artifactDir, filename), body).catch(() => {});
          reject(new Error(`invalid JSON from ${url}: ${body.slice(0, 160)}; saved=${filename}`));
          return;
        }
        resolve({ status: res.statusCode || 0, json });
      });
    });
    req.on("error", reject);
    req.setTimeout(timeoutMs, () => {
      req.destroy(new Error(`timeout fetching ${url}`));
    });
  });
}

async function waitForHttp(url, label, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      const result = await httpGetJson(url, {}, 500);
      if (result.status >= 200 && result.status < 500) {
        return result.json;
      }
      lastError = `status ${result.status}`;
    } catch (error) {
      lastError = String(error.message || error);
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error(`${label} did not become ready: ${lastError}`);
}

async function waitForWeb(url, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      await new Promise((resolve, reject) => {
        const req = http.get(url, (res) => {
          res.resume();
          if ((res.statusCode || 0) >= 200 && (res.statusCode || 0) < 500) {
            resolve();
          } else {
            reject(new Error(`status ${res.statusCode}`));
          }
        });
        req.on("error", reject);
        req.setTimeout(3000, () => req.destroy(new Error("timeout")));
      });
      return;
    } catch (error) {
      lastError = String(error.message || error);
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error(`${label} did not become ready: ${lastError}`);
}

function spawnLogged(label, command, commandArgs, options = {}) {
  const stdio = options.pipeStdin ? ["pipe", "pipe", "pipe"] : ["ignore", "pipe", "pipe"];
  const { pipeStdin, ...spawnOptions } = options;
  const child = spawn(command, commandArgs, {
    stdio,
    ...spawnOptions,
  });
  childProcesses.push({ label, child });
  child.stdout.on("data", (chunk) => {
    const text = chunk.toString().trim();
    if (text) {
      log(`${label}: ${text.split("\n").slice(-2).join(" | ")}`);
    }
  });
  child.stderr.on("data", (chunk) => {
    const text = chunk.toString().trim();
    if (text) {
      log(`${label} stderr: ${text.split("\n").slice(-2).join(" | ")}`);
    }
  });
  child.on("exit", (code, signal) => {
    if (!keepOpen) {
      log(`${label} exited code=${code} signal=${signal || ""}`);
    }
  });
  return child;
}

async function cleanup(browser) {
  if (keepOpen) {
    log("--keep-open set; leaving browser and servers running.");
    return;
  }
  await browser?.close().catch(() => {});
  for (const { child } of childProcesses.reverse()) {
    if (!child.killed) {
      child.kill("SIGTERM");
    }
  }
}

function textIncludesAny(text, needles) {
  return needles.some((needle) => text.toLowerCase().includes(needle.toLowerCase()));
}

function truncate(value, max = 500) {
  const text = String(value ?? "");
  return text.length > max ? `${text.slice(0, max)}...truncated` : text;
}

function isBenignRuntimeStreamFailure(entry) {
  const url = String(entry?.url || "");
  const failure = String(entry?.failure || "");
  return url.includes("/runtime/stream-events") && /ERR_CONNECTION_RESET|ERR_ABORTED/i.test(failure);
}

function runtimeEndpoint(url) {
  const match = String(url || "").match(/\/runtime\/[^?]+/);
  return match ? match[0] : String(url || "");
}

function isBenignRuntimeHttpError(entry) {
  const endpoint = runtimeEndpoint(entry?.url);
  const status = Number(entry?.status || 0);
  const code = String(entry?.json?.error_code || entry?.json?.error || "");
  return (
    status === 400 &&
    endpoint === "/runtime/interrupt-session" &&
    /InvalidTransition|already.*terminal|not.*executing|no active/i.test(code)
  );
}

function classifyConsoleMessages(messages, ignoredRuntimeHttpErrors) {
  const hasIgnored400 = ignoredRuntimeHttpErrors.some((entry) => Number(entry.status) === 400);
  const ignored = [];
  const blocking = [];
  for (const message of messages) {
    const text = String(message?.text || "");
    if (
      hasIgnored400 &&
      /Failed to load resource: the server responded with a status of 400 \(Bad Request\)/i.test(text)
    ) {
      ignored.push({
        ...message,
        reason: "matched_ignored_runtime_http_400",
      });
      continue;
    }
    if (/Failed to load resource: net::ERR_CONNECTION_RESET/i.test(text)) {
      ignored.push({
        ...message,
        reason: "runtime_stream_closed_during_test_shutdown",
      });
      continue;
    }
    blocking.push(message);
  }
  return { ignored, blocking };
}

function withTimeout(promise, timeoutMs, label) {
  let timeoutId;
  const timeout = new Promise((_, reject) => {
    timeoutId = setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timeoutId));
}

function notableEvents(events) {
  return events
    .filter((event) =>
      [
        "model.call_started",
        "model.call_completed",
        "model.call_blocked",
        "model.http_failure_recovered",
        "model.transport_failure_recovered",
        "agent.loop_budget_reached",
        "agent.loop_recovery",
        "runtime.error",
        "runtime.write_intent_fallback",
        "assistant.message",
        "tool.call_requested",
        "tool.call_completed",
        "permission.requested",
        "permission.decided",
      ].includes(event.event_type),
    )
    .slice(-8)
    .map((event) => ({
      sequence: event.sequence,
      event_type: event.event_type,
      payload: truncate(JSON.stringify(event.payload ?? {}), 700),
    }));
}

function validateExpectedToolSequence(roundEvents, expectedToolIds) {
  if (!expectedToolIds || expectedToolIds.length === 0) {
    return;
  }
  let cursor = 0;
  const completed = roundEvents.filter((event) => event.event_type === "tool.call_completed");
  for (const expectedToolId of expectedToolIds) {
    const foundAt = completed.findIndex((event, index) => {
      if (index < cursor) {
        return false;
      }
      return canonicalToolId(event.payload?.tool_id) === expectedToolId && event.payload?.ok === true;
    });
    if (foundAt < 0) {
      const observed = completed
        .map((event) => `${event.sequence}:${canonicalToolId(event.payload?.tool_id)}:${event.payload?.ok}`)
        .join(", ");
      throw new Error(`expected tool sequence missing ${expectedToolId}; observed=[${observed}]`);
    }
    cursor = foundAt + 1;
  }
}

function validateRoundEvents(roundEvents, expectedToolId = "", allowWriteFallback = false, expectedToolIds = []) {
  const disabledToolsFinalAnswerMarker = ["disable_tools", "and_request", "final", "answer"].join("_");
  const legacyNoToolTerminalEvent = ["agent", "loop", "finalizer"].join(".");
  const forbiddenNoToolTerminal = roundEvents.find((event) => {
    const payloadText = JSON.stringify(event.payload ?? {});
    return (
      event.event_type === legacyNoToolTerminalEvent ||
      payloadText.includes("Tools are disabled") ||
      payloadText.includes(disabledToolsFinalAnswerMarker)
    );
  });
  if (forbiddenNoToolTerminal) {
    throw new Error(`forbidden no-tool terminal observed: ${JSON.stringify(forbiddenNoToolTerminal)}`);
  }
  const writeFallback = roundEvents.find((event) => event.event_type === "runtime.write_intent_fallback");
  if (expectedToolId === "file.write" && writeFallback && !allowWriteFallback) {
    throw new Error(`file.write was satisfied by runtime fallback instead of model tool call: ${JSON.stringify(writeFallback)}`);
  }
  const budgetFallback = roundEvents.find((event) => event.event_type === "agent.loop_budget_reached");
  if (forbidLoopBudget && budgetFallback) {
    throw new Error(`agent loop budget reached during GUI live run: ${JSON.stringify(budgetFallback)}`);
  }
  if (expectedToolId === "file.write" && budgetFallback) {
    throw new Error(`file.write round reached loop budget after tool execution: ${JSON.stringify(budgetFallback)}`);
  }
  validateExpectedToolSequence(roundEvents, expectedToolIds);
}

const INCIDENT_EXPECTED_TELEMETRY_EVENTS = [
  "agent.executor.role_call",
  "agent.compactor.role_call",
  "deepseek.cache.zone_a.hit",
  "deepseek.cache.zone_a.miss",
  "deepseek.cache.zone_b.hit",
  "deepseek.cache.zone_b.miss",
  "deepseek.tool_call.partial",
  "deepseek.tool_call.assembled",
  "deepseek.dsml.leak",
  "context.compaction.started",
  "context.compaction.completed",
  "context.compaction.blocked",
  "deepseek.role_split.flash_savings",
];

function canonicalToolId(toolId) {
  return String(toolId || "").replace(/_/g, ".");
}

function payloadText(event) {
  return JSON.stringify(event?.payload ?? {});
}

function textHasShellPermissionRequired(text) {
  return /PermissionRequired/.test(text) && /shell\.command/.test(text);
}

function countBy(items, keyFn) {
  const counts = new Map();
  for (const item of items) {
    const key = keyFn(item);
    counts.set(key, (counts.get(key) || 0) + 1);
  }
  return Object.fromEntries([...counts.entries()].sort((a, b) => String(a[0]).localeCompare(String(b[0]))));
}

function eventTypes(events) {
  return new Set(events.map((event) => event.event_type).filter(Boolean));
}

async function readJsonlEvents(filePath) {
  const text = await fs.readFile(filePath, "utf8");
  const events = [];
  for (const [index, line] of text.split(/\r?\n/).entries()) {
    if (!line.trim()) {
      continue;
    }
    try {
      events.push(JSON.parse(line));
    } catch (error) {
      throw new Error(`${filePath}:${index + 1}: invalid JSON: ${error.message || error}`);
    }
  }
  return events;
}

function missingFieldsFromIssues(issues) {
  return [...new Set(
    (Array.isArray(issues) ? issues : [])
      .map((issue) => String(issue).match(/missing required field ([A-Za-z0-9_]+)/)?.[1] || "")
      .filter(Boolean),
  )].sort();
}

function validationFailureSignature(event) {
  const payload = event.payload ?? {};
  const missing = missingFieldsFromIssues(payload.issues);
  const issueText = Array.isArray(payload.issues) ? payload.issues.map(String).sort().join("|") : payloadText(event);
  return [
    canonicalToolId(payload.tool_id),
    missing.length > 0 ? `missing:${missing.join(",")}` : issueText,
  ].join("::");
}

function analyzeRepeatedValidationFailures(events) {
  const counts = new Map();
  const samples = new Map();
  let previousSignature = "";
  let currentRun = 0;
  let maxConsecutiveFailureRun = { signature: "", count: 0, sequences: [] };

  for (const event of events) {
    if (event.event_type !== "tool.validation_failed") {
      continue;
    }
    const signature = validationFailureSignature(event);
    const count = (counts.get(signature) || 0) + 1;
    counts.set(signature, count);
    if (!samples.has(signature)) {
      samples.set(signature, {
        signature,
        tool_id: canonicalToolId(event.payload?.tool_id),
        issues: event.payload?.issues || [],
        first_sequence: event.sequence,
      });
    }
    if (signature === previousSignature) {
      currentRun += 1;
    } else {
      currentRun = 1;
      previousSignature = signature;
    }
    if (currentRun > maxConsecutiveFailureRun.count) {
      const sample = samples.get(signature) || {};
      maxConsecutiveFailureRun = {
        ...sample,
        count: currentRun,
        sequences: events
          .filter((candidate) => candidate.event_type === "tool.validation_failed")
          .filter((candidate) => validationFailureSignature(candidate) === signature)
          .map((candidate) => candidate.sequence)
          .slice(0, currentRun),
      };
    }
  }

  const repeated = [...counts.entries()]
    .filter(([, count]) => count >= 2)
    .map(([signature, count]) => ({
      ...(samples.get(signature) || { signature }),
      count,
    }))
    .sort((a, b) => b.count - a.count);

  return { repeated, maxConsecutiveFailureRun };
}

function analyzePromptAudit(events) {
  const continuations = new Set(
    events
      .filter((event) => event.event_type === "model.continuation_strategy")
      .map((event) => event.payload?.call_id)
      .filter(Boolean),
  );
  const modelCalls = events.filter((event) => event.event_type === "model.call_started");
  const unknownContinuationCalls = modelCalls.filter((event, index) => {
    const payload = event.payload ?? {};
    const isContinuation = index > 0 || continuations.has(payload.call_id);
    if (!isContinuation) {
      return false;
    }
    return (
      !payload.prompt_hash ||
      payload.prompt_hash === "unknown" ||
      !payload.tool_catalog_hash ||
      payload.tool_catalog_hash === "unknown" ||
      Number(payload.prompt_tokens_estimate || 0) <= 0
    );
  });
  return {
    model_call_count: modelCalls.length,
    continuation_call_count: continuations.size,
    unknown_continuation_calls: unknownContinuationCalls.map((event) => ({
      sequence: event.sequence,
      call_id: event.payload?.call_id,
      prompt_hash: event.payload?.prompt_hash,
      tool_catalog_hash: event.payload?.tool_catalog_hash,
      prompt_tokens_estimate: event.payload?.prompt_tokens_estimate,
    })),
  };
}

function analyzeTurnIds(events) {
  const turnIds = [
    ...new Set(
      events
        .map((event) => event.payload?.turn_id)
        .filter(Boolean),
    ),
  ];
  return {
    unique_turn_ids: turnIds,
    only_native_turn_0:
      turnIds.length === 1 && String(turnIds[0]).includes("native_turn_0"),
  };
}

function analyzeStreamDensity(events) {
  const streamDeltas = events.filter((event) => event.event_type === "model.stream_delta");
  const counts = countBy(streamDeltas, (event) => event.payload?.stream_id || "unknown");
  const sorted = Object.entries(counts)
    .map(([stream_id, count]) => ({ stream_id, count }))
    .sort((a, b) => b.count - a.count);
  return {
    total_stream_delta_count: streamDeltas.length,
    max_stream_delta_count: sorted[0]?.count || 0,
    densest_streams: sorted.slice(0, 5),
  };
}

function analyzePermissionSemantics(events) {
  const permissionRequests = events
    .filter((event) => event.event_type === "permission.requested")
    .map((event) => ({
      sequence: event.sequence,
      permission_id: event.payload?.permission_id,
      request_type: event.payload?.request_type,
      tool_id: event.payload?.tool_id,
    }));
  const permissionDecisions = events
    .filter((event) => event.event_type === "permission.decided")
    .map((event) => ({
      sequence: event.sequence,
      permission_id: event.payload?.permission_id,
      decision: event.payload?.decision,
    }));
  const decidedIds = new Set(permissionDecisions.map((item) => item.permission_id).filter(Boolean));
  const unresolvedRequests = permissionRequests.filter((item) => !decidedIds.has(item.permission_id));
  const stalePermissionErrors = events
    .filter((event) => event.event_type === "runtime.error")
    .filter((event) => /no pending permission/i.test(payloadText(event)))
    .map((event) => ({ sequence: event.sequence, payload: event.payload }));
  const shellPermissionFailures = events
    .filter((event) => {
      const text = payloadText(event);
      return (
        String(event.payload?.tool_id || "") === "shell.command" &&
        textHasShellPermissionRequired(text)
      );
    })
    .map((event) => ({
      sequence: event.sequence,
      event_type: event.event_type,
      preview: event.payload?.preview,
    }));
  const permissionRequiredReadyWithoutRequest = [];
  for (let index = 0; index < events.length; index += 1) {
    const event = events[index];
    if (
      event.event_type !== "tool.mediation.completed" ||
      event.payload?.permission_required !== true ||
      event.payload?.status !== "ready"
    ) {
      continue;
    }
    const toolId = canonicalToolId(event.payload?.resolved_tool || event.payload?.requested_tool);
    const window = events.slice(index + 1, index + 12);
    const hasPermissionRequest = window.some(
      (candidate) =>
        candidate.event_type === "permission.requested" &&
        canonicalToolId(candidate.payload?.tool_id) === toolId,
    );
    const completedAsToolError = window.some(
      (candidate) =>
        candidate.event_type === "tool.result_recorded" &&
        canonicalToolId(candidate.payload?.tool_id) === toolId &&
        textHasShellPermissionRequired(payloadText(candidate)),
    );
    if (!hasPermissionRequest && completedAsToolError) {
      permissionRequiredReadyWithoutRequest.push({
        sequence: event.sequence,
        tool_id: toolId,
      });
    }
  }
  return {
    permission_request_count: permissionRequests.length,
    permission_decision_count: permissionDecisions.length,
    permission_requests: permissionRequests,
    permission_decisions: permissionDecisions,
    unresolved_permission_requests: unresolvedRequests,
    stale_permission_errors: stalePermissionErrors,
    shell_permission_failures: shellPermissionFailures,
    permission_required_ready_without_request: permissionRequiredReadyWithoutRequest,
  };
}

function assessRuntimeIncidentEvents(events, { mode = "live", strictTelemetry = false } = {}) {
  const types = eventTypes(events);
  const repeatedValidation = analyzeRepeatedValidationFailures(events);
  const promptAudit = analyzePromptAudit(events);
  const turnAudit = analyzeTurnIds(events);
  const streamDensity = analyzeStreamDensity(events);
  const permissionAudit = analyzePermissionSemantics(events);
  const recoveryEvents = events.filter((event) => event.event_type === "agent.loop_recovery");
  const recoveryEscalations = events.filter((event) => event.event_type === "agent.recovery.escalated");
  const missingTelemetryEvents = INCIDENT_EXPECTED_TELEMETRY_EVENTS.filter((type) => !types.has(type));
  const failures = [];
  const warnings = [];

  if (mode === "live") {
    if (
      repeatedValidation.maxConsecutiveFailureRun.count >= 3 &&
      recoveryEscalations.length === 0
    ) {
      failures.push(
        `same validation failure repeated ${repeatedValidation.maxConsecutiveFailureRun.count} times without agent.recovery.escalated`,
      );
    }
    if (permissionAudit.shell_permission_failures.length > 0) {
      failures.push("shell.command permission requirement escaped as a tool_result_error");
    }
    if (permissionAudit.permission_required_ready_without_request.length > 0) {
      failures.push("permission_required tool reached ready/executed path without permission.requested");
    }
    if (permissionAudit.stale_permission_errors.length > 0) {
      failures.push("runtime emitted no pending permission after GUI approval");
    }
    if (permissionAudit.unresolved_permission_requests.length > 0) {
      failures.push("permission.requested remained unresolved after the live approval probe");
    }
    if (promptAudit.unknown_continuation_calls.length > 0) {
      failures.push("continuation model.call_started events lost prompt/tool catalog audit hashes");
    }
    if (strictTelemetry && missingTelemetryEvents.length > 0) {
      failures.push(`missing incident telemetry events: ${missingTelemetryEvents.join(", ")}`);
    }
    if (streamDensity.max_stream_delta_count > 1200) {
      warnings.push(`dense model.stream_delta stream observed: ${streamDensity.max_stream_delta_count}`);
    }
    if (turnAudit.only_native_turn_0 && promptAudit.model_call_count > 1) {
      warnings.push("all model calls stayed in native_turn_0; role/stage split may be inactive");
    }
  }

  return {
    mode,
    event_count: events.length,
    event_type_counts: countBy(events, (event) => event.event_type || "unknown"),
    model_call_count: promptAudit.model_call_count,
    tool_call_requested_count: events.filter((event) => event.event_type === "tool.call_requested").length,
    recovery_count: recoveryEvents.length,
    recovery_escalation_count: recoveryEscalations.length,
    repeated_validation_failures: repeatedValidation.repeated,
    max_repeated_validation_failure: repeatedValidation.maxConsecutiveFailureRun,
    prompt_audit: promptAudit,
    turn_audit: turnAudit,
    stream_density: streamDensity,
    permission_audit: permissionAudit,
    missing_telemetry_events: missingTelemetryEvents,
    failures,
    warnings,
  };
}

function assertIncidentFixtureIsUseful(assessment, fixturePath) {
  const missingExpectations = [];
  if (assessment.event_count < 10_000) {
    missingExpectations.push(`expected >=10000 events, got ${assessment.event_count}`);
  }
  if (assessment.model_call_count < 10) {
    missingExpectations.push(`expected >=10 model calls, got ${assessment.model_call_count}`);
  }
  if ((assessment.max_repeated_validation_failure?.count || 0) < 8) {
    missingExpectations.push("did not detect the file.edit repeated schema failure burst");
  }
  if (assessment.prompt_audit.unknown_continuation_calls.length < 1) {
    missingExpectations.push("did not detect unknown continuation prompt/tool hashes");
  }
  if (assessment.permission_audit.shell_permission_failures.length < 1) {
    missingExpectations.push("did not detect shell PermissionRequired tool-result leak");
  }
  if (assessment.permission_audit.permission_request_count < 1) {
    missingExpectations.push("did not detect the final file.write permission boundary");
  }
  if (assessment.missing_telemetry_events.length < 8) {
    missingExpectations.push("did not detect broad missing incident telemetry");
  }
  if (missingExpectations.length > 0) {
    throw new Error(`incident fixture did not exercise expected failures: ${fixturePath}; ${missingExpectations.join("; ")}`);
  }
}

async function analyzeIncidentFixture(filePath) {
  const events = await readJsonlEvents(filePath);
  const assessment = assessRuntimeIncidentEvents(events, { mode: "fixture" });
  assertIncidentFixtureIsUseful(assessment, filePath);
  return assessment;
}

function requestedApproxLineCount(prompt) {
  const match = String(prompt || "").match(/(\d+)\s*(?:行|lines?)/i);
  return match ? Number(match[1]) : 0;
}

function isToolInventoryPrompt(prompt) {
  const text = String(prompt || "").toLowerCase();
  return /工具|tools?|tooling/.test(text) && /测试|尝试|拥有|所有|可用|available|inventory|catalog|test tools|try tools/.test(text);
}

function resolveWorkspacePath(workspaceRoot, candidate) {
  const cleaned = String(candidate || "").replace(/^`|`$/g, "").trim();
  if (!cleaned) {
    return "";
  }
  const resolved = path.isAbsolute(cleaned) ? path.resolve(cleaned) : path.resolve(workspaceRoot, cleaned);
  const workspace = path.resolve(workspaceRoot);
  if (resolved !== workspace && !resolved.startsWith(`${workspace}${path.sep}`)) {
    throw new Error(`written artifact escapes workspace: ${resolved}`);
  }
  return resolved;
}

function writtenPathFromRoundEvents(roundEvents, workspaceRoot) {
  const result = [...roundEvents].reverse().find(
    (event) =>
      event.event_type === "tool.result_recorded" &&
      String(event.payload?.tool_id || "") === "file.write" &&
      String(event.payload?.preview || "").includes("file.write wrote"),
  );
  const preview = String(result?.payload?.preview || "");
  const match = preview.match(/\bto\s+(.+?)(?:\s+rollback=|$)/);
  return match ? resolveWorkspacePath(workspaceRoot, match[1]) : "";
}

async function validateExpectedWrittenPath(workspaceRoot, expectedWrittenPath) {
  if (!expectedWrittenPath) {
    return null;
  }
  const writtenPath = resolveWorkspacePath(workspaceRoot, expectedWrittenPath);
  const content = await fs.readFile(writtenPath, "utf8");
  return {
    path: writtenPath,
    line_count: content.length === 0 ? 0 : content.split(/\r\n|\r|\n/).length,
  };
}

async function validateWrittenArtifact({ roundEvents, prompt, workspaceRoot, expectedWrittenPath = "" }) {
  const explicitArtifact = await validateExpectedWrittenPath(workspaceRoot, expectedWrittenPath);
  if (explicitArtifact) {
    return explicitArtifact;
  }
  const expectedLines = requestedApproxLineCount(prompt);
  if (!expectedLines || !workspaceRoot) {
    return null;
  }
  const writtenPath = writtenPathFromRoundEvents(roundEvents, workspaceRoot);
  if (!writtenPath) {
    throw new Error("file.write round did not expose a written artifact path in tool.result_recorded preview");
  }
  const content = await fs.readFile(writtenPath, "utf8");
  const lineCount = content.length === 0 ? 0 : content.split(/\r\n|\r|\n/).length;
  const minLines = Math.max(1, Math.floor(expectedLines * 0.65));
  const maxLines = Math.ceil(expectedLines * 1.6);
  if (lineCount < minLines || lineCount > maxLines) {
    throw new Error(
      `written artifact line count outside requested range: path=${writtenPath} lines=${lineCount} expected=${expectedLines} range=${minLines}-${maxLines}`,
    );
  }
  if (/html|网页|页面|小程序/i.test(prompt) && !/<html[\s>]/i.test(content)) {
    throw new Error(`written artifact does not look like HTML: path=${writtenPath}`);
  }
  return { path: writtenPath, line_count: lineCount };
}

function updateRoundEventSummary(result, events, previousEventCount) {
  const roundEvents = events.slice(previousEventCount);
  result.event_count = events.length;
  result.new_event_count = roundEvents.length;
  result.event_types = roundEvents.map((event) => event.event_type).filter(Boolean).slice(-12);
  result.notable_events = notableEvents(roundEvents);
  result.loop_budget_events = roundEvents.filter((event) => event.event_type === "agent.loop_budget_reached").length;
  result.write_fallback_events = roundEvents.filter((event) => event.event_type === "runtime.write_intent_fallback").length;
  return result;
}

async function waitForUiEvidence(page, needles, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  let lastText = "";
  while (Date.now() < deadline) {
    lastText = await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
    if (textIncludesAny(lastText, needles)) {
      return lastText;
    }
    await page.waitForTimeout(120);
  }
  throw new Error(`timed out waiting for UI evidence: ${needles.join(", ")}`);
}

async function waitForSessionId(timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = apiResponses
      .map((entry) => entry.json)
      .find((json) => json?.session?.session_id || json?.session_id);
    const id = hit?.session?.session_id || hit?.session_id;
    if (id) {
      return id;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error("session id was not observed from runtime API responses");
}

async function waitForSubmitUserMessageResponse(startIndex, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = apiResponses
      .slice(startIndex)
      .find((entry) => entry.url.includes("/runtime/submit-user-message"));
    if (hit) {
      return hit;
    }
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error("runtime submit-user-message response was not observed");
}

async function getEvents(apiBaseUrl, sessionId) {
  let cursor = 0;
  const events = [];
  for (let page = 0; page < 80; page += 1) {
    const stream = await httpGetJson(
      `${apiBaseUrl}/runtime/stream-events?session_id=${encodeURIComponent(sessionId)}&cursor=${cursor}&max_events=200`,
    );
    if (Array.isArray(stream.json.events)) {
      events.push(...stream.json.events);
    }
    if (!stream.json.has_more || stream.json.next_cursor === cursor) {
      break;
    }
    cursor = Number(stream.json.next_cursor || cursor);
  }
  return events;
}

async function getSnapshot(apiBaseUrl, sessionId) {
  const snapshot = await httpGetJson(
    `${apiBaseUrl}/runtime/get-snapshot?session_id=${encodeURIComponent(sessionId)}`,
  );
  return snapshot.json?.snapshot || snapshot.json || {};
}

async function waitForSessionNotExecuting(apiBaseUrl, sessionId, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = "";
  while (Date.now() < deadline) {
    const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
    lastState = String(snapshot.state || "");
    if (
      lastState &&
      !["Executing", "RetrievingContext", "RunningCommand", "ApplyingPatch"].includes(lastState)
    ) {
      return lastState;
    }
    await new Promise((resolve) => setTimeout(resolve, 300));
  }
  throw new Error(`session ${sessionId} did not leave Executing state; last_state=${lastState || "unknown"}`);
}

async function waitForGuiTerminalState(page, state, timeoutMs = 8_000) {
  if (!["Completed", "Failed", "Cancelled"].includes(state)) {
    return;
  }
  const deadline = Date.now() + timeoutMs;
  let lastBody = "";
  while (Date.now() < deadline) {
    lastBody = await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
    const stillRunning =
      lastBody.includes("模型正在流式输出") ||
      /最新状态\s*(执行中|运行中)/.test(lastBody) ||
      /执行中\s*收敛分歧/.test(lastBody);
    if (!stillRunning && (state !== "Completed" || lastBody.includes("已完成"))) {
      return;
    }
    await page.waitForTimeout(200);
  }
  throw new Error(`GUI did not reflect terminal state ${state}; tail=${lastBody.slice(-500)}`);
}

async function clickAnyPendingApproval(page, shouldStop = () => false, timeoutMs = 45_000) {
  const deadline = Date.now() + timeoutMs;
  let clicked = 0;
  const approvalButtons = [
    /允许一次|allow once/i,
    /批准计划并继续执行|批准并继续|通过|approve/i,
  ];
  while (Date.now() < deadline && !shouldStop()) {
    let didClick = false;
    for (const name of approvalButtons) {
      const candidates = [
        page.getByRole("button", { name }).first(),
        page.locator("button").filter({ hasText: name }).first(),
        page.locator(`[aria-label*="允许"]`).first(),
        page.locator(`[aria-label*="批准"]`).first(),
      ];
      for (const button of candidates) {
        if (await button.isVisible().catch(() => false)) {
          await button.click({ timeout: 3000, force: true });
          clicked += 1;
          didClick = true;
          await page.waitForTimeout(300);
          break;
        }
      }
      if (didClick) {
        break;
      }
    }
    if (didClick) {
      continue;
    }
    await page.waitForTimeout(250);
  }
  return clicked;
}

async function waitForEventEvidence(
  apiBaseUrl,
  sessionId,
  previousCount,
  wantedTypes,
  timeoutMs = 20_000,
  expectedToolId = "",
) {
  const deadline = Date.now() + timeoutMs;
  let events = [];
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      events = await getEvents(apiBaseUrl, sessionId);
      lastError = "";
    } catch (error) {
      events = [];
      lastError = String(error.message || error);
    }
    const newEvents = events.slice(previousCount);
    const hasExpectedTool = expectedToolId
      ? newEvents.some(
          (event) =>
            event.event_type === "tool.call_completed" &&
            String(event.payload?.tool_id || "") === expectedToolId &&
            event.payload?.ok === true,
        )
      : false;
    const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
    const state = String(snapshot.state || "");
    const hasWantedEvent =
      wantedTypes.length === 0 ||
      newEvents.some((event) => wantedTypes.includes(event.event_type));
    const hasStrongRuntimeActivity = newEvents.some((event) =>
      [
        "tool.call_requested",
        "tool.call_completed",
        "agent.loop_budget_reached",
        "agent.loop_recovery",
        "permission.requested",
        "permission.decided",
        "runtime.error",
        "model.call_blocked",
        "model.http_failure_recovered",
        "model.transport_failure_recovered",
      ].includes(event.event_type),
    );
    const settled = ["Completed", "Failed", "WaitingForUser", "WaitingForToolApproval"].includes(state);
    if (expectedToolId && hasExpectedTool) {
      return events;
    }
    if (!expectedToolId && newEvents.length > 0 && hasWantedEvent && (hasStrongRuntimeActivity || settled)) {
      return events;
    }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  const error = new Error(
    `timed out waiting for backend events after cursor ${previousCount}; last_error=${lastError || "none"}; tail=${JSON.stringify(
      notableEvents(events).slice(-6),
    )}`,
  );
  error.events = events;
  throw error;
}

async function openAppWithRetry(page, webBaseUrl) {
  let lastError = null;
  for (let attempt = 1; attempt <= 2; attempt += 1) {
    const failureStart = requestFailures.length;
    const consoleStart = consoleMessages.length;
    try {
      await page.goto(webBaseUrl, { waitUntil: "domcontentloaded" });
      await page.locator("textarea").first().waitFor({ state: "visible", timeout: 15_000 });
      return;
    } catch (error) {
      lastError = error;
      const transientNetworkChange = requestFailures
        .slice(failureStart)
        .some((failure) => String(failure.failure || "").includes("ERR_NETWORK_CHANGED"));
      if (attempt < 2 && transientNetworkChange) {
        requestFailures.splice(failureStart);
        consoleMessages.splice(consoleStart);
        await page.waitForTimeout(350);
        continue;
      }
      throw error;
    }
  }
  throw lastError;
}

async function runRound({
  page,
  apiBaseUrl,
  index,
  name,
  prompt,
  evidence = [],
  eventEvidence = [],
  eventTimeoutMs = 20_000,
  expectedToolId = "",
  expectedToolIds = [],
  expectedWrittenPath = "",
  previousEventCount = 0,
  permission,
  allowWriteFallback = false,
  workspaceRoot = "",
}) {
  const roundStarted = Date.now();
  log(`round ${index}: ${name}`);
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 10_000 });
  await textarea.click();
  await textarea.fill(prompt);
  await page.waitForTimeout(120);
  const submitResponseStart = apiResponses.length;
  log(`round ${index}: submitting prompt`);
  await page.keyboard.press("Enter");

  log(`round ${index}: waiting for session id`);
  const sessionId = await waitForSessionId();
  log(`round ${index}: observed session ${sessionId}`);
  const submitResponse = await waitForSubmitUserMessageResponse(submitResponseStart);
  if (submitResponse.json?.ok === false) {
    throw new Error(
      `submit-user-message failed: ${submitResponse.json?.error_code || JSON.stringify(submitResponse.json)}`,
    );
  }
  log(`round ${index}: submit-user-message accepted`);
  let stopApprovalWatcher = false;
  const approvalClicks = permission
    ? Promise.resolve(0)
    : clickAnyPendingApproval(page, () => stopApprovalWatcher, 45_000).catch(() => 0);

  if (permission) {
    await waitForUiEvidence(page, ["需要 Shell 命令审批", "shell.command", "permission"], 10_000);
    const allowOnce = page.getByRole("button", { name: /允许一次|allow once/i }).first();
    await allowOnce.click({ timeout: 5000 });
  }

  let events = [];
  let roundEvents = [];
  let bodyText = "";
  const screenshot = path.join(artifactDir, `round_${String(index).padStart(2, "0")}.png`);
  try {
    log(`round ${index}: waiting for runtime evidence`);
    try {
      events = eventEvidence.length > 0
        ? await waitForEventEvidence(
            apiBaseUrl,
            sessionId,
            previousEventCount,
            eventEvidence,
            eventTimeoutMs,
            expectedToolId,
          )
        : await getEvents(apiBaseUrl, sessionId).catch(() => []);
    } catch (error) {
      if (Array.isArray(error.events)) {
        events = error.events;
        const failedEventsJson = path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events_on_error.json`);
        const failedEventsJsonl = path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events_on_error.jsonl`);
        await fs.writeFile(failedEventsJson, JSON.stringify(events, null, 2));
        await fs.writeFile(failedEventsJsonl, events.map((event) => JSON.stringify(event)).join("\n") + "\n");
      }
      const failedScreenshot = path.join(artifactDir, `round_${String(index).padStart(2, "0")}_on_error.png`);
      const failedBodyText = await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
      await page.screenshot({ path: failedScreenshot, fullPage: true }).catch(() => {});
      await fs.writeFile(
        path.join(artifactDir, `round_${String(index).padStart(2, "0")}_body_on_error.txt`),
        failedBodyText,
        "utf8",
      ).catch(() => {});
      throw error;
    }
    log(`round ${index}: collected ${events.length} events`);
    const eventsJson = path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events.json`);
    const eventsJsonl = path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events.jsonl`);
    await fs.writeFile(eventsJson, JSON.stringify(events, null, 2));
    await fs.writeFile(eventsJsonl, events.map((event) => JSON.stringify(event)).join("\n") + "\n");
    roundEvents = events.slice(previousEventCount);
    validateRoundEvents(roundEvents, expectedToolId, allowWriteFallback, expectedToolIds);
    if (isToolInventoryPrompt(prompt) && roundEvents.some((event) => event.event_type === "agent.loop_budget_reached")) {
      throw new Error("tool inventory round reached loop budget instead of producing a bounded capability summary");
    }
    const writtenArtifact = expectedToolId === "file.write"
      ? await validateWrittenArtifact({ roundEvents, prompt, workspaceRoot, expectedWrittenPath })
      : null;
    if (writtenArtifact) {
      log(`round ${index}: validated written artifact ${writtenArtifact.path} lines=${writtenArtifact.line_count}`);
    }
    bodyText = evidence.length > 0
      ? await waitForUiEvidence(page, evidence, 12_000)
      : await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
    log(`round ${index}: taking screenshot`);
    await page.screenshot({ path: screenshot, fullPage: true });
  } finally {
    stopApprovalWatcher = true;
  }
  const approvalsClicked = await approvalClicks;

  return updateRoundEventSummary({
    index,
    name,
    prompt,
    ok: true,
    duration_ms: Date.now() - roundStarted,
    screenshot,
    session_id: sessionId,
    events_json: path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events.json`),
    events_jsonl: path.join(artifactDir, `round_${String(index).padStart(2, "0")}_events.jsonl`),
    evidence: evidence.filter((needle) => textIncludesAny(bodyText, [needle])),
    approvals_clicked: approvalsClicked,
    written_artifact: expectedToolId === "file.write"
      ? await validateWrittenArtifact({ roundEvents, prompt, workspaceRoot, expectedWrittenPath })
      : null,
  }, events, previousEventCount);
}

const allRounds = [
  {
    name: "repo map conversation",
    prompt: "repo structure please",
    evidence: ["repo.map", "Runtime 错误", "模型调用被拒绝", "模型实时调用未启用", "本轮已停止", "已停止"],
    eventEvidence: ["tool.call_completed", "assistant.message", "runtime.error", "model.call_blocked", "agent.loop_stopped"],
  },
  {
    name: "file read conversation",
    prompt: "read README.md",
    evidence: ["file.read", "README.md", "Runtime 错误", "模型调用被拒绝", "模型实时调用未启用", "本轮已停止", "已停止"],
    eventEvidence: ["tool.call_completed", "assistant.message", "runtime.error", "model.call_blocked", "agent.loop_stopped"],
  },
  {
    name: "permission approval flow",
    prompt: "/run echo gui-smoke",
    evidence: [
      "permission.decided",
      "repo.map",
      "允许一次",
      "已完成",
      "Runtime 错误",
      "模型调用被拒绝",
      "模型实时调用未启用",
      "本轮已停止",
      "已停止",
    ],
    eventEvidence: [
      "permission.requested",
      "permission.decided",
      "tool.call_completed",
      "assistant.message",
      "runtime.error",
      "model.call_blocked",
      "agent.loop_stopped",
    ],
  },
];

const realRuntimeRounds = [
  {
    name: "real runtime tool inventory prompt",
    prompt: "尝试一下你所拥有的工具",
    eventEvidence: ["assistant.message", "runtime.error", "model.call_blocked", "model.http_failure_recovered", "model.transport_failure_recovered"],
  },
  {
    name: "real runtime follow-up prompt",
    prompt: "你刚刚什么意思？",
    eventEvidence: ["assistant.message", "runtime.error", "model.call_blocked", "model.http_failure_recovered", "model.transport_failure_recovered"],
  },
  {
    name: "real runtime file-oriented prompt",
    prompt: "请读取 README.md 并总结一句话",
    eventEvidence: ["assistant.message", "runtime.error", "model.call_blocked", "model.http_failure_recovered", "model.transport_failure_recovered"],
  },
];

const customPromptRounds = promptArgs.map((prompt, index) => {
  const expectsWrite = /写|写入|保存|新建|html|file|write/i.test(prompt);
  const expectedToolIds = expectedToolIdsArg.length > 0
    ? expectedToolIdsArg
    : expectsWrite
      ? ["file.write"]
      : [];
  return {
    name: `custom prompt ${index + 1}`,
    prompt,
    eventEvidence: expectsWrite
      ? [
          "tool.call_completed",
          "permission.decided",
          "runtime.error",
          "model.call_blocked",
          "model.http_failure_recovered",
          "model.transport_failure_recovered",
        ]
      : [
          "tool.call_requested",
          "tool.call_completed",
          "assistant.message",
          "runtime.error",
          "model.call_blocked",
          "model.http_failure_recovered",
          "model.transport_failure_recovered",
        ],
    eventTimeoutMs: expectsWrite ? 90_000 : liveProvider ? 60_000 : 20_000,
    expectedToolId: expectedToolIds.includes("file.write")
      ? "file.write"
      : expectedToolIds[expectedToolIds.length - 1] || (expectsWrite ? "file.write" : ""),
    expectedToolIds,
    expectedWrittenPath: expectedWrittenPathArg,
    allowWriteFallback: false,
  };
});

const incidentRuntimeRounds = [
  {
    name: "incident file.write approval resume",
    prompt: "请在当前工作区新建 incident_gui_write_probe.txt，内容只写一行: runtime permission probe",
    evidence: ["file.write", "incident_gui_write_probe"],
    eventEvidence: ["permission.requested", "permission.decided", "tool.call_completed", "runtime.error"],
    eventTimeoutMs: liveProvider ? 150_000 : 75_000,
    expectedToolId: "file.write",
    allowWriteFallback: false,
  },
  {
    name: "incident shell.command approval resume",
    prompt: "/run echo incident-shell-probe",
    evidence: ["shell.command", "incident-shell-probe", "已完成"],
    eventEvidence: ["permission.requested", "permission.decided", "tool.call_completed", "runtime.error"],
    eventTimeoutMs: liveProvider ? 120_000 : 60_000,
    expectedToolId: "shell.command",
    allowWriteFallback: false,
  },
];

while (allRounds.length < requestedRounds) {
  allRounds.push({
    name: `search conversation ${allRounds.length + 1}`,
    prompt: `search runtime_facade round${allRounds.length + 1}`,
    evidence: ["search.ripgrep", "matches", "Runtime 错误", "模型调用被拒绝", "模型实时调用未启用", "本轮已停止", "已停止"],
    eventEvidence: ["tool.call_completed", "assistant.message", "runtime.error", "model.call_blocked", "agent.loop_stopped"],
  });
}

class SkipLiveGuiRun extends Error {}

let browser;
let finalReport = {
  ok: false,
  artifact_dir: artifactDir,
  headed,
  backend,
  real_dialogue: realDialogue,
  incident_verify: incidentVerify,
  incident_fixture_path: incidentFixturePath,
  incident_strict_telemetry: incidentStrictTelemetry,
  provider_mode: liveProvider ? "live" : "fast-fail",
  rounds: [],
  console_errors: consoleMessages,
  request_failures: requestFailures,
  runtime_http_errors: runtimeHttpErrors,
  runtime_event_refresh_warnings: [],
};

try {
  if (incidentVerify && incidentFixturePath) {
    log(`incident fixture: ${incidentFixturePath}`);
    finalReport.incident_fixture_assessment = await analyzeIncidentFixture(incidentFixturePath);
    const fixtureReportPath = path.join(artifactDir, "incident_fixture_assessment.json");
    await fs.writeFile(fixtureReportPath, JSON.stringify(finalReport.incident_fixture_assessment, null, 2));
    log(`incident fixture assessment: ${fixtureReportPath}`);
  }
  if (incidentVerify && incidentFixtureOnly) {
    finalReport.ok = true;
    finalReport.duration_ms = Date.now() - started;
    log("--incident-fixture-only set; skipping live GUI run.");
    throw new SkipLiveGuiRun();
  }

  const apiPort = await findFreePort();
  const webPort = await findFreePort();
  const fastFailProviderPort = await findFreePort();
  const apiBaseUrl = `http://127.0.0.1:${apiPort}`;
  const webBaseUrl = `http://127.0.0.1:${webPort}`;
  const qwenBaseUrl =
    process.env.QWEN_BASE_URL ||
    (liveProvider
      ? "http://127.0.0.1:11434/v1/chat/completions"
      : `http://127.0.0.1:${fastFailProviderPort}/v1/chat/completions`);

  log(`artifacts: ${artifactDir}`);
  if (backend !== "rust") {
    throw new Error(`unsupported backend ${backend}; desktop smoke now uses the Rust local API server`);
  }
  spawnLogged("rust-local-api", "cargo", [
    "run",
    "-p",
    "researchcode-cli",
    "--",
    "local-api-server",
    String(apiPort),
  ], {
    cwd: REPO_ROOT,
    pipeStdin: true,
    env: {
      ...process.env,
      RESEARCHCODE_LOCAL_API_TOKEN: "",
      RESEARCHCODE_ENABLE_LIVE_PROVIDER:
        process.env.RESEARCHCODE_ENABLE_LIVE_PROVIDER || "1",
      RESEARCHCODE_ALLOW_NETWORK:
        process.env.RESEARCHCODE_ALLOW_NETWORK || "1",
      RESEARCHCODE_DEEPSEEK_PROTOCOL:
        process.env.RESEARCHCODE_DEEPSEEK_PROTOCOL || "anthropic",
      RESEARCHCODE_LOCAL_API_MAX_ITERATIONS:
        process.env.RESEARCHCODE_LOCAL_API_MAX_ITERATIONS || (liveProvider ? "0" : "2"),
      RESEARCHCODE_LOCAL_API_MAX_TOOL_CALLS:
        process.env.RESEARCHCODE_LOCAL_API_MAX_TOOL_CALLS || (liveProvider ? "0" : "2"),
      RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS:
        process.env.RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS || (liveProvider ? "45" : "1.5"),
      RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS:
        process.env.RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS || (liveProvider ? "45" : "1.5"),
      QWEN_BASE_URL: qwenBaseUrl,
      QWEN_API_KEY:
        process.env.QWEN_API_KEY || "local-qwen-ollama",
    },
  });
  await waitForHttp(`${apiBaseUrl}/health`, "local api");

  const viteBin = path.join(DESKTOP_DIR, "node_modules", ".bin", "vite");
  spawnLogged("vite", viteBin, [
    "--host",
    "127.0.0.1",
    "--port",
    String(webPort),
  ], {
    cwd: DESKTOP_DIR,
    env: {
      ...process.env,
      BROWSER: "none",
    },
  });
  await waitForWeb(webBaseUrl, "vite");

  browser = await chromium.launch({
    headless: !headed,
    args: ["--window-size=1440,980"],
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 980 },
    deviceScaleFactor: 1,
  });
  await context.addInitScript(({ apiBaseUrl, apiPort, workspaceArg, providerArg }) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({
        provider: providerArg === "deepseek" ? "deepseek" : "qwen",
        baseUrl: "http://127.0.0.1:11434/v1",
        modelId: providerArg === "deepseek" ? "deepseek-v4-flash" : undefined,
      }),
    );
    if (workspaceArg) {
      window.localStorage.setItem("argon_agent_selected_project_path_v1", workspaceArg);
    }
    window.__ARGON_RUNTIME_BOOTSTRAP__ = {
      transport: "http",
      baseUrl: apiBaseUrl,
      token: "",
      workspaceRoot: workspaceArg || ".",
      port: apiPort,
      logPath: "",
    };
  }, { apiBaseUrl, apiPort, workspaceArg, providerArg });

  const page = await context.newPage();
  page.on("console", (message) => {
    const entry = { type: message.type(), text: message.text() };
    if (["error", "warning"].includes(message.type())) {
      consoleMessages.push(entry);
    }
  });
  page.on("requestfailed", (request) => {
    requestFailures.push({
      url: request.url(),
      failure: request.failure()?.errorText || "unknown",
    });
  });
  page.on("response", async (response) => {
    const url = response.url();
    if (!url.includes("/runtime/")) {
      return;
    }
    const json = await response.json().catch(() => null);
    apiResponses.push({ url, status: response.status(), json });
    if (response.status() >= 400) {
      runtimeHttpErrors.push({
        url,
        endpoint: runtimeEndpoint(url),
        status: response.status(),
        json,
      });
    }
  });

  await openAppWithRetry(page, webBaseUrl);
  if (autonomyModeArg) {
    const autonomySelect = page.locator("select").first();
    await autonomySelect.selectOption(autonomyModeArg);
    log(`selected autonomy mode ${autonomyModeArg}`);
  }
  await page.screenshot({ path: path.join(artifactDir, "round_00_initial.png"), fullPage: true });

  const selectedRounds = (customPromptRounds.length > 0
    ? customPromptRounds
    : incidentVerify
      ? incidentRuntimeRounds
      : realDialogue
      ? realRuntimeRounds
      : allRounds
  ).slice(0, requestedRounds);
  let previousEventCount = 0;
  let lastSessionId = "";
  for (let index = 0; index < selectedRounds.length; index += 1) {
    const roundStartCursor = previousEventCount;
    const result = await withTimeout(
      runRound({
        page,
        apiBaseUrl,
        index: index + 1,
        previousEventCount,
        workspaceRoot: workspaceArg,
        ...selectedRounds[index],
      }),
      liveProvider ? 150_000 : 45_000,
      `round ${index + 1}`,
    );
    finalReport.rounds.push(result);
    lastSessionId = result.session_id;
    log(`round ${index + 1}: waiting for session to settle`);
    const state = await withTimeout(
      waitForSessionNotExecuting(
        apiBaseUrl,
        result.session_id,
        liveProvider ? liveSessionSettleTimeoutMs : 20_000,
      ),
      liveProvider ? liveSessionSettleTimeoutMs + 5_000 : 25_000,
      `round ${index + 1} session settle`,
    );
    log(`round ${index + 1}: session state after settle is ${state}`);
    await waitForGuiTerminalState(page, state);
    const settledScreenshot = path.join(artifactDir, `round_${String(index + 1).padStart(2, "0")}_settled.png`);
    await page.screenshot({ path: settledScreenshot, fullPage: true });
    result.settled_screenshot = settledScreenshot;
    let settledEvents = await getEvents(apiBaseUrl, result.session_id).catch(() => []);
    if (settledEvents.length === 0 && result.events_json) {
      finalReport.runtime_event_refresh_warnings.push({
        round: index + 1,
        reason: "settled_event_refresh_empty_kept_pre_settle_events",
      });
      settledEvents = JSON.parse(await fs.readFile(result.events_json, "utf8"));
    }
    const settledBodyText = await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
    if (state === "Completed" && settledBodyText.includes("模型正在流式输出")) {
      throw new Error(`round ${index + 1} completed while GUI still showed streaming indicator`);
    }
    const settledRoundEvents = settledEvents.slice(roundStartCursor);
    validateRoundEvents(
      settledRoundEvents,
      selectedRounds[index].expectedToolId || "",
      selectedRounds[index].allowWriteFallback || false,
      selectedRounds[index].expectedToolIds || [],
    );
    if (
      isToolInventoryPrompt(selectedRounds[index].prompt) &&
      settledRoundEvents.some((event) => event.event_type === "agent.loop_budget_reached")
    ) {
      throw new Error("tool inventory round reached loop budget after settling instead of producing a bounded capability summary");
    }
    if (result.events_json) {
      await fs.writeFile(result.events_json, JSON.stringify(settledEvents, null, 2));
    }
    if (result.events_jsonl) {
      await fs.writeFile(result.events_jsonl, settledEvents.map((event) => JSON.stringify(event)).join("\n") + "\n");
    }
    updateRoundEventSummary(result, settledEvents, roundStartCursor);
    previousEventCount = settledEvents.length;
    if (index + 1 < selectedRounds.length) {
      log(`round ${index + 1}: next round cursor is ${previousEventCount}`);
    }
    if (realDialogue) {
      await page.waitForTimeout(150);
    }
  }

  const blockingRequestFailures = requestFailures.filter((entry) => !isBenignRuntimeStreamFailure(entry));
  finalReport.ignored_request_failures = requestFailures.filter(isBenignRuntimeStreamFailure);
  finalReport.blocking_request_failures = blockingRequestFailures;
  finalReport.ignored_runtime_http_errors = runtimeHttpErrors.filter(isBenignRuntimeHttpError);
  finalReport.blocking_runtime_http_errors = runtimeHttpErrors.filter(
    (entry) => !isBenignRuntimeHttpError(entry),
  );
  const consoleClassification = classifyConsoleMessages(
    consoleMessages,
    finalReport.ignored_runtime_http_errors,
  );
  finalReport.ignored_console_errors = consoleClassification.ignored;
  finalReport.blocking_console_errors = consoleClassification.blocking;
  if (incidentVerify && lastSessionId) {
    const incidentEvents = await getEvents(apiBaseUrl, lastSessionId);
    const assessment = assessRuntimeIncidentEvents(incidentEvents, {
      mode: "live",
      strictTelemetry: incidentStrictTelemetry,
    });
    finalReport.incident_live_assessment = assessment;
    const liveReportPath = path.join(artifactDir, "incident_live_assessment.json");
    await fs.writeFile(liveReportPath, JSON.stringify(assessment, null, 2));
    log(`incident live assessment: ${liveReportPath}`);
    if (assessment.failures.length > 0) {
      throw new Error(`incident live regression check failed: ${assessment.failures.join("; ")}`);
    }
  }
  finalReport.ok =
    finalReport.rounds.length === selectedRounds.length &&
    finalReport.rounds.every((round) => round.ok) &&
    blockingRequestFailures.length === 0 &&
    finalReport.blocking_runtime_http_errors.length === 0 &&
    finalReport.blocking_console_errors.length === 0;
  finalReport.duration_ms = Date.now() - started;
  finalReport.api_base_url = apiBaseUrl;
  finalReport.web_base_url = webBaseUrl;
  finalReport.qwen_base_url = qwenBaseUrl;
} catch (error) {
  if (!(error instanceof SkipLiveGuiRun)) {
    finalReport.ok = false;
    finalReport.error = String(error.stack || error.message || error);
    finalReport.duration_ms = Date.now() - started;
    console.error(finalReport.error);
  }
} finally {
  const reportPath = path.join(artifactDir, "report.json");
  await fs.writeFile(reportPath, JSON.stringify(finalReport, null, 2));
  log(`report: ${reportPath}`);
  await cleanup(browser);
  if (!finalReport.ok) {
    process.exitCode = 1;
  }
}
