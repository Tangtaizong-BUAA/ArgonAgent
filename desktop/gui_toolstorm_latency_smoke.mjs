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
const toolCalls = Math.max(1, Number(argValue("tool-calls") || "250"));
const latencyBudgetMs = Math.max(1, Number(argValue("latency-budget-ms") || "120"));
const minEventCount = Math.max(1, Number(argValue("min-events") || "1000"));
const largeOutputBytes = Math.max(0, Number(argValue("large-output-bytes") || "24000"));
const started = Date.now();
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `toolstorm-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[toolstorm +${elapsed}ms] ${message}`);
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
    event_id: `evt_${String(sequence).padStart(5, "0")}`,
    sequence,
    event_type: eventType,
    payload,
    created_at: new Date().toISOString(),
  };
}

function createMockRuntimeServer({ toolCallCount }) {
  const sessionId = "toolstorm_session_001";
  const events = [];
  let sequence = 0;
  let state = "Idle";
  let stormStarted = false;
  let stormDone = false;
  const timers = new Set();

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

  const startStorm = () => {
    if (stormStarted) {
      return;
    }
    stormStarted = true;
    state = "Executing";
    push("session.state_changed", { from_state: "Idle", to_state: "Executing" });
    push("model.context_budget", {
      prompt_tokens: 12_000,
      max_context_tokens: 128_000,
      budget_remaining: 116_000,
    });
    push("model.call_started", { call_id: "toolstorm_call", prompt_tokens_estimate: 1200 });
    for (let index = 0; index < toolCallCount; index += 1) {
      const delay = 20 + index * 18;
      schedule(delay, () => {
        const callId = `toolstorm_call_${String(index + 1).padStart(3, "0")}`;
        push("deepseek.tool_call.partial", {
          tool_call_id: callId,
          tool_id: "search.ripgrep",
          preview: `query ${index + 1}`,
        });
        push("tool.call_requested", {
          tool_call_id: callId,
          tool_id: "search.ripgrep",
        });
        push("tool.call_completed", {
          tool_call_id: callId,
          tool_id: "search.ripgrep",
          ok: true,
          duration_ms: 3,
        });
        if (index === 0 && largeOutputBytes > 0) {
          push("tool.result_recorded", {
            tool_call_id: callId,
            tool_id: "search.ripgrep",
            preview: `large-output:${"x".repeat(largeOutputBytes)}`,
            bytes: largeOutputBytes,
          });
        }
        if ((index + 1) % 20 === 0) {
          push("agent.loop_recovery", { iteration: index + 1, reason: "toolstorm_progress" });
        }
        push("model.stream_delta", {
          stream_id: "toolstorm_stream",
          delta_kind: "content",
          preview: `工具 ${index + 1} 完成。\n`,
          runtime_sanitized: true,
        });
      });
    }
    schedule(20 + toolCallCount * 18 + 120, () => {
      push("context.compaction.skipped", { reason: "below_threshold" });
      push("model.stream_completed", {
        stream_id: "toolstorm_stream",
        completion_tokens: toolCallCount * 4,
        reasoning_tokens: 0,
      });
      push("model.call_completed", { call_id: "toolstorm_call" });
      push("assistant.message", {
        content: `工具风暴完成：${toolCallCount} 次工具调用均已记录。`,
      });
      state = "Completed";
      stormDone = true;
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
            task_id: "toolstorm_task",
            workspace_root: body.workspace || ".",
            model_mode: body.model_mode || "qwen",
            autonomy_mode: body.autonomy_mode || "conservative",
            state,
          },
        });
        return;
      }
      if (url.pathname === "/runtime/submit-user-message" && req.method === "POST") {
        await readBody(req);
        startStorm();
        jsonResponse(res, 200, { ok: true, session_id: sessionId });
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
            model_mode: "qwen",
            autonomy_mode: "conservative",
            workspace_root: ".",
            pending_permission_count: 0,
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
    events,
    isDone: () => stormDone,
    close: () => {
      for (const timer of timers) {
        clearTimeout(timer);
      }
      server.close();
    },
  };
}

function percentile(values, ratio) {
  if (values.length === 0) {
    return 0;
  }
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.max(0, Math.ceil(sorted.length * ratio) - 1));
  return sorted[index];
}

async function waitForToolCount(page, count, timeoutMs = 20_000) {
  try {
    await page.waitForFunction(
      (expected) => {
        const text = document.body?.innerText || "";
        return (
          text.includes(`工具风暴完成：${expected} 次工具调用`) ||
          text.includes(`工具 ${expected}`) ||
          text.includes(`已完成 ${expected} 个任务`) ||
          text.includes(`共 ${expected} 次工具调用`)
        );
      },
      count,
      { timeout: timeoutMs },
    );
  } catch (error) {
    await page.screenshot({ path: path.join(artifactDir, "tool-count-timeout.png"), fullPage: true }).catch(() => {});
    const bodyText = await page.locator("body").innerText().catch(() => "");
    await fs.writeFile(path.join(artifactDir, "tool-count-timeout.txt"), bodyText, "utf8").catch(() => {});
    log(`tool count timeout; body=${bodyText.slice(0, 1200).replace(/\n/g, " | ")}`);
    throw error;
  }
}

async function waitForGuiCursorAtLeast(page, expectedCursor, timeoutMs = 10_000) {
  try {
    await page.waitForFunction(
      (expected) => {
        const debug = window.__ARGON_GUI_DEBUG__;
        return Boolean(debug && Number(debug.cursor || 0) >= expected);
      },
      expectedCursor,
      { timeout: timeoutMs },
    );
  } catch (error) {
    const debug = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null).catch(() => null);
    await fs.writeFile(
      path.join(artifactDir, "gui-cursor-timeout.json"),
      JSON.stringify({ expectedCursor, debug }, null, 2),
      "utf8",
    ).catch(() => {});
    throw error;
  }
}

async function measureInputLatency(page) {
  return await page.evaluate(async () => {
    const textarea = document.querySelector("textarea");
    if (!(textarea instanceof HTMLTextAreaElement)) {
      throw new Error("textarea not found");
    }
    const valueSetter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    const setTextareaValue = (value) => {
      if (valueSetter) {
        valueSetter.call(textarea, value);
      } else {
        textarea.value = value;
      }
      textarea.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: value }));
    };
    textarea.focus();
    setTextareaValue("");
    await new Promise((resolve) => requestAnimationFrame(resolve));
    await new Promise((resolve) => requestAnimationFrame(resolve));
    const samples = [];
    const chars = "abcdefghijklmnopqrstuvwxyz0123456789----";
    for (const char of chars) {
      const started = performance.now();
      setTextareaValue(textarea.value + char);
      await new Promise((resolve) => requestAnimationFrame(resolve));
      samples.push(performance.now() - started);
    }
    return {
      samples,
      final_value: textarea.value,
      expected_value: chars,
    };
  });
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
  latency_budget_ms: latencyBudgetMs,
  min_event_count: minEventCount,
  large_output_bytes: largeOutputBytes,
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
        provider: "qwen",
        baseUrl: "http://127.0.0.1:11434/v1",
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
  await textarea.fill(`触发 ${toolCalls} 工具调用风暴`);
  await page.keyboard.press("Enter");

  await page.waitForFunction(
    () => document.body?.innerText.includes("执行中") || document.body?.innerText.includes("运行 search.ripgrep"),
    null,
    { timeout: 10_000 },
  );
  const latency = await measureInputLatency(page);
  await waitForToolCount(page, toolCalls, 25_000);
  await waitForGuiCursorAtLeast(page, mockRuntime.events.length, 10_000);
  await page.screenshot({ path: path.join(artifactDir, "completed.png"), fullPage: true });

  const samples = latency.samples.map(Number).filter(Number.isFinite);
  const p95 = percentile(samples, 0.95);
  const max = samples.length > 0 ? Math.max(...samples) : 0;
  const toolRequested = mockRuntime.events.filter((event) => event.event_type === "tool.call_requested").length;
  const toolCompleted = mockRuntime.events.filter((event) => event.event_type === "tool.call_completed").length;
  const largeOutputEvents = mockRuntime.events.filter(
    (event) =>
      event.event_type === "tool.result_recorded" &&
      Number(event.payload?.bytes || 0) >= largeOutputBytes,
  ).length;
  const duplicateEventIds = mockRuntime.events.length - new Set(mockRuntime.events.map((event) => event.event_id)).size;
  const blockingRequestFailures = requestFailures.filter((entry) => !String(entry.failure || "").includes("ERR_ABORTED"));
  const blockingConsole = consoleMessages.filter((entry) => entry.type === "error");
  const guiDebug = await page.evaluate(() => window.__ARGON_GUI_DEBUG__ ?? null).catch(() => null);

  finalReport.ok =
    toolRequested === toolCalls &&
    toolCompleted === toolCalls &&
    mockRuntime.events.length >= minEventCount &&
    (largeOutputBytes === 0 || largeOutputEvents >= 1) &&
    Number(guiDebug?.cursor || 0) >= mockRuntime.events.length &&
    duplicateEventIds === 0 &&
    p95 <= latencyBudgetMs &&
    latency.final_value === latency.expected_value &&
    blockingRequestFailures.length === 0 &&
    blockingConsole.length === 0;
  finalReport.duration_ms = Date.now() - started;
  finalReport.runtime_base_url = runtimeBaseUrl;
  finalReport.web_base_url = webBaseUrl;
  finalReport.event_count = mockRuntime.events.length;
  finalReport.tool_requested = toolRequested;
  finalReport.tool_completed = toolCompleted;
  finalReport.large_output_events = largeOutputEvents;
  finalReport.duplicate_event_ids = duplicateEventIds;
  finalReport.gui_debug = guiDebug;
  finalReport.input_latency = {
    sample_count: samples.length,
    p95_ms: Number(p95.toFixed(2)),
    max_ms: Number(max.toFixed(2)),
    budget_ms: latencyBudgetMs,
    final_value: latency.final_value,
    expected_value: latency.expected_value,
  };
  await fs.writeFile(
    path.join(artifactDir, "mock_runtime_events.jsonl"),
    mockRuntime.events.map((event) => JSON.stringify(event)).join("\n") + "\n",
  );
  if (!finalReport.ok) {
    throw new Error(`toolstorm checks failed: ${JSON.stringify({
      toolRequested,
      toolCompleted,
      eventCount: mockRuntime.events.length,
      minEventCount,
      largeOutputEvents,
      guiCursor: guiDebug?.cursor,
      duplicateEventIds,
      p95,
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
