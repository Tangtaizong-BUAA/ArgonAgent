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

function argValue(name) {
  const prefix = `--${name}=`;
  return process.argv.find((arg) => arg.startsWith(prefix))?.slice(prefix.length) || "";
}

const args = new Set(process.argv.slice(2));
const headed = args.has("--headed");
const keepOpen = args.has("--keep-open");
const pauseAtApproval = args.has("--pause-at-approval");
const toolCalls = Math.max(8, Number(argValue("tool-calls") || "48"));
const approvalTimeoutMs = Math.max(1000, Number(argValue("approval-timeout-ms") || "12000"));
const started = Date.now();
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `permission-longtask-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[permission-longtask +${elapsed}ms] ${message}`);
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
    stdio: ["ignore", "pipe", "pipe"],
    ...options,
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
    if (value) {
      return value;
    }
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
    event_id: `evt_perm_${String(sequence).padStart(5, "0")}`,
    sequence,
    event_type: eventType,
    payload,
    created_at: new Date().toISOString(),
  };
}

function createMockRuntimeServer({ toolCallCount }) {
  const sessionId = "permission_longtask_session_001";
  const permissionId = "perm_shell_001";
  const events = [];
  const timers = new Set();
  let sequence = 0;
  let state = "Idle";
  let startedRun = false;
  let awaitingPermission = false;
  let permissionSubmitted = false;
  let completed = false;
  let followupSubmitted = false;
  const approvalRequests = [];

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
      const callId = `longtask_read_${String(index).padStart(3, "0")}`;
      push("tool.call_requested", {
        tool_call_id: callId,
        tool_id: "file.read",
        args_preview: `{"path":"src/file_${index}.rs"}`,
      });
      push("tool.call_completed", {
        tool_call_id: callId,
        tool_id: "file.read",
        ok: true,
        duration_ms: 4,
      });
      push("model.stream_delta", {
        stream_id: "permission_longtask_stream",
        delta_kind: "content",
        preview: `已审阅文件 ${index}。\n`,
        runtime_sanitized: true,
      });
    });
  };

  const startRun = () => {
    if (startedRun) {
      return;
    }
    startedRun = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Idle", to_state: "Executing" });
    push("model.context_budget", {
      estimated_request_tokens: 17_400,
      estimated_total_tokens: 18_500,
      target_limit_tokens: 128_000,
      hard_limit_tokens: 128_000,
      compaction_threshold_tokens: 96_000,
    });
    push("model.call_started", { call_id: "permission_longtask_model_call" });
    push("model.stream_delta", {
      stream_id: "permission_longtask_stream",
      delta_kind: "content",
      preview: "我会先审阅关键文件，然后在需要执行验证命令前请求审批。\n",
      runtime_sanitized: true,
    });
    const beforeApproval = Math.floor(toolCallCount / 2);
    for (let index = 1; index <= beforeApproval; index += 1) {
      pushTool(index, 60 + index * 45);
    }
    schedule(60 + beforeApproval * 45 + 180, () => {
      awaitingPermission = true;
      state = "WaitingForToolApproval";
      push("permission.requested", {
        permission_id: permissionId,
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "npm --prefix desktop run test:runtime-event-replay",
        path_preview: "desktop/",
        risk_level: "medium",
      });
      push("permission.context", {
        permission_id: permissionId,
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "npm --prefix desktop run test:runtime-event-replay",
        path_preview: "desktop/",
        risk_level: "medium",
      });
    });
  };

  const continueAfterApproval = (decision) => {
    if (!awaitingPermission || permissionSubmitted) {
      return;
    }
    permissionSubmitted = true;
    awaitingPermission = false;
    state = "Executing";
    push("runtime.permission_submission.accepted", {
      permission_id: permissionId,
      decision,
    });
    push("permission.decided", {
      permission_id: permissionId,
      decision,
    });
    push("runtime.permission_resume.started", {
      permission_id: permissionId,
      tool_call_id: "approved_command_001",
      tool_id: "shell.command",
    });
    push("tool.call_requested", {
      tool_call_id: "approved_command_001",
      tool_id: "shell.command",
      args_preview: "npm --prefix desktop run test:runtime-event-replay",
    });
    schedule(180, () => {
      push("tool.call_completed", {
        tool_call_id: "approved_command_001",
        tool_id: "shell.command",
        ok: true,
        duration_ms: 180,
      });
      push("runtime.permission_resume.completed", {
        permission_id: permissionId,
        tool_call_id: "approved_command_001",
        tool_id: "shell.command",
        ok: true,
      });
      const beforeApproval = Math.floor(toolCallCount / 2);
      for (let index = beforeApproval + 1; index <= toolCallCount; index += 1) {
        pushTool(index, 220 + (index - beforeApproval) * 38);
      }
      schedule(220 + (toolCallCount - beforeApproval) * 38 + 260, () => {
        push("model.stream_completed", {
          stream_id: "permission_longtask_stream",
          completion_tokens: toolCallCount * 6,
          reasoning_tokens: 0,
        });
        push("model.call_completed", { call_id: "permission_longtask_model_call" });
        push("assistant.message", {
          content: `长任务完成：${toolCallCount} 个文件工具调用和 1 个审批命令均已完成。`,
        });
        completed = true;
        state = "Completed";
        push("session.state_changed", { from_state: "Executing", to_state: "Completed" });
      });
    });
  };

  const startFollowup = () => {
    followupSubmitted = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Completed", to_state: "Executing" });
    push("model.call_started", { call_id: "permission_longtask_followup_call" });
    schedule(80, () => {
      push("model.stream_delta", {
        stream_id: "permission_longtask_followup_stream",
        delta_kind: "content",
        preview: "审批恢复后的跟进已正常响应。\n",
        runtime_sanitized: true,
      });
      push("model.stream_completed", {
        stream_id: "permission_longtask_followup_stream",
        completion_tokens: 24,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "permission_longtask_followup_call" });
      push("assistant.message", { content: "审批恢复后的跟进已正常响应。" });
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
      if (url.pathname === "/health") {
        jsonResponse(res, 200, { ok: true });
        return;
      }
      if (url.pathname === "/runtime/list-commands") {
        jsonResponse(res, 200, { commands: ["/plan", "/review", "/status", "/run"] });
        return;
      }
      if (url.pathname === "/runtime/start-session" && req.method === "POST") {
        const body = await readBody(req);
        jsonResponse(res, 200, {
          ok: true,
          session: {
            session_id: sessionId,
            task_id: "permission_longtask_task",
            workspace_root: body.workspace || ".",
            model_mode: body.model_mode || "deepseek",
            autonomy_mode: body.autonomy_mode || "conservative",
            state,
          },
        });
        return;
      }
      if (url.pathname === "/runtime/submit-user-message" && req.method === "POST") {
        await readBody(req);
        if (completed) {
          startFollowup();
        } else {
          startRun();
        }
        jsonResponse(res, 200, { ok: true, session_id: sessionId });
        return;
      }
      if (url.pathname === "/runtime/submit-permission-decision" && req.method === "POST") {
        const body = await readBody(req);
        approvalRequests.push(body);
        if (body.permission_id !== permissionId) {
          jsonResponse(res, 409, {
            ok: false,
            session_id: sessionId,
            error_code: "permission_id_mismatch",
          });
          return;
        }
        continueAfterApproval(String(body.decision || "allow_once"));
        jsonResponse(res, 200, {
          ok: true,
          session_id: sessionId,
          permission_id: permissionId,
          tool_call_id: "approved_command_001",
          tool_id: "shell.command",
          resume_strategy: "execute_pending_tool",
          tool_executed: true,
          model_continuation_required: true,
        });
        return;
      }
      if (url.pathname === "/runtime/stream-events") {
        const cursor = Number(url.searchParams.get("cursor") || "0");
        const maxEvents = Number(url.searchParams.get("max_events") || "200");
        const nextEvents = events.filter((event) => Number(event.sequence || 0) > cursor).slice(0, maxEvents);
        const nextCursor = nextEvents.length > 0 ? Number(nextEvents[nextEvents.length - 1].sequence) : cursor;
        jsonResponse(res, 200, {
          session_id: sessionId,
          from_cursor: cursor,
          next_cursor: nextCursor,
          has_more: events.some((event) => Number(event.sequence || 0) > nextCursor),
          events: nextEvents,
          jsonl: nextEvents.map((event) => JSON.stringify(event)).join("\n"),
        });
        return;
      }
      if (url.pathname === "/runtime/get-snapshot") {
        jsonResponse(res, 200, {
          snapshot: {
            session_id: sessionId,
            state,
            event_count: events.length,
            model_mode: "deepseek",
            autonomy_mode: "conservative",
            workspace_root: ".",
            pending_permission_count: awaitingPermission ? 1 : 0,
            pending_plan_approval_count: 0,
          },
        });
        return;
      }
      if (url.pathname === "/runtime/export-events" && req.method === "POST") {
        jsonResponse(res, 200, {
          ok: true,
          session_id: sessionId,
          path: path.join(artifactDir, "mock_runtime_events.jsonl"),
        });
        return;
      }
      jsonResponse(res, 404, { error: "not_found", path: url.pathname });
    } catch (error) {
      jsonResponse(res, 500, { error: String(error.message || error) });
    }
  });

  return {
    server,
    sessionId,
    permissionId,
    events,
    approvalRequests,
    followupSubmitted: () => followupSubmitted,
    isCompleted: () => completed,
    close: () => {
      for (const timer of timers) {
        clearTimeout(timer);
      }
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
    if (!child.killed) {
      child.kill("SIGTERM");
    }
  }
}

let browser;
let mockRuntime;
const finalReport = {
  ok: false,
  artifact_dir: artifactDir,
  headed,
  tool_calls: toolCalls,
  console_errors: consoleMessages,
  request_failures: requestFailures,
};

try {
  const runtimePort = await findFreePort();
  const webPort = await findFreePort();
  const runtimeBaseUrl = `http://127.0.0.1:${runtimePort}`;
  const webBaseUrl = `http://127.0.0.1:${webPort}`;
  log(`artifacts: ${artifactDir}`);

  mockRuntime = createMockRuntimeServer({ toolCallCount: toolCalls });
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

  browser = await chromium.launch({
    headless: !headed,
    args: ["--window-size=1440,980"],
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 980 },
    deviceScaleFactor: 1,
  });
  await context.addInitScript(({ runtimeBaseUrl, runtimePort }) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({
        provider: "deepseek",
        baseUrl: "https://api.deepseek.com/anthropic/v1/messages",
      }),
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

  await page.goto(webBaseUrl, { waitUntil: "domcontentloaded" });
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15_000 });
  await page.screenshot({ path: path.join(artifactDir, "initial.png"), fullPage: true });
  await textarea.fill("执行一个长任务：审阅很多文件，必要时请求 shell 审批，然后继续完成。");
  await page.keyboard.press("Enter");

  await page.getByRole("dialog", { name: "权限审批" }).waitFor({ state: "visible", timeout: approvalTimeoutMs });
  await page.screenshot({ path: path.join(artifactDir, "approval.png"), fullPage: true });
  const approvalText = await page.getByRole("dialog", { name: "权限审批" }).innerText();
  const approvalPageText = await page.locator("body").innerText();
  if (!approvalText.includes(mockRuntime.permissionId) || !approvalText.includes("npm --prefix desktop")) {
    throw new Error(`approval card missing expected context: ${approvalText}`);
  }
  if (!approvalPageText.includes("等待审批")) {
    throw new Error("approval state is visible, but the dashboard/topbar does not show 等待审批");
  }
  if (pauseAtApproval) {
    log("paused at approval; waiting for external click on 允许一次");
    await page.waitForFunction(
      () => document.body?.innerText.includes("权限已处理: perm_shell_001"),
      null,
      { timeout: 60_000 },
    );
  } else {
    await page.getByRole("button", { name: "允许本次权限请求" }).click();
  }

  await page.waitForFunction(
    () => window.__ARGON_GUI_DEBUG__?.run_status === "running",
    null,
    { timeout: 10_000 },
  );
  const resumeCompletedEvent = await waitForCondition(
    "runtime.permission_resume.completed event",
    () => mockRuntime.events.find((event) => event.event_type === "runtime.permission_resume.completed"),
    10_000,
  );
  await page.waitForFunction(
    (resumeCompletedSequence) =>
      Number(window.__ARGON_GUI_DEBUG__?.cursor || 0) >= Number(resumeCompletedSequence) &&
      window.__ARGON_GUI_DEBUG__?.run_status === "running",
    resumeCompletedEvent.sequence,
    { timeout: 10_000 },
  );
  const debugAfterResumeCompletedBeforeTerminal = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  const terminalEventAtResumeCheckpoint = mockRuntime.events.find(
    (event) =>
      event.event_type === "session.state_changed" &&
      event.payload?.to_state === "Completed" &&
      Number(event.sequence || 0) <= Number(debugAfterResumeCompletedBeforeTerminal?.cursor || 0),
  );
  await page.waitForFunction(
    () => !document.body?.innerText.includes("perm_shell_001") || document.body?.innerText.includes("权限已处理"),
    null,
    { timeout: 10_000 },
  );
  await page.waitForFunction(
    (expected) => document.body?.innerText.includes(`长任务完成：${expected} 个文件工具调用`),
    toolCalls,
    { timeout: 25_000 },
  );
  await page.waitForFunction(
    () => window.__ARGON_GUI_DEBUG__?.run_status === "completed",
    null,
    { timeout: 10_000 },
  );
  const debugAfterCompletion = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  await textarea.fill("审批恢复完成后，立刻回答一个跟进问题。");
  await page.keyboard.press("Enter");
  await page.waitForFunction(
    () => document.body?.innerText.includes("审批恢复后的跟进已正常响应"),
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
  await page.getByRole("button", { name: "诊断" }).click();
  await page.waitForFunction(
    () =>
      document.body?.innerText.includes("诊断时间线") &&
      document.body?.innerText.includes("上下文压力") &&
      document.body?.innerText.includes("审批恢复"),
    null,
    { timeout: 5000 },
  );
  await page.screenshot({ path: path.join(artifactDir, "diagnostics.png"), fullPage: true });

  const finalText = await page.locator("body").innerText();
  const diagnosticsVisible =
    finalText.includes("诊断时间线") &&
    finalText.includes("上下文压力") &&
    finalText.includes("审批恢复");
  const toolRequested = mockRuntime.events.filter((event) => event.event_type === "tool.call_requested").length;
  const toolCompleted = mockRuntime.events.filter((event) => event.event_type === "tool.call_completed").length;
  const permissionRequested = mockRuntime.events.some((event) => event.event_type === "permission.requested");
  const permissionDecided = mockRuntime.events.some((event) => event.event_type === "permission.decided");
  const resumeCompleted = mockRuntime.events.some((event) => event.event_type === "runtime.permission_resume.completed");
  const followupCompleted = mockRuntime.events.some(
    (event) =>
      event.event_type === "assistant.message" &&
      String(event.payload?.content ?? "").includes("审批恢复后的跟进已正常响应"),
  );
  const blockingRequestFailures = requestFailures.filter((entry) => !String(entry.failure || "").includes("ERR_ABORTED"));
  const blockingConsole = consoleMessages.filter((entry) => entry.type === "error");

  finalReport.ok =
    permissionRequested &&
    permissionDecided &&
    resumeCompleted &&
    Number(debugAfterResumeCompletedBeforeTerminal?.cursor || 0) >= Number(resumeCompletedEvent.sequence || 0) &&
    debugAfterResumeCompletedBeforeTerminal?.run_status === "running" &&
    !terminalEventAtResumeCheckpoint &&
    debugAfterCompletion?.run_status === "completed" &&
    mockRuntime.followupSubmitted() &&
    followupCompleted &&
    debugAfterFollowup?.run_status === "completed" &&
    mockRuntime.approvalRequests.length === 1 &&
    toolRequested === toolCalls + 1 &&
    toolCompleted === toolCalls + 1 &&
    finalText.includes(`长任务完成：${toolCalls} 个文件工具调用`) &&
    diagnosticsVisible &&
    !finalText.includes("正在提交并恢复工具执行") &&
    blockingRequestFailures.length === 0 &&
    blockingConsole.length === 0;
  finalReport.duration_ms = Date.now() - started;
  finalReport.runtime_base_url = runtimeBaseUrl;
  finalReport.web_base_url = webBaseUrl;
  finalReport.event_count = mockRuntime.events.length;
  finalReport.approval_requests = mockRuntime.approvalRequests;
  finalReport.tool_requested = toolRequested;
  finalReport.tool_completed = toolCompleted;
  finalReport.permission_requested = permissionRequested;
  finalReport.permission_decided = permissionDecided;
  finalReport.resume_completed = resumeCompleted;
  finalReport.resume_completed_sequence = resumeCompletedEvent.sequence;
  finalReport.debug_after_resume_completed_before_terminal = debugAfterResumeCompletedBeforeTerminal;
  finalReport.terminal_event_at_resume_checkpoint = terminalEventAtResumeCheckpoint || null;
  finalReport.approval_resume_held_running_before_terminal =
    Number(debugAfterResumeCompletedBeforeTerminal?.cursor || 0) >= Number(resumeCompletedEvent.sequence || 0) &&
    debugAfterResumeCompletedBeforeTerminal?.run_status === "running" &&
    !terminalEventAtResumeCheckpoint;
  finalReport.debug_after_completion = debugAfterCompletion;
  finalReport.approval_resume_released_after_terminal =
    debugAfterCompletion?.run_status === "completed";
  finalReport.followup_submitted_after_approval_resume = mockRuntime.followupSubmitted();
  finalReport.followup_completed = followupCompleted;
  finalReport.debug_after_followup = debugAfterFollowup;
  finalReport.followup_done_visible = finalText.includes("审批恢复后的跟进已正常响应");
  finalReport.diagnostics_visible = diagnosticsVisible;

  await fs.writeFile(
    path.join(artifactDir, "mock_runtime_events.jsonl"),
    mockRuntime.events.map((event) => JSON.stringify(event)).join("\n") + "\n",
  );
  if (!finalReport.ok) {
    throw new Error(`permission longtask checks failed: ${JSON.stringify({
      approvalRequests: mockRuntime.approvalRequests.length,
      toolRequested,
      toolCompleted,
      permissionRequested,
      permissionDecided,
      resumeCompleted,
      resumeCompletedSequence: resumeCompletedEvent.sequence,
      debugAfterResumeCompletedBeforeTerminal,
      terminalEventAtResumeCheckpoint,
      debugAfterCompletion,
      followupSubmitted: mockRuntime.followupSubmitted(),
      followupCompleted,
      debugAfterFollowup,
      diagnosticsVisible,
      blockingRequestFailures: blockingRequestFailures.length,
      blockingConsole: blockingConsole.length,
    })}`);
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
  if (!finalReport.ok) {
    process.exitCode = 1;
  }
}
