#!/usr/bin/env node
import { chromium } from "playwright";
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs/promises";
import http from "node:http";
import net from "node:net";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const DESKTOP_DIR = path.dirname(__filename);
const REPO_ROOT = path.resolve(DESKTOP_DIR, "..");
const DEFAULT_SOURCE_WORKSPACE = "/Users/gongyuxuan/Documents/Argon-Agent-test";

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find((arg) => arg.startsWith(prefix))?.slice(prefix.length) || "";
}

const args = new Set(process.argv.slice(2));
const headed = args.has("--headed");
const keepOpen = args.has("--keep-open");
const useSourceWorkspaceDirectly = args.has("--use-source-workspace");
const syntheticWorkspace = args.has("--synthetic-workspace");
const privacyDryRun = args.has("--privacy-dry-run");
const includeWorkspaceManifest = args.has("--include-workspace-manifest");
const redactWorkspaceManifest = args.has("--redact-workspace-manifest") || (privacyDryRun && !includeWorkspaceManifest);
const allowExternalWorkspaceProvider = args.has("--allow-external-workspace-provider");
const sourceWorkspace = argValue("source") || DEFAULT_SOURCE_WORKSPACE;
const workspaceArg = argValue("workspace") || "";
const provider = argValue("provider") || "deepseek";
const rounds = Math.max(1, Math.min(4, Number(argValue("rounds") || "4")));
const started = Date.now();
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `argon-longtask-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];
const apiResponses = [];
const runtimeHttpErrors = [];
const apiApprovalFallbacks = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[argon-longtask +${elapsed}ms] ${message}`);
}

function parseJson(text) {
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

function runtimeEndpoint(url) {
  const match = String(url || "").match(/\/runtime\/[^?]+/);
  return match ? match[0] : String(url || "");
}

function isBenignRuntimeStreamFailure(entry) {
  const url = String(entry?.url || "");
  const failure = String(entry?.failure || "");
  return url.includes("/runtime/stream-events") && /ERR_CONNECTION_RESET|ERR_ABORTED/i.test(failure);
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
      ignored.push({ ...message, reason: "matched_ignored_runtime_http_400" });
      continue;
    }
    if (/Failed to load resource: net::ERR_CONNECTION_RESET/i.test(text)) {
      ignored.push({ ...message, reason: "runtime_stream_closed_during_test_shutdown" });
      continue;
    }
    blocking.push(message);
  }
  return { ignored, blocking };
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

function spawnLogged(label, command, commandArgs, options = {}) {
  const child = spawn(command, commandArgs, {
    stdio: options.pipeStdin ? ["pipe", "pipe", "pipe"] : ["ignore", "pipe", "pipe"],
    ...options,
  });
  childProcesses.push({ label, child });
  child.stdout.on("data", (chunk) => {
    const text = chunk.toString().trim();
    if (text) log(`${label}: ${text.split("\n").slice(-2).join(" | ")}`);
  });
  child.stderr.on("data", (chunk) => {
    const text = chunk.toString().trim();
    if (text) log(`${label} stderr: ${text.split("\n").slice(-2).join(" | ")}`);
  });
  child.on("exit", (code, signal) => {
    if (!keepOpen) log(`${label} exited code=${code} signal=${signal || ""}`);
  });
  return child;
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
          reject(new Error(`invalid JSON from ${url}: ${body.slice(0, 160)}`));
          return;
        }
        resolve({ status: res.statusCode || 0, json });
      });
    });
    req.on("error", reject);
    req.setTimeout(timeoutMs, () => req.destroy(new Error(`timeout fetching ${url}`)));
  });
}

function httpPostJson(url, payload, headers = {}, timeoutMs = 15_000) {
  return new Promise((resolve, reject) => {
    const body = JSON.stringify(payload);
    const parsed = new URL(url);
    const req = http.request(
      {
        hostname: parsed.hostname,
        port: parsed.port,
        path: `${parsed.pathname}${parsed.search}`,
        method: "POST",
        headers: {
          "content-type": "application/json; charset=utf-8",
          "content-length": Buffer.byteLength(body),
          ...headers,
        },
      },
      (res) => {
        let responseBody = "";
        res.setEncoding("utf8");
        res.on("data", (chunk) => {
          responseBody += chunk;
        });
        res.on("end", () => {
          const json = parseJson(responseBody);
          if (!json) {
            reject(new Error(`invalid JSON from ${url}: ${responseBody.slice(0, 160)}`));
            return;
          }
          resolve({ status: res.statusCode || 0, json });
        });
      },
    );
    req.on("error", reject);
    req.setTimeout(timeoutMs, () => req.destroy(new Error(`timeout posting ${url}`)));
    req.write(body);
    req.end();
  });
}

async function waitForHttp(url, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      const result = await httpGetJson(url, {}, 800);
      if (result.status >= 200 && result.status < 500) return result.json;
      lastError = `status ${result.status}`;
    } catch (error) {
      lastError = String(error.message || error);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label} did not become ready: ${lastError}`);
}

async function waitForWeb(url, label, timeoutMs = 25_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError = "";
  while (Date.now() < deadline) {
    try {
      await new Promise((resolve, reject) => {
        const req = http.get(url, (res) => {
          res.resume();
          if ((res.statusCode || 0) >= 200 && (res.statusCode || 0) < 500) resolve();
          else reject(new Error(`status ${res.statusCode}`));
        });
        req.on("error", reject);
        req.setTimeout(1000, () => req.destroy(new Error("timeout")));
      });
      return;
    } catch (error) {
      lastError = String(error.message || error);
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`${label} did not become ready: ${lastError}`);
}

async function copyWorkspaceIfNeeded() {
  if (workspaceArg) return path.resolve(workspaceArg);
  if (syntheticWorkspace) return await createSyntheticArgonWorkspace();
  if (provider === "deepseek" && !allowExternalWorkspaceProvider) {
    throw new Error(
      [
        "Refusing to send the real Argon-Agent-test workspace to an external provider without explicit script consent.",
        "Use --synthetic-workspace for a non-private pipeline stress run, or add --allow-external-workspace-provider only after the user explicitly accepts that workspace contents may be sent to DeepSeek.",
      ].join(" "),
    );
  }
  if (useSourceWorkspaceDirectly) return path.resolve(sourceWorkspace);
  const target = path.join(artifactDir, "Argon-Agent-test-workspace");
  await fs.cp(sourceWorkspace, target, {
    recursive: true,
    dereference: false,
    filter: (src) => {
      const rel = path.relative(sourceWorkspace, src);
      return !rel.includes(`${path.sep}.argon_agent${path.sep}`) && !rel.includes(`${path.sep}.git${path.sep}`);
    },
  });
  return target;
}

async function createSyntheticArgonWorkspace() {
  const root = path.join(artifactDir, "Argon-Agent-test-synthetic-workspace");
  const files = new Map([
    [
      "plan/VoiceNote-AI-实施计划.md",
      [
        "# VoiceNote AI implementation plan",
        "",
        "Goal: provide a compact voice-note app with recording, transcript, speaker, and summary flows.",
        "",
        "- Model layer: Recording, Speaker, Summary, TranscriptSegment.",
        "- Service layer: ASRService, LLMService, AudioEngine.",
        "- Tests should validate configuration defaults and a small probe that can run without network.",
      ].join("\n"),
    ],
    [
      "plan/技术实现要点.md",
      [
        "# Technical Notes",
        "",
        "- Use SwiftData-style model names in examples.",
        "- Keep tests deterministic and avoid live microphone or network access.",
        "- Shell verification may use `swift test --help` when a full package build is not required.",
      ].join("\n"),
    ],
    [
      "plan/QUICKREF.md",
      [
        "# Quick Reference",
        "",
        "- Target test file: VoiceNote/Tests/VoiceNoteTests/ArgonGuiLongTaskProbeTests.swift",
        "- Safe shell probe: swift test --help",
      ].join("\n"),
    ],
    [
      "VoiceNote/project.yml",
      [
        "name: VoiceNote",
        "targets:",
        "  VoiceNote:",
        "    type: application",
        "    platform: iOS",
        "  VoiceNoteTests:",
        "    type: bundle.unit-test",
        "    platform: iOS",
      ].join("\n"),
    ],
    [
      "VoiceNote/Tests/VoiceNoteTests/ExistingSmokeTests.swift",
      [
        "import XCTest",
        "",
        "final class ExistingSmokeTests: XCTestCase {",
        "    func testFixtureIsAvailable() {",
        "        XCTAssertTrue(true)",
        "    }",
        "}",
      ].join("\n"),
    ],
    [
      "review_2048.md",
      [
        "# Synthetic Review",
        "",
        "This fixture intentionally contains no private Argon-Agent-test code.",
      ].join("\n"),
    ],
  ]);
  for (const [rel, content] of files) {
    const abs = path.join(root, rel);
    await fs.mkdir(path.dirname(abs), { recursive: true });
    await fs.writeFile(abs, `${content}\n`, "utf8");
  }
  return root;
}

async function readWorkspaceManifest(workspaceRoot) {
  const files = [
    "plan/VoiceNote-AI-实施计划.md",
    "plan/技术实现要点.md",
    "plan/QUICKREF.md",
    "VoiceNote/project.yml",
    "review_2048.md",
  ];
  const manifest = [];
  for (const [index, rel] of files.entries()) {
    const abs = path.join(workspaceRoot, rel);
    const pathValue = redactWorkspaceManifest ? `canary_input_${index + 1}` : rel;
    try {
      const stat = await fs.stat(abs);
      manifest.push(redactWorkspaceManifest ? { path: pathValue, present: true } : { path: pathValue, bytes: stat.size });
    } catch {
      manifest.push({ path: pathValue, missing: true });
    }
  }
  return manifest;
}

async function cleanup(browser) {
  if (keepOpen) {
    log("--keep-open set; leaving browser and servers running.");
    return;
  }
  await browser?.close().catch(() => {});
  for (const { child } of childProcesses.reverse()) {
    if (!child.killed) child.kill("SIGTERM");
  }
}

async function openApp(page, webBaseUrl) {
  await page.goto(webBaseUrl, { waitUntil: "domcontentloaded" });
  await page.locator("textarea").first().waitFor({ state: "visible", timeout: 20_000 });
}

async function sendPrompt(page, text) {
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15_000 });
  await textarea.click();
  await textarea.fill(text);
  await page.keyboard.press("Enter");
}

async function waitForSessionId(timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const hit = apiResponses
      .map((entry) => entry.json)
      .find((json) => json?.session?.session_id || json?.session_id);
    const id = hit?.session?.session_id || hit?.session_id;
    if (id) return id;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error("session id was not observed from runtime API responses");
}

async function getEvents(apiBaseUrl, sessionId) {
  let cursor = 0;
  const events = [];
  for (let page = 0; page < 200; page += 1) {
    const result = await httpGetJson(
      `${apiBaseUrl}/runtime/stream-events?session_id=${encodeURIComponent(sessionId)}&cursor=${cursor}&max_events=200`,
      {},
      15_000,
    );
    if (Array.isArray(result.json.events)) events.push(...result.json.events);
    if (!result.json.has_more || Number(result.json.next_cursor || cursor) === cursor) break;
    cursor = Number(result.json.next_cursor || cursor);
  }
  return events;
}

async function getSnapshot(apiBaseUrl, sessionId) {
  const result = await httpGetJson(
    `${apiBaseUrl}/runtime/get-snapshot?session_id=${encodeURIComponent(sessionId)}`,
    {},
    10_000,
  );
  return result.json?.snapshot || result.json || {};
}

async function clickPendingApprovals(page, stop, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs;
  let clicked = 0;
  const buttonNames = [
    /批准计划并继续执行|批准并继续|approve/i,
    /允许本次权限请求|允许一次|allow once/i,
  ];
  while (Date.now() < deadline && !stop()) {
    let didClick = false;
    for (const name of buttonNames) {
      const candidates = [
        page.getByRole("button", { name }).first(),
        page.locator("button").filter({ hasText: name }).first(),
      ];
      for (const button of candidates) {
        if (await button.isVisible().catch(() => false)) {
          await button.click({ timeout: 5000, force: true });
          clicked += 1;
          didClick = true;
          await page.waitForTimeout(450);
          break;
        }
      }
      if (didClick) break;
    }
    await page.waitForTimeout(didClick ? 250 : 350);
  }
  return clicked;
}

async function clickVisibleApprovalButton(page) {
  const buttonNames = [
    /批准计划并继续执行|批准并继续|approve/i,
    /允许本次权限请求|允许一次|allow once/i,
  ];
  for (const name of buttonNames) {
    const candidates = [
      page.getByRole("button", { name }).first(),
      page.locator("button").filter({ hasText: name }).first(),
    ];
    for (const button of candidates) {
      if (await button.isVisible().catch(() => false)) {
        await button.click({ timeout: 5000, force: true });
        return true;
      }
    }
  }
  return false;
}

function latestPendingPlan(events) {
  const decided = new Set(
    events
      .filter((event) => event.event_type === "plan.approval_decided")
      .map((event) => eventPayload(event)?.plan_approval_id)
      .filter(Boolean),
  );
  return [...events]
    .reverse()
    .find(
      (event) =>
        event.event_type === "plan.approval_requested" &&
        eventPayload(event)?.plan_approval_id &&
        !decided.has(eventPayload(event).plan_approval_id),
    );
}

function latestPendingPermission(events) {
  const decided = new Set(
    events
      .filter((event) =>
        event.event_type === "permission.decided" ||
        event.event_type === "tool.permission.resolved" ||
        event.event_type === "tool.permission.denied"
      )
      .map((event) => eventPayload(event)?.permission_id)
      .filter(Boolean),
  );
  return [...events]
    .reverse()
    .find(
      (event) =>
        (event.event_type === "permission.requested" ||
          event.event_type === "tool.permission.requested") &&
        eventPayload(event)?.permission_id &&
        !decided.has(eventPayload(event).permission_id),
    );
}

function eventPayload(event) {
  if (event?.payload && typeof event.payload === "object") return event.payload;
  if (typeof event?.payload === "string") return parseJson(event.payload) || {};
  return {};
}

async function approvePendingViaApiIfNeeded(apiBaseUrl, sessionId, events, reason) {
  const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
  const state = String(snapshot.state || "");
  if (state === "WaitingForPlanApproval" || Number(snapshot.pending_plan_approval_count || 0) > 0) {
    const pending = latestPendingPlan(events);
    const planApprovalId = String(eventPayload(pending)?.plan_approval_id || "");
    if (planApprovalId) {
      log(`approving plan via API (${reason}): ${planApprovalId}`);
      const response = await httpPostJson(`${apiBaseUrl}/runtime/submit-plan-decision`, {
        session_id: sessionId,
        plan_approval_id: planApprovalId,
        decision: "approve",
      });
      apiApprovalFallbacks.push({
        kind: "plan",
        id: planApprovalId,
        reason,
        status: response.status,
        ok: response.json?.ok !== false,
      });
      return true;
    }
  }
  if (state === "WaitingForToolApproval" || Number(snapshot.pending_permission_count || 0) > 0) {
    const pending = latestPendingPermission(events);
    const permissionId = String(eventPayload(pending)?.permission_id || "");
    if (permissionId) {
      log(`approving permission via API (${reason}): ${permissionId}`);
      const response = await httpPostJson(`${apiBaseUrl}/runtime/submit-permission-decision`, {
        session_id: sessionId,
        permission_id: permissionId,
        decision: "allow_once",
      });
      apiApprovalFallbacks.push({
        kind: "permission",
        id: permissionId,
        reason,
        status: response.status,
        ok: response.json?.ok !== false,
      });
      return true;
    }
  }
  return false;
}

async function waitForEvent(apiBaseUrl, sessionId, predicate, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  let events = [];
  let lastState = "";
  while (Date.now() < deadline) {
    events = await getEvents(apiBaseUrl, sessionId).catch(() => events);
    if (events.some(predicate)) return events;
    const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
    lastState = String(snapshot.state || lastState || "");
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`${label} not observed before timeout; last_state=${lastState}; tail=${eventTail(events)}`);
}

async function waitForSettled(apiBaseUrl, sessionId, timeoutMs = 300_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = "";
  while (Date.now() < deadline) {
    const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
    lastState = String(snapshot.state || "");
    if (["Completed", "Failed", "Cancelled", "WaitingForUser", "WaitingForToolApproval", "WaitingForPlanApproval"].includes(lastState)) {
      return lastState;
    }
    await new Promise((resolve) => setTimeout(resolve, 800));
  }
  throw new Error(`session did not settle; last_state=${lastState || "unknown"}`);
}

async function waitForSettledDrivingApprovals(page, apiBaseUrl, sessionId, timeoutMs = 900_000) {
  const deadline = Date.now() + timeoutMs;
  let lastState = "";
  let events = [];
  while (Date.now() < deadline) {
    events = await getEvents(apiBaseUrl, sessionId).catch(() => events);
    const snapshot = await getSnapshot(apiBaseUrl, sessionId).catch(() => ({}));
    lastState = String(snapshot.state || lastState || "");
    if (
      lastState === "WaitingForPlanApproval" ||
      lastState === "WaitingForToolApproval" ||
      Number(snapshot.pending_plan_approval_count || 0) > 0 ||
      Number(snapshot.pending_permission_count || 0) > 0
    ) {
      const clicked = await clickVisibleApprovalButton(page);
      if (!clicked) {
        await approvePendingViaApiIfNeeded(
          apiBaseUrl,
          sessionId,
          events,
          "gui_button_not_visible_during_stress",
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 900));
      continue;
    }
    if (["Completed", "Failed", "Cancelled", "WaitingForUser"].includes(lastState)) {
      return lastState;
    }
    await new Promise((resolve) => setTimeout(resolve, 800));
  }
  throw new Error(`session did not settle; last_state=${lastState || "unknown"}; tail=${eventTail(events)}`);
}

async function saveArtifacts(page, name, events = null) {
  const screenshot = path.join(artifactDir, `${name}.png`);
  const bodyPath = path.join(artifactDir, `${name}.txt`);
  await page.screenshot({ path: screenshot, fullPage: true }).catch(() => {});
  const bodyText = await page.locator("body").innerText({ timeout: 1000 }).catch(() => "");
  await fs.writeFile(bodyPath, bodyText, "utf8").catch(() => {});
  if (events) {
    await fs.writeFile(path.join(artifactDir, `${name}_events.json`), JSON.stringify(events, null, 2), "utf8");
    await fs.writeFile(path.join(artifactDir, `${name}_events.jsonl`), events.map((event) => JSON.stringify(event)).join("\n") + "\n", "utf8");
  }
  return { screenshot, bodyPath };
}

function eventTail(events, count = 10) {
  return JSON.stringify(
    events.slice(-count).map((event) => ({
      seq: event.sequence,
      type: event.event_type,
      payload: JSON.stringify(event.payload ?? {}).slice(0, 240),
    })),
  );
}

function countEvents(events, type) {
  return events.filter((event) => event.event_type === type).length;
}

function terminalBlockedReason(events) {
  const terminal = [...events]
    .reverse()
    .find(
      (event) =>
        event.event_type === "agent.loop_stopped" ||
        event.event_type === "agent.loop_plateau_stopped" ||
        event.event_type === "agent.loop_state.terminal",
    );
  if (terminal?.event_type === "agent.loop_state.terminal") {
    const status = String(terminal.payload?.status || "").toLowerCase();
    const category = String(terminal.payload?.category || "").toLowerCase();
    if (status === "completed" || category === "model_answer") {
      return "";
    }
    if (!["blocked", "failed", "interrupted"].includes(status)) {
      return "";
    }
  }
  return String(terminal?.payload?.reason || "");
}

function hasTool(events, toolId) {
  return events.some(
    (event) =>
      event.event_type === "tool.call_completed" &&
      String(event.payload?.tool_id || "").replace(/_/g, ".") === toolId &&
      event.payload?.ok !== false,
  );
}

function eventText(event) {
  return JSON.stringify(event?.payload ?? {});
}

function hasToolEvidenceTouching(events, toolId, text) {
  return events.some(
    (event) =>
      ["tool.call_requested", "tool.result_recorded"].includes(event.event_type) &&
      String(event.payload?.tool_id || "").replace(/_/g, ".") === toolId &&
      eventText(event).includes(text),
  );
}

async function assessWorkspaceArtifacts(workspaceRoot) {
  const rel = "VoiceNote/Tests/VoiceNoteTests/ArgonGuiLongTaskProbeTests.swift";
  const abs = path.join(workspaceRoot, rel);
  try {
    const content = await fs.readFile(abs, "utf8");
    return {
      expected_test_file: rel,
      exists: true,
      bytes: Buffer.byteLength(content),
      line_count: content.length === 0 ? 0 : content.split(/\r\n|\r|\n/).length,
      contains_xctest: content.includes("XCTest"),
      contains_probe_name: content.includes("ArgonGuiLongTaskProbe"),
    };
  } catch (error) {
    return {
      expected_test_file: rel,
      exists: false,
      error: String(error.message || error),
    };
  }
}

function assertNoArchitecturalDrift(events) {
  const forbidden = [];
  for (const event of events) {
    const text = JSON.stringify(event.payload ?? {});
    if (event.event_type === "agent.final_answer" || text.includes("agent.final_answer")) {
      forbidden.push(`final_answer:${event.sequence}`);
    }
    if (event.event_type === "runtime.permission_resume.model_continuation_skipped") {
      forbidden.push(`permission_continuation_skipped:${event.sequence}`);
    }
    if (event.event_type === "agent.loop_budget_reached") {
      forbidden.push(`loop_budget:${event.sequence}`);
    }
    if (text.includes("Tools are disabled") || text.includes("disable_tools_and_request_final_answer")) {
      forbidden.push(`disabled_tools_finalizer:${event.sequence}`);
    }
  }
  if (forbidden.length > 0) {
    throw new Error(`doc39 drift / legacy finalizer path observed: ${forbidden.join(", ")}`);
  }
}

async function assessCoverage(events, workspaceRoot = "") {
  const failures = [];
  const toolIds = [...new Set(events
    .filter((event) => event.event_type === "tool.call_completed")
    .map((event) => String(event.payload?.tool_id || "").replace(/_/g, ".")))].sort();
  if (countEvents(events, "model.call_started") < 2) failures.push("expected at least two model calls / continuations");
  if (countEvents(events, "model.stream_delta") < 1) failures.push("expected visible streaming deltas");
  if (countEvents(events, "tool.call_completed") < 6) failures.push("expected at least six completed tool calls");
  if (countEvents(events, "plan.approval_requested") < 1) failures.push("missing plan approval request");
  if (countEvents(events, "plan.approval_decided") < 1) failures.push("missing plan approval decision");
  if (countEvents(events, "permission.requested") < 1) failures.push("missing permission request");
  if (countEvents(events, "permission.decided") < 1) failures.push("missing permission decision");
  if (!hasTool(events, "shell.command")) failures.push("missing successful shell.command");
  if (!toolIds.some((id) => ["file.read", "search.ripgrep", "repo.map"].includes(id))) {
    failures.push("missing read/search/repo-map tool evidence");
  }
  if (!toolIds.some((id) => ["file.write", "file.edit", "file.multi.edit", "patch.apply"].includes(id))) {
    failures.push("missing write/edit tool evidence");
  }
  const artifact = workspaceRoot ? await assessWorkspaceArtifacts(workspaceRoot) : null;
  if (artifact && !artifact.exists) failures.push(`missing expected written test file: ${artifact.expected_test_file}`);
  if (artifact && artifact.exists && !artifact.contains_xctest) failures.push("written test probe does not contain XCTest");
  if (!hasToolEvidenceTouching(events, "file.read", "ArgonGuiLongTaskProbeTests.swift")) {
    failures.push("missing read-back request for ArgonGuiLongTaskProbeTests.swift");
  }
  const unresolvedPermissions = events
    .filter((event) => event.event_type === "permission.requested")
    .filter((request) => !events.some(
      (event) => event.event_type === "permission.decided" &&
        event.payload?.permission_id === request.payload?.permission_id,
    ));
  if (unresolvedPermissions.length > 0) failures.push(`unresolved permissions: ${unresolvedPermissions.map((event) => event.payload?.permission_id).join(", ")}`);
  assertNoArchitecturalDrift(events);
  return {
    event_count: events.length,
    model_calls: countEvents(events, "model.call_started"),
    stream_deltas: countEvents(events, "model.stream_delta"),
    tool_completed: countEvents(events, "tool.call_completed"),
    tool_ids: toolIds,
    plan_approvals: countEvents(events, "plan.approval_requested"),
    permission_requests: countEvents(events, "permission.requested"),
    workspace_artifact: artifact,
    failures,
  };
}

function buildPrompts(workspaceManifest) {
  const manifestText = workspaceManifest
    .map((entry) => {
      if (entry.missing) return `${entry.path} missing`;
      if (Number.isFinite(entry.bytes)) return `${entry.path} ${entry.bytes} bytes`;
      if (entry.present) return `${entry.path} present`;
      return `${entry.path} status_unknown`;
    })
    .join("; ");
  const workspaceDescription = syntheticWorkspace
    ? "Argon-Agent-test 结构等价 synthetic fixture"
    : "Argon-Agent-test 的真实长任务压力测试";
  return [
    [
      `这是 ${workspaceDescription}。`,
      "第一步只能调用 plan.enter 提交一个计划；不要先读取文件、搜索目录或运行命令。",
      "目标：依据 VoiceNote 和 plan 目录，补一个小而真实的测试探针文件，然后运行一个安全的 shell 验证命令。",
      "计划里应写明：批准后至少读取/搜索这些材料中的一部分：plan/VoiceNote-AI-实施计划.md、plan/技术实现要点.md、VoiceNote/project.yml。",
      "批准后请执行：读取或搜索项目结构，写入 VoiceNote/Tests/VoiceNoteTests/ArgonGuiLongTaskProbeTests.swift，随后通过 shell.command 运行 swift test --help 或 xcodegen --version 这类轻量验证。",
      "不要使用 final answer 工具，不要用工具禁用后的 finalizer；自然地在工具结果后继续。",
      `已知文件清单：${manifestText}`,
    ].join("\n"),
    "继续执行上一轮计划：如果测试探针还没写完就写完；如果已写完，请读取它并运行一次轻量 shell 验证，然后总结下一步。",
    "现在追问：你刚才 shell 审批后是否继续了？请基于事件/文件状态回答，并避免重新读取完全相同的大文件。",
    "最后做一次简短复盘：列出本轮你实际使用过的工具、审批点、写入文件和验证结果。",
  ].slice(0, rounds);
}

function redactPromptForReport(prompt) {
  return String(prompt)
    .replace(/plan\/VoiceNote-AI-实施计划\.md/g, "<canary-plan-1>")
    .replace(/plan\/技术实现要点\.md/g, "<canary-plan-2>")
    .replace(/plan\/QUICKREF\.md/g, "<canary-plan-3>")
    .replace(/VoiceNote\/project\.yml/g, "<canary-project>")
    .replace(/review_2048\.md/g, "<canary-review>")
    .replace(/VoiceNote\/Tests\/VoiceNoteTests\/ArgonGuiLongTaskProbeTests\.swift/g, "<canary-test-probe>");
}

function promptRecordsForReport(prompts) {
  if (!redactWorkspaceManifest) return prompts;
  return prompts.map((prompt, index) => ({
    index: index + 1,
    sha256: crypto.createHash("sha256").update(prompt).digest("hex"),
    bytes: Buffer.byteLength(prompt),
    redacted_preview: redactPromptForReport(prompt).slice(0, 1200),
  }));
}

function buildLiveCanaryContract(workspaceRoot, workspaceManifest) {
  const prompts = buildPrompts(workspaceManifest);
  const requiredCoverage = [
    "Anthropic-compatible DeepSeek protocol path",
    "plan.enter approval and resume",
    "file read/search after plan approval",
    "file.write/file.edit/patch.apply evidence",
    "shell.command permission approval and continuation",
    "read-back of VoiceNote/Tests/VoiceNoteTests/ArgonGuiLongTaskProbeTests.swift",
    "follow-up question after the long turn settles",
    "no agent.final_answer / visible_finalizer / model_continuation_skipped",
    "no premature GUI Completed state",
    "no unresolved permission or active turn after terminal state",
  ];
  return {
    name: "Argon-Agent-test long live canary",
    mode: privacyDryRun ? "privacy_dry_run" : syntheticWorkspace ? "synthetic_workspace" : "real_workspace",
    workspace_root: workspaceRoot,
    source_workspace: sourceWorkspace,
    provider,
    protocol: provider === "deepseek" ? "anthropic_compatible" : "provider_default",
    rounds: prompts.length,
    external_provider_boundary: {
      may_send_workspace_contents: !privacyDryRun && !syntheticWorkspace && allowExternalWorkspaceProvider,
      requires_flag: "--allow-external-workspace-provider",
      required_user_acceptance:
        "The real Argon-Agent-test workspace may be copied into the run artifact directory and sent to the configured external provider during model/tool context construction.",
      safe_alternative_flag: "--synthetic-workspace",
      dry_run_flag: "--privacy-dry-run",
      manifest_redacted: redactWorkspaceManifest,
      include_manifest_flag: "--include-workspace-manifest",
    },
    required_coverage: redactWorkspaceManifest
      ? requiredCoverage.map(redactPromptForReport)
      : requiredCoverage,
    prompts: promptRecordsForReport(prompts),
    workspace_manifest: workspaceManifest,
  };
}

let browser;
const finalReport = {
  ok: false,
  artifact_dir: artifactDir,
  source_workspace: sourceWorkspace,
  provider,
  headed,
  rounds,
  synthetic_workspace: syntheticWorkspace,
  allow_external_workspace_provider: allowExternalWorkspaceProvider,
  console_errors: consoleMessages,
  request_failures: requestFailures,
  runtime_http_errors: runtimeHttpErrors,
  api_approval_fallbacks: apiApprovalFallbacks,
};

try {
  if (privacyDryRun) {
    const workspaceRoot = path.resolve(workspaceArg || sourceWorkspace);
    const workspaceManifest = await readWorkspaceManifest(workspaceRoot);
    finalReport.ok = true;
    finalReport.privacy_dry_run = true;
    finalReport.workspace_root = workspaceRoot;
    finalReport.workspace_manifest = workspaceManifest;
    finalReport.live_canary_contract = buildLiveCanaryContract(workspaceRoot, workspaceManifest);
    finalReport.duration_ms = Date.now() - started;
    log("privacy dry run only; no browser, runtime server, model call, workspace copy, or provider request was started.");
  } else {
  const workspaceRoot = await copyWorkspaceIfNeeded();
  finalReport.workspace_root = workspaceRoot;
  const workspaceManifest = await readWorkspaceManifest(workspaceRoot);
  finalReport.workspace_manifest = workspaceManifest;
  finalReport.live_canary_contract = buildLiveCanaryContract(workspaceRoot, workspaceManifest);

  const apiPort = await findFreePort();
  const webPort = await findFreePort();
  const apiBaseUrl = `http://127.0.0.1:${apiPort}`;
  const webBaseUrl = `http://127.0.0.1:${webPort}`;
  log(`artifacts: ${artifactDir}`);
  log(`workspace: ${workspaceRoot}`);

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
      RESEARCHCODE_ENABLE_LIVE_PROVIDER: "1",
      RESEARCHCODE_ALLOW_NETWORK: process.env.RESEARCHCODE_ALLOW_NETWORK || "1",
      RESEARCHCODE_DEEPSEEK_PROTOCOL: process.env.RESEARCHCODE_DEEPSEEK_PROTOCOL || "anthropic_compatible",
      RESEARCHCODE_LOCAL_API_MAX_ITERATIONS: process.env.RESEARCHCODE_LOCAL_API_MAX_ITERATIONS || "0",
      RESEARCHCODE_LOCAL_API_MAX_TOOL_CALLS: process.env.RESEARCHCODE_LOCAL_API_MAX_TOOL_CALLS || "0",
      RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS: process.env.RESEARCHCODE_LOCAL_API_TURN_TIMEOUT_SECS || "0",
      RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS: process.env.RESEARCHCODE_PROVIDER_HTTP_TIMEOUT_SECONDS || "90",
      RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS: process.env.RESEARCHCODE_PROVIDER_STREAM_IDLE_TIMEOUT_SECONDS || "90",
    },
  });
  await waitForHttp(`${apiBaseUrl}/health`, "local api");

  const viteBin = path.join(DESKTOP_DIR, "node_modules", ".bin", "vite");
  spawnLogged("vite", viteBin, ["--host", "127.0.0.1", "--port", String(webPort)], {
    cwd: DESKTOP_DIR,
    env: { ...process.env, BROWSER: "none" },
  });
  await waitForWeb(webBaseUrl, "vite");

  browser = await chromium.launch({ headless: !headed, args: ["--window-size=1512,982"] });
  const context = await browser.newContext({
    viewport: { width: 1512, height: 982 },
    deviceScaleFactor: 1,
  });
  await context.addInitScript(({ apiBaseUrl, apiPort, workspaceRoot, provider }) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({
        provider,
        baseUrl: provider === "qwen" ? "http://127.0.0.1:11434/v1" : "https://api.deepseek.com/v1",
        modelId: provider === "deepseek" ? "deepseek-v4-flash" : undefined,
      }),
    );
    window.localStorage.setItem("argon_agent_selected_project_path_v1", workspaceRoot);
    window.__ARGON_RUNTIME_BOOTSTRAP__ = {
      transport: "http",
      baseUrl: apiBaseUrl,
      token: "",
      workspaceRoot,
      port: apiPort,
      logPath: "",
    };
  }, { apiBaseUrl, apiPort, workspaceRoot, provider });

  const page = await context.newPage();
  page.on("console", (message) => {
    const entry = { type: message.type(), text: message.text() };
    if (["error", "warning"].includes(message.type())) consoleMessages.push(entry);
  });
  page.on("requestfailed", (request) => {
    requestFailures.push({ url: request.url(), failure: request.failure()?.errorText || "unknown" });
  });
  page.on("response", async (response) => {
    if (!response.url().includes("/runtime/")) return;
    const json = await response.json().catch(() => null);
    apiResponses.push({ url: response.url(), status: response.status(), json });
    if (response.status() >= 400) {
      runtimeHttpErrors.push({
        url: response.url(),
        endpoint: runtimeEndpoint(response.url()),
        status: response.status(),
        json,
      });
    }
  });

  await openApp(page, webBaseUrl);
  await saveArtifacts(page, "initial");

  const prompts = buildPrompts(workspaceManifest);
  let sessionId = "";
  const roundReports = [];
  for (let index = 0; index < prompts.length; index += 1) {
    const prompt = prompts[index];
    log(`round ${index + 1}: submit`);
    await sendPrompt(page, prompt);
    sessionId = await waitForSessionId();
    const wanted = index === 0
      ? (event) => event.event_type === "plan.approval_requested"
      : (event) => event.event_type === "model.call_started" || event.event_type === "tool.call_completed";
    const eventsAfterEvidence = await waitForEvent(apiBaseUrl, sessionId, wanted, 180_000, `round ${index + 1} evidence`);
    await saveArtifacts(page, `round_${String(index + 1).padStart(2, "0")}_evidence`, eventsAfterEvidence);
    await approvePendingViaApiIfNeeded(apiBaseUrl, sessionId, eventsAfterEvidence, "post_evidence_pending");
    const state = await waitForSettledDrivingApprovals(page, apiBaseUrl, sessionId, 900_000);
    const settledEvents = await getEvents(apiBaseUrl, sessionId);
    const assessment = await assessCoverage(settledEvents, workspaceRoot);
    await saveArtifacts(page, `round_${String(index + 1).padStart(2, "0")}_settled`, settledEvents);
    roundReports.push({
      index: index + 1,
      state,
      event_count: settledEvents.length,
      coverage: assessment,
    });
    log(`round ${index + 1}: state=${state} events=${settledEvents.length} tools=${assessment.tool_completed}`);
    if (state === "Failed" || state === "Cancelled") {
      throw new Error(`round ${index + 1} reached terminal bad state ${state}; tail=${eventTail(settledEvents)}`);
    }
    const blockedReason = terminalBlockedReason(settledEvents);
    if (blockedReason) {
      throw new Error(`round ${index + 1} stopped structurally: ${blockedReason}; tail=${eventTail(settledEvents)}`);
    }
    if (index === 0) {
      await waitForEvent(
        apiBaseUrl,
        sessionId,
        (event) =>
          event.event_type === "permission.decided" ||
          (event.event_type === "tool.call_completed" && String(event.payload?.tool_id || "") === "shell.command"),
        240_000,
        "post-plan permission/shell continuation",
      );
    }
  }

  const events = await getEvents(apiBaseUrl, sessionId);
  const coverage = await assessCoverage(events, workspaceRoot);
  if (coverage.failures.length > 0) {
    throw new Error(`coverage failed: ${coverage.failures.join("; ")}; tail=${eventTail(events)}`);
  }
  const blockingRequestFailures = requestFailures.filter(
    (entry) => !isBenignRuntimeStreamFailure(entry),
  );
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
  if (blockingRequestFailures.length > 0) {
    throw new Error(`browser request failures: ${JSON.stringify(blockingRequestFailures.slice(0, 5))}`);
  }
  if (finalReport.blocking_runtime_http_errors.length > 0) {
    throw new Error(`runtime HTTP errors: ${JSON.stringify(finalReport.blocking_runtime_http_errors.slice(0, 5))}`);
  }
  if (finalReport.blocking_console_errors.length > 0) {
    throw new Error(`browser console errors: ${JSON.stringify(finalReport.blocking_console_errors.slice(0, 5))}`);
  }

  finalReport.ok = true;
  finalReport.session_id = sessionId;
  finalReport.round_reports = roundReports;
  finalReport.coverage = coverage;
  finalReport.api_approval_fallbacks = apiApprovalFallbacks;
  finalReport.duration_ms = Date.now() - started;
  finalReport.api_base_url = apiBaseUrl;
  finalReport.web_base_url = webBaseUrl;
  await saveArtifacts(page, "final", events);
  }
} catch (error) {
  finalReport.ok = false;
  finalReport.error = String(error.stack || error.message || error);
  finalReport.duration_ms = Date.now() - started;
  console.error(finalReport.error);
} finally {
  const reportPath = path.join(artifactDir, "report.json");
  await fs.writeFile(reportPath, JSON.stringify(finalReport, null, 2), "utf8");
  log(`report: ${reportPath}`);
  await cleanup(browser);
  if (!finalReport.ok) process.exitCode = 1;
}
