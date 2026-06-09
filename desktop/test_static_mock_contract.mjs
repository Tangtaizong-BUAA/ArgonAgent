import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

const html = await readFile(new URL("./static_mock.html", import.meta.url), "utf8");

assert.match(html, /id="modelTimeline"/);
assert.match(html, /id="workflowPanels"/);
assert.match(html, /id="repoMap"/);
assert.match(html, /id="toolCatalog"/);
assert.match(html, /id="permissionDrawer"/);
assert.match(html, /function renderModelTimeline/);
assert.match(html, /function renderWorkflowPanels/);
assert.match(html, /function renderRepoMap/);
assert.match(html, /function renderToolCatalog/);
assert.match(html, /function renderApprovalQueue/);
assert.match(html, /blocked_permission_events/);
assert.match(html, /getLatestRun/);
assert.match(html, /getApprovalQueue/);
assert.match(html, /getRepoMap/);
assert.match(html, /getToolCatalog/);
assert.match(html, /reasoning=/);
assert.match(html, /cache_hit=/);
assert.match(html, /Workflow Panels/);
assert.match(html, /Project Map/);
assert.match(html, /Tool Catalog/);
assert.match(html, /window\.location\.origin/);
assert.match(html, /mock_data\/model_events\.jsonl/);
assert.doesNotMatch(html, /sk-testsecret/);

console.log("desktop static mock contract tests passed");
