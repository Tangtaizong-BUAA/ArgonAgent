#!/usr/bin/env node

import process from "node:process";
import readline from "node:readline";
import fs from "node:fs";
import path from "node:path";

function parseArgs(argv) {
  const out = {
    server: "http://127.0.0.1:8765",
    token: "",
    workspace: process.cwd(),
    model: "deepseek",
    autonomy: "fast_auto",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    const next = argv[i + 1];
    if (arg === "--server" && next) {
      out.server = next;
      i += 1;
    } else if (arg === "--token" && next) {
      out.token = next;
      i += 1;
    } else if (arg === "--workspace" && next) {
      out.workspace = next;
      i += 1;
    } else if (arg === "--model" && next) {
      out.model = next;
      i += 1;
    } else if (arg === "--autonomy" && next) {
      out.autonomy = next;
      i += 1;
    }
  }
  return out;
}

function authHeaders(token) {
  if (!token) {
    return { "Content-Type": "application/json" };
  }
  return {
    "Content-Type": "application/json",
    Authorization: `Bearer ${token}`,
  };
}

async function apiJson(config, method, route, payload = null) {
  const response = await fetch(`${config.server}${route}`, {
    method,
    headers: authHeaders(config.token),
    body: payload == null ? undefined : JSON.stringify(payload),
  });
  const text = await response.text();
  let data = {};
  if (text.trim()) {
    try {
      data = JSON.parse(text);
    } catch (error) {
      throw new Error(`invalid JSON response for ${route}: ${text.slice(0, 200)}`);
    }
  }
  if (!response.ok) {
    throw new Error(data.error || `HTTP ${response.status}`);
  }
  return data;
}

function renderCard(title, lines) {
  process.stdout.write(`\n╭─ ${title}\n`);
  for (const line of lines) {
    process.stdout.write(`│ ${line}\n`);
  }
  process.stdout.write("╰\n");
}

function welcome(state) {
  process.stdout.write(
    "╭─── ResearchCode Open-ClaudeCode Adapter v0.1.0 ────────────────────────────╮\n",
  );
  process.stdout.write("│                                                                             │\n");
  process.stdout.write("│                        Welcome back 唐太宗!                                 │\n");
  process.stdout.write("│                                                                             │\n");
  process.stdout.write("│                                <·>                                          │\n");
  process.stdout.write("│                                                                             │\n");
  process.stdout.write(
    `│  Native model: ${state.model.padEnd(10)} Provider: runtime facade                        │\n`,
  );
  process.stdout.write(`│  Workspace: ${state.workspace.slice(0, 56).padEnd(56)} │\n`);
  process.stdout.write("│                                                                             │\n");
  process.stdout.write("│  Tips: bare input runs /agent. Type / to list commands.                    │\n");
  process.stdout.write("╰─────────────────────────────────────────────────────────────────────────────╯\n");
}

function eventPayload(event) {
  return event && typeof event.payload === "object" ? event.payload : {};
}

async function renderEvent(state, rl, event) {
  const type = event.event_type || "";
  const payload = eventPayload(event);
  if (type === "model.stream_delta") {
    const text = String(payload.preview || "");
    if (text) {
      process.stdout.write(text);
    }
    return;
  }
  if (type === "assistant.message") {
    const text = String(payload.content || "");
    if (text) {
      process.stdout.write(`\n⏺ ${text}\n`);
    }
    return;
  }
  if (type === "tool.call_requested") {
    renderCard("ToolCallCard", [`requested: ${payload.tool_id || "unknown"}`]);
    return;
  }
  if (type === "tool.call_completed") {
    renderCard("CommandResultCard", [
      `tool: ${payload.tool_id || "unknown"}`,
      `ok=${payload.ok === false ? "false" : "true"}`,
      payload.result_preview ? String(payload.result_preview).slice(0, 180) : "",
    ]);
    return;
  }
  if (type === "permission.requested") {
    renderCard("PermissionCard", [
      `permission_id=${payload.permission_id || "unknown"}`,
      `request_type=${payload.request_type || "unknown"}`,
      `tool=${payload.tool_id || "unknown"}`,
    ]);
    const answer = await askLine(rl, "approve? [y/N] ");
    const decision = answer.trim().toLowerCase() === "y" ? "allow_once" : "deny";
    await apiJson(state.config, "POST", "/runtime/submit-permission-decision", {
      session_id: state.sessionId,
      permission_id: String(payload.permission_id || ""),
      decision,
    });
    return;
  }
  if (type === "plan.approval_requested") {
    renderCard("PlanModeCard", [
      `plan_approval_id=${payload.plan_approval_id || "unknown"}`,
      `goal=${String(payload.goal || "").slice(0, 180)}`,
    ]);
    const answer = await askLine(rl, "plan approve? [y/N] ");
    const decision = answer.trim().toLowerCase() === "y" ? "approve" : "request_revision";
    await apiJson(state.config, "POST", "/runtime/submit-plan-decision", {
      session_id: state.sessionId,
      plan_approval_id: String(payload.plan_approval_id || ""),
      decision,
      feedback: decision === "approve" ? "" : "manual revision requested",
    });
    return;
  }
  if (
    type === "tool.error.model_readable" ||
    type === "runtime.error" ||
    type === "model.call_blocked"
  ) {
    renderCard("RuntimeError", [JSON.stringify(payload).slice(0, 220)]);
  }
}

async function askLine(rl, prompt) {
  return await new Promise(resolve => {
    rl.question(prompt, value => resolve(value));
  });
}

async function streamEvents(state, rl) {
  let keep = true;
  let guard = 0;
  while (keep && guard < 12) {
    guard += 1;
    const data = await apiJson(
      state.config,
      "GET",
      `/runtime/stream-events?session_id=${encodeURIComponent(state.sessionId)}&cursor=${state.cursor}`,
    );
    const events = Array.isArray(data.events) ? data.events : [];
    state.cursor = Number(data.next_cursor || state.cursor);
    for (const event of events) {
      await renderEvent(state, rl, event);
    }
    keep = Boolean(events.length);
  }
}

async function startSession(state) {
  const response = await apiJson(state.config, "POST", "/runtime/start-session", {
    workspace: state.workspace,
    model_mode: state.model,
    autonomy_mode: state.autonomy,
  });
  state.sessionId = response?.session?.session_id || response?.session_id || "";
  if (!state.sessionId) {
    throw new Error(
      `runtime start-session response missing session id: ${JSON.stringify(response).slice(0, 240)}`,
    );
  }
  state.cursor = 0;
}

async function showSlashPalette(state) {
  const data = await apiJson(state.config, "GET", "/runtime/list-commands");
  const commands = Array.isArray(data.commands) ? data.commands : [];
  process.stdout.write(
    "────────────────────────────────────────────────────────────────────────────────\n",
  );
  for (const command of commands) {
    process.stdout.write(`${command}\n`);
  }
  process.stdout.write(
    "────────────────────────────────────────────────────────────────────────────────\n",
  );
}

async function submitText(state, text) {
  await apiJson(state.config, "POST", "/runtime/submit-user-message", {
    session_id: state.sessionId,
    text,
  });
}

async function main() {
  const config = parseArgs(process.argv.slice(2));
  const state = {
    config,
    workspace: path.resolve(config.workspace),
    model: config.model,
    autonomy: config.autonomy,
    sessionId: "",
    cursor: 0,
  };

  await apiJson(config, "GET", "/health");
  await startSession(state);
  welcome(state);
  const rl = readline.createInterface({
    input: process.stdin,
    output: process.stdout,
    historySize: 200,
  });

  const prompt = () => {
    rl.setPrompt("❯ ");
    rl.prompt();
  };

  rl.on("line", async line => {
    const text = String(line || "").trim();
    try {
      if (!text) {
        prompt();
        return;
      }
      if (text === "/exit") {
        rl.close();
        return;
      }
      if (text === "/") {
        await showSlashPalette(state);
        prompt();
        return;
      }
      if (text.startsWith("/model ")) {
        const model = text.split(" ", 2)[1]?.trim();
        if (model && (model === "deepseek" || model === "qwen")) {
          state.model = model;
          await startSession(state);
          renderCard("SessionSummary", [
            `new session: ${state.sessionId}`,
            `model=${state.model}`,
          ]);
        } else {
          renderCard("RuntimeError", ["model must be deepseek or qwen"]);
        }
        prompt();
        return;
      }
      if (text.startsWith("/events")) {
        const output = text.split(" ", 2)[1]?.trim() || ".researchcode/open_tui_events.jsonl";
        const abs = path.resolve(state.workspace, output);
        const exported = await apiJson(state.config, "POST", "/runtime/export-events", {
          session_id: state.sessionId,
          path: abs,
        });
        renderCard("SessionSummary", [
          `exported: ${exported.path}`,
          `events=${exported.event_count}`,
        ]);
        prompt();
        return;
      }
      await submitText(state, text);
      await streamEvents(state, rl);
      process.stdout.write("\n");
    } catch (error) {
      renderCard("RuntimeError", [String(error?.message || error)]);
    }
    prompt();
  });

  rl.on("close", () => {
    process.stdout.write("bye\n");
    process.exit(0);
  });

  prompt();
}

main().catch(error => {
  process.stderr.write(`fatal: ${String(error?.message || error)}\n`);
  process.exit(1);
});
