import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const tempDir = mkdtempSync(join(tmpdir(), "argon-transcript-dedupe-"));
const desktopDir = dirname(fileURLToPath(import.meta.url));

try {
  const compile = spawnSync(
    "node",
    [
      "node_modules/typescript/bin/tsc",
      "src/runtime/transcriptDedupe.ts",
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
  const dedupe = require(join(tempDir, "transcriptDedupe.js"));

  const streamed = String.raw`好的！我查看了当前的工具集，遗憾的是我没有 shell_command 工具。\n\n## 可选演示`;
  const final = "好的！我查看了当前的工具集，遗憾的是我没有 shell_command 工具。\n\n## 可选演示";
  assert.equal(dedupe.isDuplicateAgentText(streamed, final), true);

  const streamedWithToolMarkup = `好的！\n{"tool_calls":[{"name":"repo_map","arguments":{"root":"."}}]}\n\n## 可选演示`;
  assert.equal(dedupe.isDuplicateAgentText(streamedWithToolMarkup, "好的！\n\n## 可选演示"), true);

  assert.equal(dedupe.isDuplicateAgentText("好的！\n\n## 可选演示", "好的！\n\n## 另一段"), false);

  console.log("desktop transcript dedupe tests passed");
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}
