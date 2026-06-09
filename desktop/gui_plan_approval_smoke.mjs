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
const args = new Set(process.argv.slice(2));
const headed = args.has("--headed");
const keepOpen = args.has("--keep-open");
const started = Date.now();
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `plan-approval-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[plan-approval +${elapsed}ms] ${message}`);
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
  const child = spawn(command, commandArgs, { stdio: ["ignore", "pipe", "pipe"], ...options });
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

async function waitForWeb(url, label, timeoutMs = 20_000) {
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
    await new Promise((resolve) => setTimeout(resolve, 80));
  }
  throw new Error(`${label} did not become ready: ${lastError}`);
}

async function waitForCondition(label, predicate, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = predicate();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error(`${label} did not become true within ${timeoutMs}ms`);
}

function jsonResponse(res, status, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "access-control-allow-origin": "*",
    "access-control-allow-headers": "content-type, authorization",
    "access-control-allow-methods": "GET,POST,OPTIONS",
  });
  res.end(body);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => {
      body += chunk;
    });
    req.on("end", () => {
      try {
        resolve(body.trim() ? JSON.parse(body) : {});
      } catch (error) {
        reject(error);
      }
    });
    req.on("error", reject);
  });
}

function makeEvent(sequence, eventType, payload = {}) {
  return {
    event_id: `evt_plan_${String(sequence).padStart(5, "0")}`,
    sequence,
    event_type: eventType,
    payload,
    created_at: new Date().toISOString(),
  };
}

function createMockRuntimeServer() {
  const sessionId = "plan_approval_session_001";
  const planApprovalId = "plan_smoke_approval";
  const events = [];
  const decisions = [];
  const timers = new Set();
  let sequence = 0;
  let state = "Idle";
  let completed = false;
  let followupSubmitted = false;

  const push = (eventType, payload = {}) => {
    sequence += 1;
    events.push(makeEvent(sequence, eventType, payload));
  };

  const schedule = (delayMs, fn) => {
    const timer = setTimeout(() => {
      timers.delete(timer);
      fn();
    }, delayMs);
    timers.add(timer);
  };

  const pushTool = (index, delayMs) => {
    schedule(delayMs, () => {
      const toolCallId = `plan_resume_read_${String(index).padStart(2, "0")}`;
      push("tool.call_requested", {
        tool_call_id: toolCallId,
        tool_id: "file.read",
        args_preview: `{"path":"VoiceNote/Sources/File${index}.swift"}`,
      });
      push("tool.call_completed", {
        tool_call_id: toolCallId,
        tool_id: "file.read",
        ok: true,
        duration_ms: 5,
      });
      push("model.stream_delta", {
        stream_id: "plan_stream",
        delta_kind: "content",
        preview: `计划恢复后继续审阅文件 ${index}。\n`,
        runtime_sanitized: true,
      });
    });
  };

  const startRun = () => {
    if (state !== "Idle") return;
    state = "WaitingForPlanApproval";
    push("session.state_changed", { from_state: "Idle", to_state: "Executing" });
    push("model.context_budget", {
      estimated_total_tokens: 24_000,
      target_limit_tokens: 128_000,
      hard_limit_tokens: 128_000,
    });
    push("model.stream_delta", {
      stream_id: "plan_stream",
      delta_kind: "content",
      preview: "我先形成计划，等待你批准后继续。\n",
      runtime_sanitized: true,
    });
    push("plan.mode_entered", {
      plan_approval_id: planApprovalId,
      plan_preview: "## 计划\n\n1. 继续读取现有证据\n2. 创建测试文件\n3. 验证结果",
    });
    push("plan.approval_requested", {
      plan_approval_id: planApprovalId,
      goal: "为 VoiceNote 创建测试文件",
    });
    push("session.state_changed", { from_state: "Executing", to_state: "WaitingForPlanApproval" });
  };

  const approvePlan = (body) => {
    decisions.push(body);
    state = "Executing";
    push("plan.approval_decided", { plan_approval_id: planApprovalId, decision: "approve" });
    push("runtime.plan_approval.model_continued", { plan_approval_id: planApprovalId });
    push("session.state_changed", { from_state: "WaitingForPlanApproval", to_state: "Executing" });
    push("model.stream_delta", {
      stream_id: "plan_stream",
      delta_kind: "content",
      preview: "计划已批准，我会继续执行。\n",
      runtime_sanitized: true,
    });
    for (let index = 1; index <= 6; index += 1) {
      pushTool(index, 120 + index * 90);
    }
    schedule(900, () => {
      push("model.stream_completed", {
        stream_id: "plan_stream",
        completion_tokens: 96,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "plan_resume_model_call" });
      push("assistant.message", { content: "计划审批链路已恢复并继续执行。" });
      completed = true;
      state = "Completed";
      push("session.state_changed", { from_state: "Executing", to_state: "Completed" });
    });
  };

  const startFollowup = () => {
    followupSubmitted = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Completed", to_state: "Executing" });
    push("model.call_started", { call_id: "plan_followup_model_call" });
    schedule(80, () => {
      push("model.stream_delta", {
        stream_id: "plan_followup_stream",
        delta_kind: "content",
        preview: "计划审批恢复后的跟进已正常响应。\n",
        runtime_sanitized: true,
      });
      push("model.stream_completed", {
        stream_id: "plan_followup_stream",
        completion_tokens: 24,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "plan_followup_model_call" });
      push("assistant.message", { content: "计划审批恢复后的跟进已正常响应。" });
      state = "Completed";
      push("session.state_changed", { from_state: "Executing", to_state: "Completed" });
    });
  };

  const server = http.createServer(async (req, res) => {
    if (req.method === "OPTIONS") {
      jsonResponse(res, 200, {});
      return;
    }
    const url = new URL(req.url || "/", "http://127.0.0.1");
    try {
      if (url.pathname === "/health") return jsonResponse(res, 200, { ok: true });
      if (url.pathname === "/runtime/list-commands") {
        return jsonResponse(res, 200, { commands: ["/plan", "/review", "/status"] });
      }
      if (url.pathname === "/runtime/start-session" && req.method === "POST") {
        await readBody(req);
        return jsonResponse(res, 200, {
          ok: true,
          session: {
            session_id: sessionId,
            task_id: "plan_approval_task",
            workspace_root: ".",
            model_mode: "deepseek",
            autonomy_mode: "conservative",
            state,
          },
        });
      }
      if (url.pathname === "/runtime/submit-user-message" && req.method === "POST") {
        await readBody(req);
        if (completed) startFollowup();
        else startRun();
        return jsonResponse(res, 200, { ok: true, session_id: sessionId });
      }
      if (url.pathname === "/runtime/submit-plan-decision" && req.method === "POST") {
        const body = await readBody(req);
        if (body.plan_approval_id !== planApprovalId) {
          return jsonResponse(res, 409, {
            ok: false,
            session_id: sessionId,
            error_code: "plan_id_mismatch",
            plan_approval_id: body.plan_approval_id,
          });
        }
        approvePlan(body);
        return jsonResponse(res, 200, {
          ok: true,
          session_id: sessionId,
          plan_approval_id: planApprovalId,
        });
      }
      if (url.pathname === "/runtime/stream-events") {
        const cursor = Number(url.searchParams.get("cursor") || "0");
        const maxEvents = Number(url.searchParams.get("max_events") || "200");
        const nextEvents = events.filter((event) => Number(event.sequence || 0) > cursor).slice(0, maxEvents);
        const nextCursor = nextEvents.length > 0 ? Number(nextEvents[nextEvents.length - 1].sequence) : cursor;
        return jsonResponse(res, 200, {
          session_id: sessionId,
          from_cursor: cursor,
          next_cursor: nextCursor,
          has_more: events.some((event) => Number(event.sequence || 0) > nextCursor),
          events: nextEvents,
          jsonl: nextEvents.map((event) => JSON.stringify(event)).join("\n"),
        });
      }
      if (url.pathname === "/runtime/get-snapshot") {
        return jsonResponse(res, 200, {
          snapshot: {
            session_id: sessionId,
            state,
            event_count: events.length,
            model_mode: "deepseek",
            autonomy_mode: "conservative",
            workspace_root: ".",
            pending_permission_count: 0,
            pending_plan_approval_count: state === "WaitingForPlanApproval" ? 1 : 0,
          },
        });
      }
      if (url.pathname === "/runtime/export-events" && req.method === "POST") {
        return jsonResponse(res, 200, {
          ok: true,
          session_id: sessionId,
          path: path.join(artifactDir, "mock_runtime_events.jsonl"),
        });
      }
      return jsonResponse(res, 404, { error: "not_found", path: url.pathname });
    } catch (error) {
      return jsonResponse(res, 500, { error: String(error.message || error) });
    }
  });

  return {
    server,
    planApprovalId,
    events,
    decisions,
    followupSubmitted: () => followupSubmitted,
    close: () => {
      for (const timer of timers) clearTimeout(timer);
      server.close();
    },
  };
}

async function cleanup(browser, mockRuntime) {
  if (keepOpen) {
    log("--keep-open set; leaving browser and servers running.");
    return;
  }
  await browser?.close().catch(() => {});
  mockRuntime?.close();
  for (const { child } of childProcesses.reverse()) {
    if (!child.killed) child.kill("SIGTERM");
  }
}

let browser;
let mockRuntime;
const finalReport = {
  ok: false,
  artifact_dir: artifactDir,
  headed,
  console_errors: consoleMessages,
  request_failures: requestFailures,
};

try {
  const runtimePort = await findFreePort();
  const webPort = await findFreePort();
  const runtimeBaseUrl = `http://127.0.0.1:${runtimePort}`;
  const webBaseUrl = `http://127.0.0.1:${webPort}`;
  log(`artifacts: ${artifactDir}`);

  mockRuntime = createMockRuntimeServer();
  await new Promise((resolve, reject) => {
    mockRuntime.server.once("error", reject);
    mockRuntime.server.listen(runtimePort, "127.0.0.1", resolve);
  });
  log(`mock runtime listening on ${runtimeBaseUrl}`);

  const viteBin = path.join(DESKTOP_DIR, "node_modules", ".bin", "vite");
  spawnLogged("vite", viteBin, ["--host", "127.0.0.1", "--port", String(webPort)], {
    cwd: DESKTOP_DIR,
    env: { ...process.env, BROWSER: "none" },
  });
  await waitForWeb(webBaseUrl, "vite");

  browser = await chromium.launch({ headless: !headed, args: ["--window-size=1440,980"] });
  const context = await browser.newContext({ viewport: { width: 1440, height: 980 }, deviceScaleFactor: 1 });
  await context.addInitScript(({ runtimeBaseUrl, runtimePort }) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({ provider: "deepseek", baseUrl: "https://api.deepseek.com/anthropic/v1/messages" }),
    );
    window.__ARGON_RUNTIME_BOOTSTRAP__ = {
      transport: "http",
      baseUrl: runtimeBaseUrl,
      token: "",
      workspaceRoot: ".",
      port: runtimePort,
      logPath: "",
    };
  }, { runtimeBaseUrl, runtimePort });

  const page = await context.newPage();
  page.on("console", (message) => {
    const entry = { type: message.type(), text: message.text() };
    if (["error", "warning"].includes(message.type())) consoleMessages.push(entry);
  });
  page.on("requestfailed", (request) => {
    requestFailures.push({ url: request.url(), failure: request.failure()?.errorText || "unknown" });
  });

  await page.goto(webBaseUrl, { waitUntil: "domcontentloaded" });
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15_000 });
  await textarea.fill("先进入计划审批，然后批准并继续。");
  await page.keyboard.press("Enter");

  const dialog = page.getByRole("dialog", { name: "计划审批" });
  await dialog.waitFor({ state: "visible", timeout: 10_000 });
  await page.screenshot({ path: path.join(artifactDir, "approval.png"), fullPage: true });
  const approvalText = await dialog.innerText();
  if (!approvalText.includes(mockRuntime.planApprovalId) || !approvalText.includes("创建测试文件")) {
    throw new Error(`plan approval dialog missing context: ${approvalText}`);
  }

  await page.getByRole("button", { name: "批准计划并继续执行" }).click();
  await dialog.waitFor({ state: "hidden", timeout: 5_000 });
  const planContinuedEvent = await waitForCondition(
    "runtime.plan_approval.model_continued event",
    () => mockRuntime.events.find((event) => event.event_type === "runtime.plan_approval.model_continued"),
    10_000,
  );
  await page.waitForFunction(
    (planContinuedSequence) =>
      Number(window.__ARGON_GUI_DEBUG__?.cursor || 0) >= Number(planContinuedSequence) &&
      window.__ARGON_GUI_DEBUG__?.run_status === "running",
    planContinuedEvent.sequence,
    { timeout: 10_000 },
  );
  const debugAfterPlanContinuedBeforeTerminal = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  const terminalEventAtPlanContinuedCheckpoint = mockRuntime.events.find(
    (event) =>
      event.event_type === "session.state_changed" &&
      event.payload?.to_state === "Completed" &&
      Number(event.sequence || 0) <= Number(debugAfterPlanContinuedBeforeTerminal?.cursor || 0),
  );
  await page.waitForFunction(
    () => document.body?.innerText.includes("计划审批链路已恢复并继续执行。"),
    null,
    { timeout: 10_000 },
  );
  await page.waitForFunction(
    () => window.__ARGON_GUI_DEBUG__?.run_status === "completed",
    null,
    { timeout: 10_000 },
  );
  const debugAfterCompletion = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  await textarea.fill("计划审批恢复后，立刻回答一个跟进问题。");
  await page.keyboard.press("Enter");
  await page.waitForFunction(
    () => document.body?.innerText.includes("计划审批恢复后的跟进已正常响应"),
    null,
    { timeout: 10_000 },
  );
  await page.waitForFunction(
    () => window.__ARGON_GUI_DEBUG__?.run_status === "completed",
    null,
    { timeout: 10_000 },
  );
  const debugAfterFollowup = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  await page.screenshot({ path: path.join(artifactDir, "completed.png"), fullPage: true });

  const finalText = await page.locator("body").innerText();
  const dialogCleared = await dialog.isHidden();
  const toolRequested = mockRuntime.events.filter((event) => event.event_type === "tool.call_requested").length;
  const toolCompleted = mockRuntime.events.filter((event) => event.event_type === "tool.call_completed").length;
  const planContinued = mockRuntime.events.some((event) => event.event_type === "runtime.plan_approval.model_continued");
  const followupCompleted = mockRuntime.events.some(
    (event) =>
      event.event_type === "assistant.message" &&
      String(event.payload?.content ?? "").includes("计划审批恢复后的跟进已正常响应"),
  );
  const blockingRequestFailures = requestFailures.filter((entry) => !String(entry.failure || "").includes("ERR_ABORTED"));
  const blockingConsole = consoleMessages.filter((entry) => entry.type === "error");
  finalReport.ok =
    mockRuntime.decisions.length === 1 &&
    mockRuntime.events.some((event) => event.event_type === "plan.approval_decided") &&
    planContinued &&
    Number(debugAfterPlanContinuedBeforeTerminal?.cursor || 0) >= Number(planContinuedEvent.sequence || 0) &&
    debugAfterPlanContinuedBeforeTerminal?.run_status === "running" &&
    !terminalEventAtPlanContinuedCheckpoint &&
    debugAfterCompletion?.run_status === "completed" &&
    mockRuntime.followupSubmitted() &&
    followupCompleted &&
    debugAfterFollowup?.run_status === "completed" &&
    toolRequested === 6 &&
    toolCompleted === 6 &&
    finalText.includes("计划审批链路已恢复并继续执行。") &&
    finalText.includes("计划审批恢复后的跟进已正常响应") &&
    !finalText.includes("等待当前审批处理完成") &&
    dialogCleared &&
    blockingRequestFailures.length === 0 &&
    blockingConsole.length === 0;
  finalReport.duration_ms = Date.now() - started;
  finalReport.event_count = mockRuntime.events.length;
  finalReport.decisions = mockRuntime.decisions;
  finalReport.dialog_cleared = dialogCleared;
  finalReport.plan_continued = planContinued;
  finalReport.plan_continued_sequence = planContinuedEvent.sequence;
  finalReport.debug_after_plan_continued_before_terminal = debugAfterPlanContinuedBeforeTerminal;
  finalReport.terminal_event_at_plan_continued_checkpoint = terminalEventAtPlanContinuedCheckpoint || null;
  finalReport.plan_resume_held_running_before_terminal =
    Number(debugAfterPlanContinuedBeforeTerminal?.cursor || 0) >= Number(planContinuedEvent.sequence || 0) &&
    debugAfterPlanContinuedBeforeTerminal?.run_status === "running" &&
    !terminalEventAtPlanContinuedCheckpoint;
  finalReport.debug_after_completion = debugAfterCompletion;
  finalReport.plan_resume_released_after_terminal = debugAfterCompletion?.run_status === "completed";
  finalReport.followup_submitted_after_plan_resume = mockRuntime.followupSubmitted();
  finalReport.followup_completed = followupCompleted;
  finalReport.debug_after_followup = debugAfterFollowup;
  finalReport.followup_done_visible = finalText.includes("计划审批恢复后的跟进已正常响应");
  finalReport.tool_requested = toolRequested;
  finalReport.tool_completed = toolCompleted;

  await fs.writeFile(
    path.join(artifactDir, "mock_runtime_events.jsonl"),
    mockRuntime.events.map((event) => JSON.stringify(event)).join("\n") + "\n",
  );
  if (!finalReport.ok) {
    throw new Error(`plan approval checks failed: ${JSON.stringify(finalReport)}`);
  }
} catch (error) {
  finalReport.ok = false;
  finalReport.error = String(error.stack || error.message || error);
  finalReport.duration_ms = Date.now() - started;
  console.error(finalReport.error);
} finally {
  const reportPath = path.join(artifactDir, "report.json");
  await fs.writeFile(reportPath, JSON.stringify(finalReport, null, 2));
  log(`report: ${reportPath}`);
  await cleanup(browser, mockRuntime);
  if (!finalReport.ok) process.exitCode = 1;
}
