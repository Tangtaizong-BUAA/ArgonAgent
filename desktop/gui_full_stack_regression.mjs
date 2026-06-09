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
const pauseAtApprovals = args.has("--pause-at-approvals");
const provider = argValue("provider") || "deepseek";
const requestedRounds = Math.min(6, Math.max(4, Number(argValue("rounds") || "6")));
const toolCalls = Math.max(16, Number(argValue("tool-calls") || "36"));
const started = Date.now();
const runId = new Date().toISOString().replace(/[:.]/g, "-");
const artifactDir = path.join(DESKTOP_DIR, ".gui-smoke-runs", `full-stack-${runId}`);
await fs.mkdir(artifactDir, { recursive: true });

const childProcesses = [];
const consoleMessages = [];
const requestFailures = [];

function log(message) {
  const elapsed = String(Date.now() - started).padStart(5, " ");
  console.log(`[full-regression +${elapsed}ms] ${message}`);
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
    event_id: `evt_full_${String(sequence).padStart(5, "0")}`,
    sequence,
    event_type: eventType,
    payload,
    created_at: new Date().toISOString(),
  };
}

function createMockRuntimeServer({ rounds, toolCallCount }) {
  const sessionId = "full_stack_regression_session_001";
  const planApprovalId = "full_stack_plan_approval_001";
  const permissionId = "full_stack_shell_permission_001";
  const events = [];
  const timers = new Set();
  const userMessages = [];
  const planDecisions = [];
  const permissionDecisions = [];
  let sequence = 0;
  let state = "Idle";
  let turnIndex = 0;
  let awaitingPlan = false;
  let awaitingPermission = false;
  let completedTurns = 0;

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

  const startExecuting = (label) => {
    state = "Executing";
    push("session.state_changed", { from_state: "Completed", to_state: "Executing", reason: label });
    push("model.context_budget", {
      estimated_request_tokens: 19_000 + turnIndex * 1100,
      estimated_total_tokens: 24_000 + turnIndex * 1600,
      target_limit_tokens: 256_000,
      hard_limit_tokens: 256_000,
      compaction_threshold_tokens: 180_000,
    });
    push("model.call_started", { call_id: `full_stack_call_${turnIndex}`, provider });
  };

  const finishTurn = (content, extraPayload = {}) => {
    push("model.stream_completed", {
      stream_id: `full_stack_stream_${turnIndex}`,
      completion_tokens: 128 + turnIndex * 12,
      reasoning_tokens: 0,
    });
    push("model.call_completed", { call_id: `full_stack_call_${turnIndex}` });
    push("assistant.message", { content, ...extraPayload });
    state = "Completed";
    completedTurns += 1;
    push("session.state_changed", { from_state: "Executing", to_state: "Completed", turn_index: turnIndex });
  };

  const stream = (preview, delayMs) => {
    schedule(delayMs, () => {
      push("model.stream_delta", {
        stream_id: `full_stack_stream_${turnIndex}`,
        delta_kind: "content",
        preview,
        runtime_sanitized: true,
      });
    });
  };

  const pushTool = (toolId, callId, argsPreview, delayMs, ok = true) => {
    schedule(delayMs, () => {
      push("tool.call_requested", {
        tool_call_id: callId,
        tool_id: toolId,
        args_preview: argsPreview,
      });
      push("tool.call_completed", {
        tool_call_id: callId,
        tool_id: toolId,
        ok,
        duration_ms: 5 + (delayMs % 23),
      });
    });
  };

  const runGreeting = () => {
    startExecuting("greeting");
    stream("你好，我已经接入真实 GUI 回归场景。\n", 40);
    stream("这一轮不调用工具，只验证普通对话可以正常收尾。\n", 90);
    schedule(180, () => {
      finishTurn("你好，我已经准备好。普通对话收尾正常，下一轮可以进入计划审批。");
    });
  };

  const runPlanApproval = () => {
    startExecuting("plan_approval");
    stream("我先进入计划模式，形成可审批的执行方案。\n", 40);
    schedule(130, () => {
      awaitingPlan = true;
      state = "WaitingForPlanApproval";
      push("plan.mode_entered", {
        plan_approval_id: planApprovalId,
        plan_preview: [
          "## 计划：完整 GUI 回归",
          "",
          "1. 读取项目结构并记录证据",
          "2. 创建测试计划并等待批准",
          "3. 执行工具调用与后续验证",
        ].join("\n"),
      });
      push("plan.approval_requested", {
        plan_approval_id: planApprovalId,
        goal: "验证 plan 设定并执行链路",
      });
      push("session.state_changed", { from_state: "Executing", to_state: "WaitingForPlanApproval" });
    });
  };

  const continuePlan = (body) => {
    if (!awaitingPlan) return;
    awaitingPlan = false;
    planDecisions.push(body);
    state = "Executing";
    push("plan.approval_decided", { plan_approval_id: planApprovalId, decision: "approve" });
    push("runtime.plan_approval.model_continued", { plan_approval_id: planApprovalId });
    push("session.state_changed", { from_state: "WaitingForPlanApproval", to_state: "Executing" });
    push("plan.mode_exited", { plan_approval_id: planApprovalId, reason: "approved" });
    pushTool("file.read", "plan_read_001", '{"path":"VoiceNote/Package.swift"}', 80);
    pushTool("search.ripgrep", "plan_search_001", "VoiceNote Tests", 140);
    stream("计划已批准，我正在执行读取和搜索。\n", 180);
    schedule(320, () => {
      finishTurn("计划审批已通过并执行：读取、搜索和计划退出事件都已进入 GUI。");
    });
  };

  const runPermissionRound = () => {
    startExecuting("permission_shell");
    stream("我会先读文件，然后申请 shell 权限运行验证命令。\n", 30);
    for (let index = 1; index <= 6; index += 1) {
      pushTool(
        "file.read",
        `perm_read_${String(index).padStart(3, "0")}`,
        `{"path":"VoiceNote/Sources/File${index}.swift"}`,
        50 + index * 38,
      );
    }
    schedule(380, () => {
      awaitingPermission = true;
      state = "WaitingForToolApproval";
      push("permission.requested", {
        permission_id: permissionId,
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "swift test --package-path VoiceNote",
        path_preview: "VoiceNote/",
        risk_level: "medium",
      });
      push("permission.context", {
        permission_id: permissionId,
        request_type: "command",
        tool_id: "shell.command",
        args_preview: "swift test --package-path VoiceNote",
        path_preview: "VoiceNote/",
        risk_level: "medium",
      });
      push("session.state_changed", { from_state: "Executing", to_state: "WaitingForToolApproval" });
    });
  };

  const continuePermission = (body) => {
    if (!awaitingPermission) return;
    awaitingPermission = false;
    permissionDecisions.push(body);
    state = "Executing";
    push("runtime.permission_submission.accepted", { permission_id: permissionId, decision: body.decision || "allow_once" });
    push("permission.decided", { permission_id: permissionId, decision: body.decision || "allow_once" });
    push("runtime.permission_resume.started", {
      permission_id: permissionId,
      tool_call_id: "approved_shell_001",
      tool_id: "shell.command",
    });
    push("session.state_changed", { from_state: "WaitingForToolApproval", to_state: "Executing" });
    push("tool.call_requested", {
      tool_call_id: "approved_shell_001",
      tool_id: "shell.command",
      args_preview: "swift test --package-path VoiceNote",
    });
    schedule(180, () => {
      push("tool.call_completed", {
        tool_call_id: "approved_shell_001",
        tool_id: "shell.command",
        ok: true,
        duration_ms: 180,
      });
      push("runtime.permission_resume.completed", {
        permission_id: permissionId,
        tool_call_id: "approved_shell_001",
        tool_id: "shell.command",
        ok: true,
      });
      stream("shell 验证已恢复完成，继续整理结果。\n", 60);
      finishTurn("shell 权限恢复完成：工具卡片应收口，stdout 不应伪装成助手正文。");
    });
  };

  const runSubagentRound = () => {
    startExecuting("subagent");
    stream("我会派发一个只读子 Agent 检查测试覆盖面。\n", 40);
    pushTool(
      "task.dispatch",
      "task_dispatch_001",
      '{"agent_type":"reviewer","prompt":"检查 VoiceNote 测试覆盖"}',
      80,
    );
    schedule(150, () => {
      push("subagent.spawned", {
        subagent_id: "subagent_review_001",
        parent_session_id: sessionId,
        agent_type: "reviewer",
      });
      push("subagent.child_created", {
        subagent_id: "subagent_review_001",
        agent_type: "reviewer",
      });
      push("subagent.message_sent", {
        subagent_id: "subagent_review_001",
        content_preview: "检查 VoiceNote 测试覆盖",
      });
      push("subagent.model_turn_started", {
        subagent_id: "subagent_review_001",
        model_role: "reviewer",
      });
    });
    schedule(260, () => {
      push("subagent.tool_completed", {
        subagent_id: "subagent_review_001",
        tool_id: "search.ripgrep",
        ok: true,
      });
      push("subagent.summary_recorded", {
        subagent_id: "subagent_review_001",
        summary: "发现 ConfigTests 与 service mock 覆盖最关键。",
        artifact_refs: ["ref://subagent/subagent_review_001/summary"],
      });
      push("subagent.completed", {
        subagent_id: "subagent_review_001",
        summary: "子 Agent 已完成覆盖面审查。",
        artifact_refs: [
          "ref://subagent/subagent_review_001/summary",
          "ref://subagent/subagent_review_001/evidence/search-ripgrep",
        ],
      });
      finishTurn("子 Agent 回合完成：父会话只接收折叠摘要，诊断里应出现子 Agent 分类。");
    });
  };

  const runToolstormRound = () => {
    startExecuting("toolstorm");
    stream("## 验证摘要\n\n", 30);
    stream("| 阶段 | 状态 |\n| --- | --- |\n| 工具风暴 | 进行中 |\n\n", 80);
    for (let index = 1; index <= toolCallCount; index += 1) {
      const toolId = index % 5 === 0 ? "file.write" : index % 3 === 0 ? "shell.command" : "search.ripgrep";
      const callId = `storm_${String(index).padStart(3, "0")}`;
      const preview =
        toolId === "file.write"
          ? `{"path":"VoiceNote/Tests/Generated${index}.swift"}`
          : toolId === "shell.command"
            ? "swift test --help"
            : `VoiceNote pattern ${index}`;
      pushTool(toolId, callId, preview, 30 + index * 14);
      if (index % 9 === 0) {
        schedule(35 + index * 14, () => {
          push("deepseek.tool_call.partial", {
            tool_call_id: callId,
            tool_id: toolId,
            preview,
          });
        });
      }
      if (index % 12 === 0) {
        stream(`- 已完成第 ${index} 批工具调用。\n`, 55 + index * 14);
      }
    }
    schedule(80 + toolCallCount * 14, () => {
      push("context.compaction.skipped", { reason: "below_threshold" });
      finishTurn(`工具风暴完成：${toolCallCount} 次工具调用、Markdown 流式表格和上下文状态都已记录。`);
    });
  };

  const runFollowupRound = () => {
    startExecuting("followup_release");
    stream("上一轮已释放，我现在可以回答新的追问。\n", 40);
    schedule(140, () => {
      finishTurn("后续追问正常响应：上一轮完成后没有卡住，也没有残留审批或执行态。");
    });
  };

  const startNextTurn = (body) => {
    userMessages.push(body);
    turnIndex += 1;
    if (turnIndex === 1) runGreeting();
    else if (turnIndex === 2) runPlanApproval();
    else if (turnIndex === 3) runPermissionRound();
    else if (turnIndex === 4 && rounds >= 4) runSubagentRound();
    else if (turnIndex === 5 && rounds >= 5) runToolstormRound();
    else runFollowupRound();
  };

  const interrupt = () => {
    const from = state;
    state = "Cancelled";
    push("runtime.interrupt_requested", { session_id: sessionId });
    push("session.state_changed", { from_state: from, to_state: "Cancelled" });
    state = "Completed";
    push("session.state_changed", { from_state: "Cancelled", to_state: "Completed" });
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
        return jsonResponse(res, 200, { commands: ["/plan", "/review", "/status", "/run"] });
      }
      if (url.pathname === "/runtime/start-session" && req.method === "POST") {
        const body = await readBody(req);
        return jsonResponse(res, 200, {
          ok: true,
          session: {
            session_id: sessionId,
            task_id: "full_stack_gui_regression",
            workspace_root: body.workspace || ".",
            model_mode: body.model_mode || provider,
            autonomy_mode: body.autonomy_mode || "conservative",
            state,
          },
        });
      }
      if (url.pathname === "/runtime/submit-user-message" && req.method === "POST") {
        const body = await readBody(req);
        if (state !== "Idle" && state !== "Completed" && state !== "Cancelled") {
          return jsonResponse(res, 200, {
            ok: false,
            session_id: sessionId,
            error_code: "runtime_turn_in_progress",
          });
        }
        startNextTurn(body);
        return jsonResponse(res, 200, { ok: true, session_id: sessionId });
      }
      if (url.pathname === "/runtime/submit-plan-decision" && req.method === "POST") {
        const body = await readBody(req);
        if (body.plan_approval_id !== planApprovalId) {
          return jsonResponse(res, 409, { ok: false, session_id: sessionId, error_code: "plan_id_mismatch" });
        }
        continuePlan(body);
        return jsonResponse(res, 200, { ok: true, session_id: sessionId, plan_approval_id: planApprovalId });
      }
      if (url.pathname === "/runtime/submit-permission-decision" && req.method === "POST") {
        const body = await readBody(req);
        if (body.permission_id !== permissionId) {
          return jsonResponse(res, 409, { ok: false, session_id: sessionId, error_code: "permission_id_mismatch" });
        }
        continuePermission(body);
        return jsonResponse(res, 200, {
          ok: true,
          session_id: sessionId,
          permission_id: permissionId,
          tool_call_id: "approved_shell_001",
          tool_id: "shell.command",
          resume_strategy: "execute_pending_tool",
          tool_executed: true,
          model_continuation_required: true,
        });
      }
      if (url.pathname === "/runtime/interrupt-session" && req.method === "POST") {
        await readBody(req);
        interrupt();
        return jsonResponse(res, 200, { ok: true, session_id: sessionId, state });
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
            model_mode: provider,
            autonomy_mode: "conservative",
            workspace_root: ".",
            pending_permission_count: awaitingPermission ? 1 : 0,
            pending_plan_approval_count: awaitingPlan ? 1 : 0,
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
    sessionId,
    planApprovalId,
    permissionId,
    events,
    userMessages,
    planDecisions,
    permissionDecisions,
    completedTurns: () => completedTurns,
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

async function sendPrompt(page, text) {
  const textarea = page.locator("textarea").first();
  await textarea.waitFor({ state: "visible", timeout: 15_000 });
  await textarea.fill(text);
  await page.keyboard.press("Enter");
}

async function waitForBodyText(page, text, timeoutMs = 15_000) {
  await page.waitForFunction(
    (expected) => document.body?.innerText.includes(expected),
    text,
    { timeout: timeoutMs },
  );
}

async function savePageArtifact(page, name) {
  await page.screenshot({ path: path.join(artifactDir, `${name}.png`), fullPage: true }).catch(() => {});
  const bodyText = await page.locator("body").innerText().catch(() => "");
  await fs.writeFile(path.join(artifactDir, `${name}.txt`), bodyText, "utf8").catch(() => {});
  return bodyText;
}

let browser;
let mockRuntime;
const finalReport = {
  ok: false,
  artifact_dir: artifactDir,
  headed,
  rounds: requestedRounds,
  tool_calls: toolCalls,
  provider,
  console_errors: consoleMessages,
  request_failures: requestFailures,
};

try {
  const runtimePort = await findFreePort();
  const webPort = await findFreePort();
  const runtimeBaseUrl = `http://127.0.0.1:${runtimePort}`;
  const webBaseUrl = `http://127.0.0.1:${webPort}`;
  log(`artifacts: ${artifactDir}`);

  mockRuntime = createMockRuntimeServer({ rounds: requestedRounds, toolCallCount: toolCalls });
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
    args: ["--window-size=1512,982"],
  });
  const context = await browser.newContext({
    viewport: { width: 1512, height: 982 },
    deviceScaleFactor: 1,
  });
  await context.addInitScript(({ runtimeBaseUrl, runtimePort, provider }) => {
    window.localStorage.setItem(
      "deepcode_config",
      JSON.stringify({
        provider,
        baseUrl: provider === "qwen" ? "http://127.0.0.1:11434/v1" : "https://api.deepseek.com/anthropic/v1/messages",
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
  }, { runtimeBaseUrl, runtimePort, provider });

  const page = await context.newPage();
  page.on("console", (message) => {
    const entry = { type: message.type(), text: message.text() };
    if (["error", "warning"].includes(message.type())) consoleMessages.push(entry);
  });
  page.on("requestfailed", (request) => {
    requestFailures.push({ url: request.url(), failure: request.failure()?.errorText || "unknown" });
  });

  await page.goto(webBaseUrl, { waitUntil: "domcontentloaded" });
  await savePageArtifact(page, "initial");

  await sendPrompt(page, "你好啊，先确认普通对话能不能完整收尾。");
  await waitForBodyText(page, "普通对话收尾正常");
  await savePageArtifact(page, "round-01-greeting");

  await sendPrompt(page, "进入 plan，规划真实 GUI 回归，然后等我批准。");
  const planDialog = page.getByRole("dialog", { name: "计划审批" });
  await planDialog.waitFor({ state: "visible", timeout: 12_000 });
  const planText = await planDialog.innerText();
  if (!planText.includes(mockRuntime.planApprovalId) || !planText.includes("完整 GUI 回归")) {
    throw new Error(`plan approval dialog missing expected text: ${planText}`);
  }
  await savePageArtifact(page, "round-02-plan-dialog");
  if (pauseAtApprovals) {
    await planDialog.waitFor({ state: "hidden", timeout: 120_000 });
  } else {
    await page.getByRole("button", { name: "批准计划并继续执行" }).click();
  }
  await planDialog.waitFor({ state: "hidden", timeout: 8_000 });
  await waitForBodyText(page, "计划审批已通过并执行");
  await savePageArtifact(page, "round-02-plan-complete");

  await sendPrompt(page, "现在申请 shell 权限并在批准后继续执行。");
  const permissionDialog = page.getByRole("dialog", { name: "权限审批" });
  await permissionDialog.waitFor({ state: "visible", timeout: 12_000 });
  const permissionText = await permissionDialog.innerText();
  if (!permissionText.includes(mockRuntime.permissionId) || !permissionText.includes("swift test")) {
    throw new Error(`permission dialog missing expected text: ${permissionText}`);
  }
  await savePageArtifact(page, "round-03-permission-dialog");
  if (pauseAtApprovals) {
    await permissionDialog.waitFor({ state: "hidden", timeout: 120_000 });
  } else {
    await page.getByRole("button", { name: "允许本次权限请求" }).click();
  }
  await permissionDialog.waitFor({ state: "hidden", timeout: 8_000 });
  await waitForBodyText(page, "shell 权限恢复完成");
  await savePageArtifact(page, "round-03-permission-complete");

  if (requestedRounds >= 4) {
    await sendPrompt(page, "派发 subagent 做一个只读审查，然后把结果折叠回来。");
    await waitForBodyText(page, "子 Agent 回合完成");
    await savePageArtifact(page, "round-04-subagent-complete");
  }

  if (requestedRounds >= 5) {
    await sendPrompt(page, "跑一轮大量工具调用，并且边输出 Markdown 表格边执行。");
    await waitForBodyText(page, `工具风暴完成：${toolCalls} 次工具调用`, 25_000);
    await savePageArtifact(page, "round-05-toolstorm-complete");
  }

  if (requestedRounds >= 6) {
    await sendPrompt(page, "你完事了吗？确认上一轮结束后还能继续回答。");
    await waitForBodyText(page, "后续追问正常响应");
    await savePageArtifact(page, "round-06-followup-complete");
  }

  await page.getByRole("button", { name: "诊断" }).click();
  await waitForBodyText(page, "诊断时间线", 5_000);
  const finalText = await savePageArtifact(page, "diagnostics");

  const eventTypes = new Set(mockRuntime.events.map((event) => event.event_type));
  const toolRequested = mockRuntime.events.filter((event) => event.event_type === "tool.call_requested").length;
  const toolCompleted = mockRuntime.events.filter((event) => event.event_type === "tool.call_completed").length;
  const duplicateEventIds = mockRuntime.events.length - new Set(mockRuntime.events.map((event) => event.event_id)).size;
  const blockingRequestFailures = requestFailures.filter((entry) => !String(entry.failure || "").includes("ERR_ABORTED"));
  const blockingConsole = consoleMessages.filter((entry) => entry.type === "error");
  const requiredEvents = [
    "assistant.message",
    "model.stream_delta",
    "tool.call_requested",
    "tool.call_completed",
    "plan.approval_requested",
    "runtime.plan_approval.model_continued",
    "permission.requested",
    "runtime.permission_resume.completed",
    "subagent.spawned",
    "subagent.completed",
    "context.compaction.skipped",
  ];
  const missingEvents = requiredEvents.filter((eventType) => !eventTypes.has(eventType));
  const subagentCompleted = mockRuntime.events.find((event) => event.event_type === "subagent.completed");
  const subagentArtifactRefs = Array.isArray(subagentCompleted?.payload?.artifact_refs)
    ? subagentCompleted.payload.artifact_refs
    : [];
  const forbiddenText = [
    "agent.visible_finalizer",
    "visible_finalizer",
    "final_answer",
    "agent.final_answer",
    "运行中",
    "permission_resume_tool_completed",
    "等待当前审批处理完成",
    "正在提交并恢复工具执行",
    "上一轮仍在收尾",
    "Duplicate observation plateau detected",
    "same_tool_error_plateau",
    "Agent 已达到本轮迭代上限",
  ].filter((needle) => finalText.includes(needle));

  finalReport.ok =
    mockRuntime.userMessages.length === requestedRounds &&
    mockRuntime.planDecisions.length === 1 &&
    mockRuntime.permissionDecisions.length === 1 &&
    mockRuntime.completedTurns() === requestedRounds &&
    missingEvents.length === 0 &&
    subagentArtifactRefs.length >= 2 &&
    duplicateEventIds === 0 &&
    toolRequested === toolCompleted &&
    toolRequested >= toolCalls + 10 &&
    finalText.includes("诊断时间线") &&
    finalText.includes("稳定") &&
    finalText.includes("上下文压力") &&
    finalText.includes("子 Agent") &&
    finalText.includes("后续追问正常响应") &&
    forbiddenText.length === 0 &&
    blockingRequestFailures.length === 0 &&
    blockingConsole.length === 0;
  finalReport.duration_ms = Date.now() - started;
  finalReport.runtime_base_url = runtimeBaseUrl;
  finalReport.web_base_url = webBaseUrl;
  finalReport.event_count = mockRuntime.events.length;
  finalReport.user_messages = mockRuntime.userMessages.length;
  finalReport.completed_turns = mockRuntime.completedTurns();
  finalReport.plan_decisions = mockRuntime.planDecisions;
  finalReport.permission_decisions = mockRuntime.permissionDecisions;
  finalReport.tool_requested = toolRequested;
  finalReport.tool_completed = toolCompleted;
  finalReport.duplicate_event_ids = duplicateEventIds;
  finalReport.missing_events = missingEvents;
  finalReport.subagent_artifact_refs = subagentArtifactRefs;
  finalReport.forbidden_text = forbiddenText;

  await fs.writeFile(
    path.join(artifactDir, "mock_runtime_events.jsonl"),
    mockRuntime.events.map((event) => JSON.stringify(event)).join("\n") + "\n",
  );
  if (!finalReport.ok) {
    throw new Error(`full GUI regression checks failed: ${JSON.stringify({
      userMessages: mockRuntime.userMessages.length,
      completedTurns: mockRuntime.completedTurns(),
      planDecisions: mockRuntime.planDecisions.length,
      permissionDecisions: mockRuntime.permissionDecisions.length,
      toolRequested,
      toolCompleted,
      missingEvents,
      forbiddenText,
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
  if (!finalReport.ok) process.exitCode = 1;
}
