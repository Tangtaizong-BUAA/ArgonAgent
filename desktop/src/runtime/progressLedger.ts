import type { ProgressItem } from "../types";

type ProgressStatus = ProgressItem["status"];

function sameTool(item: ProgressItem, toolCallId?: string, toolId?: string): boolean {
  if (toolCallId) {
    if (item.toolCallId) {
      return item.toolCallId === toolCallId;
    }
    if (!toolId) {
      return false;
    }
    return item.toolId === toolId || item.label.includes(toolId);
  }
  if (!toolId) {
    return false;
  }
  if (item.toolId === toolId) {
    return true;
  }
  return item.label.includes(toolId);
}

function samePermission(item: ProgressItem, permissionId?: string): boolean {
  return Boolean(permissionId && item.permissionId === permissionId);
}

function terminal(status: ProgressStatus): status is "done" | "failed" {
  return status === "done" || status === "failed";
}

export function upsertToolProgress(
  items: ProgressItem[],
  nextItem: Omit<ProgressItem, "id" | "kind"> & { id?: string },
): ProgressItem[] {
  const next = items.slice();
  const index = next.findIndex((item) =>
    item.kind === "tool" && sameTool(item, nextItem.toolCallId, nextItem.toolId),
  );
  const item: ProgressItem = {
    ...nextItem,
    id: nextItem.id ?? `tool-progress-${nextItem.toolCallId ?? nextItem.toolId ?? next.length}`,
    kind: "tool",
  };
  if (index >= 0) {
    next[index] = { ...next[index], ...item };
    return next;
  }
  return [...next, item];
}

export function completeToolProgress(
  items: ProgressItem[],
  identity: {
    toolCallId?: string;
    toolId?: string;
    status: "done" | "failed";
    allowTerminalUpdate?: boolean;
    category?: ProgressItem["category"];
    detail?: string;
    eventType?: string;
  },
): ProgressItem[] {
  let matched = false;
  const next = items.map((item) => {
    if (
      (!terminal(item.status) || identity.allowTerminalUpdate === true) &&
      sameTool(item, identity.toolCallId, identity.toolId)
    ) {
      matched = true;
      return {
        ...item,
        status: identity.status,
        toolCallId: item.toolCallId ?? identity.toolCallId,
        toolId: item.toolId ?? identity.toolId,
        category: identity.category ?? item.category,
        detail: identity.detail ?? item.detail,
        eventType: identity.eventType ?? item.eventType,
      };
    }
    return item;
  });
  if (matched || !identity.toolId) {
    return next;
  }
  for (let index = next.length - 1; index >= 0; index -= 1) {
    const item = next[index];
    if (!terminal(item.status) && item.label.includes(identity.toolId)) {
      const patched = next.slice();
      patched[index] = {
        ...item,
        status: identity.status,
        toolId: item.toolId ?? identity.toolId,
        category: identity.category ?? item.category,
        detail: identity.detail ?? item.detail,
        eventType: identity.eventType ?? item.eventType,
      };
      return patched;
    }
  }
  return next;
}

export function upsertPermissionProgress(
  items: ProgressItem[],
  nextItem: Omit<ProgressItem, "id" | "kind"> & { id?: string },
): ProgressItem[] {
  const next = items.slice();
  const index = next.findIndex((item) =>
    item.kind === "permission" && samePermission(item, nextItem.permissionId),
  );
  const item: ProgressItem = {
    ...nextItem,
    id: nextItem.id ?? `permission-progress-${nextItem.permissionId ?? next.length}`,
    kind: "permission",
  };
  if (index >= 0) {
    next[index] = { ...next[index], ...item };
    return next;
  }
  return [...next, item];
}

export function completePermissionProgress(
  items: ProgressItem[],
  permissionId: string | undefined,
  status: "done" | "failed" = "done",
  patch: Pick<ProgressItem, "category" | "detail" | "eventType"> = {},
): ProgressItem[] {
  if (!permissionId) {
    return items;
  }
  return items.map((item) =>
    samePermission(item, permissionId) && !terminal(item.status)
      ? {
          ...item,
          status,
          category: patch.category ?? item.category,
          detail: patch.detail ?? item.detail,
          eventType: patch.eventType ?? item.eventType,
        }
      : item,
  );
}

export function normalizeStoredProgressItems(items: ProgressItem[]): ProgressItem[] {
  let normalized = items;
  for (const item of items) {
    if (item.toolCallId && terminal(item.status)) {
      normalized = completeToolProgress(normalized, {
        toolCallId: item.toolCallId,
        toolId: item.toolId,
        status: item.status,
      });
    }
    if (item.permissionId && terminal(item.status)) {
      normalized = completePermissionProgress(normalized, item.permissionId, item.status);
    }
  }
  return normalized;
}
