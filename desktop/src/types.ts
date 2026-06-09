export type RunStatus =
  | "idle"
  | "planning"
  | "running"
  | "waiting_approval"
  | "failed"
  | "completed"
  | "stopped";

export type AgentMode = "ask" | "plan" | "execute" | "review" | "auto";

export interface AgentStep {
  id: string;
  title: string;
  status: "pending" | "running" | "done" | "failed" | "skipped";
  type: "message" | "tool_call" | "file_edit" | "test" | "approval" | "summary";
  content?: string;
  startedAt?: string;
  endedAt?: string;
}

export interface FileChange {
  path: string;
  additions: number;
  deletions: number;
  status: "modified" | "created" | "deleted";
}

export interface Artifact {
  id: string;
  name: string;
  type: "markdown" | "log" | "patch" | "report" | "image" | "other";
  path?: string;
  url?: string;
}

export interface AgentRun {
  id: string;
  projectId: string;
  title: string;
  status: RunStatus;
  mode: AgentMode;
  model: string;
  branch?: string;
  startedAt: string;
  updatedAt: string;
  steps: AgentStep[];
  artifacts: Artifact[];
  changes: FileChange[];
}

export interface Project {
  id: string;
  name: string;
  path: string;
  branch?: string;
  runs: AgentRun[];
}

export interface NavItem {
  id: string;
  label: string;
  icon?: string;
}

export interface TranscriptMessage {
  id: string;
  role: "user" | "agent" | "system";
  type: "text" | "tool_call" | "file_edit" | "test_result" | "approval" | "thinking";
  content: string;
  metadata?: Record<string, unknown>;
  timestamp: string;
}

export interface ProgressItem {
  id: string;
  label: string;
  status: "pending" | "running" | "done" | "failed";
  toolCallId?: string;
  toolId?: string;
  permissionId?: string;
  kind?: "tool" | "permission" | "observability";
  category?: "recovery" | "context" | "repair" | "cache" | "permission" | "tool" | "model" | "subagent" | "other";
  detail?: string;
  eventType?: string;
}

export interface BranchDetail {
  name: string;
  checkpoint?: string;
  hasChanges: boolean;
}

export interface TokenUsageStats {
  uploaded: number;
  downloaded: number;
  reasoning: number;
  cacheHit: number;
  cacheMiss: number;
}
