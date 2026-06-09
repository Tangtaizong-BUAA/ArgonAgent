import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const tempDir = mkdtempSync(join(tmpdir(), "argon-progress-ledger-"));
const desktopDir = dirname(fileURLToPath(import.meta.url));

try {
  const compile = spawnSync(
    "node",
    [
      "node_modules/typescript/bin/tsc",
      "src/runtime/progressLedger.ts",
      "--outDir",
      tempDir,
      "--module",
      "commonjs",
      "--target",
      "es2020",
      "--moduleResolution",
      "node",
      "--skipLibCheck",
    ],
    { cwd: desktopDir, encoding: "utf8" },
  );
  assert.equal(compile.status, 0, compile.stderr || compile.stdout);

  const require = createRequire(import.meta.url);
  const ledger = require(join(tempDir, "runtime", "progressLedger.js"));

  let items = [];
  items = ledger.upsertToolProgress(items, {
    id: "progress_1",
    label: "运行 shell.command",
    status: "running",
    toolCallId: "tool_1",
    toolId: "shell.command",
  });
  items = ledger.upsertToolProgress(items, {
    id: "progress_2",
    label: "已分派 shell.command",
    status: "running",
    toolCallId: "tool_1",
    toolId: "shell.command",
  });
  assert.equal(items.length, 1);
  assert.equal(items[0].label, "已分派 shell.command");

  items = [
    ...items,
    {
      id: "legacy_running",
      label: "运行 shell.command",
      status: "running",
      toolCallId: "tool_1",
    },
  ];
  items = ledger.completeToolProgress(items, {
    toolCallId: "tool_1",
    toolId: "shell.command",
    status: "done",
  });
  assert.deepEqual(items.map((item) => item.status), ["done", "done"]);

  items = ledger.upsertPermissionProgress(items, {
    id: "permission_1",
    label: "权限审批已排队",
    status: "running",
    permissionId: "perm_1",
  });
  items = ledger.upsertPermissionProgress(items, {
    id: "permission_2",
    label: "等待 runtime 接收审批",
    status: "running",
    permissionId: "perm_1",
  });
  assert.equal(items.filter((item) => item.kind === "permission").length, 1);
  items = ledger.completePermissionProgress(items, "perm_1", "done");
  assert.equal(items.find((item) => item.permissionId === "perm_1").status, "done");

  items = [{
    id: "legacy_no_id",
    label: "正在执行 shell.command",
    status: "running",
  }];
  items = ledger.completeToolProgress(items, {
    toolId: "shell.command",
    status: "done",
  });
  assert.equal(items[0].status, "done");

  console.log("desktop progress ledger tests passed");
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}
