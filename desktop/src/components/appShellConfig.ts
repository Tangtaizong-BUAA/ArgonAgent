import type { DeepSeekModelId } from "./BottomComposer";
import type { ContextPressureState } from "./RuntimeStatus";
import type { TokenUsageStats } from "@/types";

export const PROJECT_PICK_STORAGE_KEY = "argon_agent_selected_project_path_v1";
export const FRONTEND_BUILD_MARK = "argon-agent-ui:runtime-v1";

export const FALLBACK_SLASH_COMMANDS: string[] = [
  "/repo [path]",
  "/read <path> [offset] [limit]",
  "/search <pattern> [path]",
  "/git status|diff|log [args]",
  "/run <command>",
  "/plan [goal]",
  "/plan approve",
  "/plan reject <feedback>",
  "/permissions",
  "/snapshot",
  "/export [path]",
];

export const EMPTY_TOKEN_USAGE: TokenUsageStats = {
  uploaded: 0,
  downloaded: 0,
  reasoning: 0,
  cacheHit: 0,
  cacheMiss: 0,
};

export const EMPTY_CONTEXT_PRESSURE: ContextPressureState = {
  promptTokens: null,
  maxTokens: null,
  remainingTokens: null,
  status: "normal",
  label: "等待上下文预算",
};

export function normalizeDeepSeekModelId(value?: string): DeepSeekModelId {
  return value === "deepseek-v4-pro" ? "deepseek-v4-pro" : "deepseek-v4-flash";
}

export function makeId(prefix: string): string {
  return `${prefix}_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
}
