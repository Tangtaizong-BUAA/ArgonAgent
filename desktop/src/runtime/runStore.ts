import type { PendingPermissionState, PendingPlanApprovalState } from "@/hooks/usePermissionFlow";
import type { AgentRun, ProgressItem, Project, RunStatus, TranscriptMessage } from "@/types";

export interface StoredRunState {
  run: AgentRun;
  messages: TranscriptMessage[];
  progressItems: ProgressItem[];
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  sessionId: string | null;
  cursor: number;
  workspaceRoot: string;
  sourceWorkspaceRoot: string;
}

export const RUN_STORE_STORAGE_KEY = "argon_agent_run_store_v1";
export const MAX_STORED_RUNS = 20;
const MAX_PERSISTED_MESSAGES_PER_RUN = 180;
const MAX_PROGRESS_ITEMS = 120;

export function nowIso(): string {
  return new Date().toISOString();
}

export function normalizeProjectPath(pathValue: string): string {
  let value = pathValue.trim();
  if (!value) {
    return "";
  }
  value = value.replace(/\\/g, "/");
  value = value.replace(/\/{2,}/g, "/");
  value = value.replace(/\/\.\//g, "/");
  while (
    value.length > 1 &&
    value.endsWith("/") &&
    !/^[A-Za-z]:\/$/.test(value)
  ) {
    value = value.slice(0, -1);
  }
  return value;
}

export function stablePathProjectId(pathValue: string): string {
  const input = normalizeProjectPath(pathValue).toLowerCase();
  if (!input) {
    return "project_default";
  }
  let hash = 2166136261;
  for (let index = 0; index < input.length; index += 1) {
    hash ^= input.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return `project_${(hash >>> 0).toString(16)}`;
}

export function isIsolatedWorkspacePath(pathValue: string): boolean {
  const value = normalizeProjectPath(pathValue).toLowerCase();
  return value.includes("/.researchcode/argon_agent/workspaces/projects/");
}

export function extractProjectName(pathValue: string): string {
  const compact = normalizeProjectPath(pathValue);
  if (!compact) {
    return "workspace";
  }
  const segments = compact.split("/");
  return segments[segments.length - 1] || "workspace";
}

export function compactMessagesForStorage(messages: TranscriptMessage[]): TranscriptMessage[] {
  if (messages.length <= MAX_PERSISTED_MESSAGES_PER_RUN) {
    return messages;
  }
  const keepHead = messages.slice(0, 6);
  const keepTail = messages.slice(-(MAX_PERSISTED_MESSAGES_PER_RUN - keepHead.length - 1));
  const compactedCount = messages.length - keepHead.length - keepTail.length;
  return [
    ...keepHead,
    {
      id: `compacted_${messages.length}_${compactedCount}`,
      role: "system",
      type: "text",
      content: `已折叠 ${compactedCount} 条较早 UI 记录；完整事件仍可通过导出查看。`,
      timestamp: keepTail[0]?.timestamp ?? nowIso(),
    },
    ...keepTail,
  ];
}

export function trimProgressItems(items: ProgressItem[]): ProgressItem[] {
  if (items.length <= MAX_PROGRESS_ITEMS) {
    return items;
  }
  const running = items.filter((item) => item.status === "running");
  const recent = items.slice(-MAX_PROGRESS_ITEMS);
  const merged = [...running, ...recent];
  const seen = new Set<string>();
  return merged.filter((item) => {
    const key = item.toolCallId ?? item.permissionId ?? item.id;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  }).slice(-MAX_PROGRESS_ITEMS);
}

export function deriveArtifacts(progressItems: ProgressItem[]): AgentRun["artifacts"] {
  const artifacts: AgentRun["artifacts"] = [];
  const seen = new Set<string>();
  for (const item of progressItems) {
    if (item.status !== "done") {
      continue;
    }
    const toolId = item.toolId ?? "";
    let name = "";
    let type: AgentRun["artifacts"][number]["type"] = "other";
    if (toolId.includes("patch") || toolId.includes("file.write") || toolId.includes("file.edit")) {
      name = item.label.replace(/^.*?:?\s*/, "") || "代码变更";
      type = "patch";
    } else if (toolId.includes("search") || toolId.includes("repo.map") || toolId.includes("file.read")) {
      name = item.label || "上下文证据";
      type = "log";
    } else if (toolId.includes("shell") || toolId.startsWith("git.")) {
      name = item.label || "命令输出";
      type = "log";
    }
    if (!name) {
      continue;
    }
    const key = `${type}:${name}`;
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    artifacts.push({ id: `artifact_${artifacts.length}_${key}`, name, type });
  }
  return artifacts.slice(-8);
}

export function deriveChanges(progressItems: ProgressItem[]): AgentRun["changes"] {
  const edits = progressItems.filter((item) => {
    const toolId = item.toolId ?? "";
    return item.status === "done" && (
      toolId.includes("patch.apply") ||
      toolId.includes("file.write") ||
      toolId.includes("file.edit") ||
      toolId.includes("file.multi_edit")
    );
  });
  return edits.slice(-8).map((item, index) => ({
    path: item.label.replace(/^(运行|已分派|权限恢复完成|工具完成)\s*/, "").trim() || `change-${index + 1}`,
    additions: 0,
    deletions: 0,
    status: "modified",
  }));
}

export function buildAgentRun(args: {
  activeModelId: string;
  activeRunId: string;
  messages: TranscriptMessage[];
  progressItems: ProgressItem[];
  provider: "deepseek" | "qwen";
  runStatus: RunStatus;
  runTitle: string;
  selectedSourceWorkspaceRoot: string;
  sessionId: string | null;
}): AgentRun | null {
  if (!args.sessionId && args.messages.length === 0 && args.runStatus === "idle") {
    return null;
  }
  const startedAt = args.messages[0]?.timestamp ?? nowIso();
  const updatedAt = args.messages[args.messages.length - 1]?.timestamp ?? startedAt;
  return {
    id: args.sessionId ?? args.activeRunId,
    projectId: stablePathProjectId(normalizeProjectPath(args.selectedSourceWorkspaceRoot)),
    title: args.runTitle,
    status: args.runStatus,
    mode: "execute",
    model: args.provider === "deepseek" ? args.activeModelId : "qwen-3.6-27b",
    branch: args.sessionId ? `runtime/${args.sessionId.slice(-8)}` : "runtime/new",
    startedAt,
    updatedAt,
    steps: [],
    artifacts: deriveArtifacts(args.progressItems),
    changes: deriveChanges(args.progressItems),
  };
}

export function makeStoredRunSnapshot(args: {
  activeModelId: string;
  activeRunId: string;
  cursor: number;
  messages: TranscriptMessage[];
  pendingPermissions: PendingPermissionState[];
  pendingPlanApprovals: PendingPlanApprovalState[];
  progressItems: ProgressItem[];
  provider: "deepseek" | "qwen";
  runStatus: RunStatus;
  runTitle: string;
  selectedSourceWorkspaceRoot: string;
  sessionId: string | null;
  workspaceRoot: string;
}): StoredRunState | null {
  const run = buildAgentRun(args);
  if (!run) {
    return null;
  }
  return {
    run,
    messages: compactMessagesForStorage(args.messages),
    progressItems: trimProgressItems(args.progressItems),
    pendingPermissions: args.pendingPermissions,
    pendingPlanApprovals: args.pendingPlanApprovals,
    sessionId: args.sessionId,
    cursor: args.cursor,
    workspaceRoot: normalizeProjectPath(args.workspaceRoot),
    sourceWorkspaceRoot: normalizeProjectPath(args.selectedSourceWorkspaceRoot),
  };
}

export function mergeStoredRun(
  currentStore: Record<string, StoredRunState>,
  snapshot: StoredRunState,
): Record<string, StoredRunState> {
  const merged = {
    ...currentStore,
    [snapshot.run.id]: snapshot,
  };
  return Object.values(merged)
    .sort((a, b) => b.run.updatedAt.localeCompare(a.run.updatedAt))
    .slice(0, MAX_STORED_RUNS)
    .reduce<Record<string, StoredRunState>>((acc, item) => {
      acc[item.run.id] = item;
      return acc;
    }, {});
}

export function parseStoredRunStore(raw: string | null): {
  store: Record<string, StoredRunState>;
  latest: StoredRunState | null;
} {
  if (!raw) {
    return { store: {}, latest: null };
  }
  const parsed = JSON.parse(raw) as Record<string, StoredRunState>;
  const entries = Object.values(parsed)
    .filter((item) => item?.run?.id)
    .sort((a, b) => b.run.updatedAt.localeCompare(a.run.updatedAt))
    .slice(0, MAX_STORED_RUNS);
  const store = entries.reduce<Record<string, StoredRunState>>((acc, item) => {
    acc[item.run.id] = item;
    return acc;
  }, {});
  return { store, latest: entries[0] ?? null };
}

export function storedRunsWithCurrent(
  store: Record<string, StoredRunState>,
  currentRun: AgentRun | null,
): AgentRun[] {
  const stored = Object.values(store)
    .map((item) => item.run)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  const merged = currentRun ? [currentRun, ...stored.filter((run) => run.id !== currentRun.id)] : stored;
  return merged.slice(0, MAX_STORED_RUNS);
}

export function buildProjectsFromRuns(args: {
  runStore: Record<string, StoredRunState>;
  runs: AgentRun[];
  selectedSourceWorkspaceRoot: string;
  workspaceRoot: string;
}): Project[] {
  const grouped = new Map<
    string,
    {
      path: string;
      runs: AgentRun[];
    }
  >();
  const sourcePathByProjectName = new Map<string, string>();

  const pathForRun = (run: AgentRun) => normalizeProjectPath(
    args.runStore[run.id]?.sourceWorkspaceRoot ||
      args.runStore[run.id]?.workspaceRoot ||
      args.selectedSourceWorkspaceRoot ||
      args.workspaceRoot,
  );

  for (const run of args.runs) {
    const path = pathForRun(run);
    if (!path || isIsolatedWorkspacePath(path)) {
      continue;
    }
    const name = extractProjectName(path).toLowerCase();
    if (!sourcePathByProjectName.has(name)) {
      sourcePathByProjectName.set(name, path);
    }
  }

  for (const run of args.runs) {
    const path = pathForRun(run);
    const name = extractProjectName(path).toLowerCase();
    const effectivePath =
      isIsolatedWorkspacePath(path) && sourcePathByProjectName.has(name)
        ? sourcePathByProjectName.get(name)!
        : path;
    const id = stablePathProjectId(effectivePath);
    const bucket = grouped.get(id) ?? { path: effectivePath, runs: [] };
    bucket.runs.push(run);
    grouped.set(id, bucket);
  }

  if (grouped.size === 0) {
    const path = normalizeProjectPath(args.selectedSourceWorkspaceRoot || args.workspaceRoot);
    grouped.set(stablePathProjectId(path), {
      path,
      runs: [],
    });
  }

  return Array.from(grouped.entries())
    .map(([id, value]) => ({
      id,
      name: extractProjectName(value.path),
      path: value.path,
      branch: value.runs[0]?.branch ?? "runtime/new",
      runs: value.runs.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)),
    }))
    .sort((a, b) => {
      const left = a.runs[0]?.updatedAt ?? "";
      const right = b.runs[0]?.updatedAt ?? "";
      return right.localeCompare(left);
    });
}
