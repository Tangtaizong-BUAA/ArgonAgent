export type RuntimeTransport = "http" | "tauri";

export interface RuntimeBootstrap {
  transport?: RuntimeTransport;
  baseUrl: string;
  token: string;
  workspaceRoot: string;
  port: number;
  logPath: string;
}

declare global {
  interface Window {
    __ARGON_RUNTIME_BOOTSTRAP__?: RuntimeBootstrap;
    __ARGON_GUI_DEBUG__?: {
      cursor: number;
      progress_count: number;
      message_count: number;
      run_status: string;
      session_id: string | null;
    };
  }
}

export {};
