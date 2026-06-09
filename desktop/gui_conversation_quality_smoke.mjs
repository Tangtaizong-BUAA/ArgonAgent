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
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `conversation-quality-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const INTER_TOOL_TEXT = "工具之间的叙述必须显示：我正在继续查第二个文件。";
const LONG_STREAM_TEXT = [
  "这是一个较长的流式正文第一段，不能被最终 assistant.message 覆盖。",
  "这是一个较长的流式正文第二段，用来复现对话截断问题。",
  "这是一个较长的流式正文第三段，界面应该完整保留它。",
].join("\n\n");
const FINAL_SUPPLEMENT = "最终补充：不会覆盖前面长段落。";
const FOLLOWUP_DONE = "跟进已处理：已根据你的新要求改成简短结论。";
const INTERRUPT_ACK_DELAY_MS = 650;

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[conversation-quality +${elapsed}ms] ${message}`);
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
    event_id: `evt_quality_${String(sequence).padStart(5, "0")}`,
    sequence,
    event_type: eventType,
    payload,
    created_at: new Date().toISOString(),
  };
}

function createMockRuntimeServer() {
  const sessionId = "conversation_quality_session_001";
  const events = [];
  const timers = new Set();
  let sequence = 0;
  let state = "Idle";
  let turnActive = false;
  let firstTurnStarted = false;
  let followupSubmittedAfterInterrupt = false;
  let interruptRequests = 0;
  const submissions = [];

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

  const clearTimers = () => {
    for (const timer of timers) {
      clearTimeout(timer);
    }
    timers.clear();
  };

  const completeTurn = () => {
    turnActive = false;
    state = "Completed";
    push("session.state_changed", { from_state: "Executing", to_state: "Completed" });
  };

  const startFirstTurn = () => {
    firstTurnStarted = true;
    turnActive = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Idle", to_state: "Executing" });
    push("model.call_started", { call_id: "quality_first_call", prompt_tokens_estimate: 600 });
    push("model.stream_delta", {
      stream_id: "quality_first_stream",
      delta_kind: "content",
      preview: "我先读一个文件，然后继续说明下一步。\n",
      runtime_sanitized: true,
    });
    schedule(80, () => {
      push("tool.call_requested", { tool_call_id: "quality_tool_1", tool_id: "file.read" });
      push("tool.call_completed", { tool_call_id: "quality_tool_1", tool_id: "file.read", ok: true });
    });
    schedule(160, () => {
      push("model.stream_delta", {
        stream_id: "quality_first_stream",
        delta_kind: "content",
        preview: `${INTER_TOOL_TEXT}\n`,
        runtime_sanitized: true,
      });
    });
    schedule(250, () => {
      push("tool.call_requested", { tool_call_id: "quality_tool_2", tool_id: "search.ripgrep" });
      push("tool.call_completed", { tool_call_id: "quality_tool_2", tool_id: "search.ripgrep", ok: true });
    });
    schedule(340, () => {
      push("model.stream_delta", {
        stream_id: "quality_first_stream",
        delta_kind: "content",
        preview: `${LONG_STREAM_TEXT}\n`,
        runtime_sanitized: true,
      });
    });
    schedule(560, () => {
      push("model.stream_completed", {
        stream_id: "quality_first_stream",
        completion_tokens: 120,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "quality_first_call" });
      push("assistant.message", { content: FINAL_SUPPLEMENT });
      completeTurn();
    });
  };

  const startInterruptibleTurn = () => {
    turnActive = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Completed", to_state: "Executing" });
    push("model.call_started", { call_id: "quality_interruptible_call" });
    push("model.stream_delta", {
      stream_id: "quality_interruptible_stream",
      delta_kind: "content",
      preview: "这个长任务正在运行，稍后应该能被跟进打断。\n",
      runtime_sanitized: true,
    });
    for (let index = 1; index <= 20; index += 1) {
      schedule(200 + index * 180, () => {
        if (!turnActive) {
          return;
        }
        push("model.stream_delta", {
          stream_id: "quality_interruptible_stream",
          delta_kind: "content",
          preview: `仍在运行 ${index}。\n`,
          runtime_sanitized: true,
        });
      });
    }
  };

  const startFollowupTurn = () => {
    followupSubmittedAfterInterrupt = true;
    turnActive = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Cancelled", to_state: "Executing" });
    push("model.call_started", { call_id: "quality_followup_call" });
    schedule(80, () => {
      push("model.stream_delta", {
        stream_id: "quality_followup_stream",
        delta_kind: "content",
        preview: `${FOLLOWUP_DONE}\n`,
        runtime_sanitized: true,
      });
      push("model.stream_completed", {
        stream_id: "quality_followup_stream",
        completion_tokens: 32,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "quality_followup_call" });
      push("assistant.message", { content: FOLLOWUP_DONE });
      completeTurn();
    });
  };

  const interruptTurn = () => {
    clearTimers();
    turnActive = false;
    state = "Cancelled";
    push("runtime.turn_cancel_requested", { session_id: sessionId });
    push("session.state_changed", { from_state: "Executing", to_state: "Cancelled" });
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
        log(`runtime start-session state=${state}`);
        jsonResponse(res, 200, {
          ok: true,
          session: {
            session_id: sessionId,
            task_id: "conversation_quality_task",
            workspace_root: body.workspace || ".",
            model_mode: body.model_mode || "deepseek",
            autonomy_mode: body.autonomy_mode || "conservative",
            state,
          },
        });
        return;
      }
      if (url.pathname === "/runtime/submit-user-message" && req.method === "POST") {
        const body = await readBody(req);
        submissions.push(String(body.text ?? ""));
        log(`runtime submit-user-message active=${turnActive} state=${state} text=${String(body.text ?? "").slice(0, 40)}`);
        if (turnActive) {
          jsonResponse(res, 200, {
            ok: false,
            session_id: sessionId,
            error_code: "runtime_turn_in_progress",
          });
          return;
        }
        if (!firstTurnStarted) {
          startFirstTurn();
        } else if (state === "Cancelled") {
          startFollowupTurn();
        } else {
          startInterruptibleTurn();
        }
        jsonResponse(res, 200, { ok: true, session_id: sessionId });
        return;
      }
      if (url.pathname === "/runtime/interrupt-session" && req.method === "POST") {
        await readBody(req);
        log("runtime interrupt-session");
        interruptRequests += 1;
        setTimeout(() => {
          interruptTurn();
          jsonResponse(res, 200, { ok: true, session_id: sessionId, state });
        }, INTERRUPT_ACK_DELAY_MS);
        return;
      }
      if (url.pathname === "/runtime/stream-events") {
        const cursor = Number(url.searchParams.get("cursor") || "0");
        const selected = events.filter((event) => Number(event.sequence || 0) > cursor);
        const nextCursor = selected.length > 0 ? Number(selected[selected.length - 1].sequence || cursor) : cursor;
        jsonResponse(res, 200, {
          session_id: sessionId,
          from_cursor: cursor,
          next_cursor: nextCursor,
          has_more: false,
          events: selected,
          jsonl: selected.map((event) => JSON.stringify(event)).join("\n"),
        });
        return;
      }
      if (url.pathname === "/runtime/get-snapshot") {
        jsonResponse(res, 200, {
          session_id: sessionId,
          state,
          event_count: events.length,
          model_mode: "deepseek",
          autonomy_mode: "conservative",
          workspace_root: ".",
          pending_permission_count: 0,
          pending_plan_approval_count: 0,
        });
        return;
      }
      if (url.pathname === "/__report") {
        jsonResponse(res, 200, {
          events,
          submissions,
          followupSubmittedAfterInterrupt,
          interruptRequests,
        });
        return;
      }
      jsonResponse(res, 404, { ok: false, error: `not found: ${url.pathname}` });
    } catch (error) {
      jsonResponse(res, 500, { ok: false, error: String(error.message || error) });
    }
  });

  return { server };
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

async function setTextareaValue(page, value) {
  await page.evaluate((nextValue) => {
    const textarea = document.querySelector("textarea");
    if (!(textarea instanceof HTMLTextAreaElement)) {
      throw new Error("textarea not found");
    }
    const descriptor = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value");
    if (descriptor?.set) {
      descriptor.set.call(textarea, nextValue);
    } else {
      textarea.value = nextValue;
    }
    textarea.dispatchEvent(new InputEvent("input", {
      bubbles: true,
      inputType: "insertText",
      data: nextValue,
    }));
    textarea.focus();
  }, value);
}

async function waitForBodyText(page, text, screenshotName, timeoutMs = 12_000) {
  try {
    await page.waitForFunction((needle) => document.body.innerText.includes(needle), text, { timeout: timeoutMs });
  } catch (error) {
    await page.screenshot({ path: path.join(artifactDir, screenshotName), fullPage: true }).catch(() => {});
    const bodyText = await page.locator("body").innerText().catch(() => "");
    await fs.writeFile(path.join(artifactDir, `${screenshotName}.txt`), bodyText, "utf8").catch(() => {});
    log(`waitForBodyText failed for ${text}; body=${bodyText.slice(0, 1000).replace(/\n/g, " | ")}`);
    throw error;
  }
}

let browser;
try {
  const runtimePort = await findFreePort();
  const vitePort = await findFreePort();
  const { server } = createMockRuntimeServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(runtimePort, "127.0.0.1", resolve);
  });
  childProcesses.push({ label: "mock-runtime", child: { killed: false, kill: () => server.close() } });
  log(`mock runtime listening on ${runtimePort}`);

  spawnLogged("vite", "npm", ["run", "dev", "--", "--host", "127.0.0.1", "--port", String(vitePort)], {
    cwd: DESKTOP_DIR,
  });
  const appUrl = `http://127.0.0.1:${vitePort}`;
  await waitForWeb(appUrl, "vite");

  browser = await chromium.launch({
    headless: !headed,
    args: ["--window-size=1440,980"],
  });
  const context = await browser.newContext({
    viewport: { width: 1440, height: 980 },
    deviceScaleFactor: 1,
  });
  await context.addInitScript((port) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({
        provider: "deepseek",
        baseUrl: "https://api.deepseek.com/anthropic/v1/messages",
        modelId: "deepseek-v4-flash",
      }),
    );
    window.__ARGON_RUNTIME_BOOTSTRAP__ = {
      transport: "http",
      baseUrl: `http://127.0.0.1:${port}`,
      token: "",
      workspaceRoot: ".",
      port,
      logPath: "",
    };
  }, runtimePort);
  const page = await context.newPage();
  page.on("console", (message) => {
    const type = message.type();
    const text = message.text();
    consoleMessages.push({ type, text });
    if (type === "error") {
      log(`browser console error: ${text}`);
    }
  });
  page.on("requestfailed", (request) => {
    requestFailures.push({
      url: request.url(),
      failure: request.failure()?.errorText || "unknown",
    });
  });
  await page.goto(appUrl, { waitUntil: "domcontentloaded" });
  await page.screenshot({ path: path.join(artifactDir, "initial.png"), fullPage: true });

  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15_000 });
  await setTextareaValue(page, "复现工具间叙述、长流式正文和 final 覆盖问题。");
  await page.getByTitle("发送").click();
  await waitForBodyText(page, FINAL_SUPPLEMENT, "first-turn-timeout.png");
  await page.screenshot({ path: path.join(artifactDir, "first-turn-completed.png"), fullPage: true });
  const firstTurnText = await page.locator("body").innerText();

  await setTextareaValue(page, "启动一个很长的任务，我马上会跟进打断。");
  await page.getByTitle("发送").click();
  await waitForBodyText(page, "这个长任务正在运行", "interruptible-timeout.png", 8_000);
  await page.getByTitle("中断当前轮").click();
  await page.waitForTimeout(Math.max(120, Math.floor(INTERRUPT_ACK_DELAY_MS / 3)));
  const debugBeforeInterruptAck = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  await page.waitForFunction(
    () => window.__ARGON_GUI_DEBUG__?.run_status === "stopped",
    null,
    { timeout: 5_000 },
  );
  const debugAfterInterruptAck = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null);
  await setTextareaValue(page, "请打断上一轮，改成简短结论。");
  await page.keyboard.press("Enter");
  await waitForBodyText(page, FOLLOWUP_DONE, "followup-timeout.png");
  await page.screenshot({ path: path.join(artifactDir, "followup-completed.png"), fullPage: true });
  const finalText = await page.locator("body").innerText();

  const reportResponse = await fetch(`http://127.0.0.1:${runtimePort}/__report`);
  const runtimeReport = await reportResponse.json();
  const report = {
    ok: true,
    artifactDir,
    checks: {
      interToolTextVisible: firstTurnText.includes(INTER_TOOL_TEXT),
      longStreamVisible: firstTurnText.includes(LONG_STREAM_TEXT.split("\n\n")[0]) &&
        firstTurnText.includes(LONG_STREAM_TEXT.split("\n\n")[2]),
      finalSupplementVisible: firstTurnText.includes(FINAL_SUPPLEMENT),
      noRetryAgainCopy: !finalText.includes("请再发送一次"),
      interruptNoticeVisible: finalText.includes("上一轮仍在执行，正在中断并提交你的跟进。") ||
        finalText.includes("已中断当前轮"),
      followupDoneVisible: finalText.includes(FOLLOWUP_DONE),
      followupSubmittedAfterInterrupt: runtimeReport.followupSubmittedAfterInterrupt === true,
      explicitInterruptRequested: runtimeReport.interruptRequests === 1,
      stopHeldRunningBeforeAck: debugBeforeInterruptAck?.run_status === "running",
      stopReleasedAfterAck: debugAfterInterruptAck?.run_status === "stopped",
      consoleErrors: consoleMessages.filter((entry) => entry.type === "error"),
      requestFailures,
    },
    debugBeforeInterruptAck,
    debugAfterInterruptAck,
    submissions: runtimeReport.submissions,
    event_count: runtimeReport.events.length,
  };
  report.ok = Object.entries(report.checks).every(([key, value]) => {
    if (key === "consoleErrors" || key === "requestFailures") {
      return Array.isArray(value) && value.length === 0;
    }
    return value === true;
  });
  await fs.writeFile(path.join(artifactDir, "report.json"), JSON.stringify(report, null, 2), "utf8");
  await fs.writeFile(
    path.join(artifactDir, "mock_runtime_events.jsonl"),
    runtimeReport.events.map((event) => JSON.stringify(event)).join("\n"),
    "utf8",
  );
  log(`report written to ${path.join(artifactDir, "report.json")}`);
  if (!report.ok) {
    throw new Error(`conversation quality smoke failed: ${JSON.stringify(report.checks, null, 2)}`);
  }
  log("conversation quality smoke passed");
} finally {
  await cleanup(browser);
}
