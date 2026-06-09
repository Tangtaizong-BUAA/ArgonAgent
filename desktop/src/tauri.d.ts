declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke?: <T = unknown>(
          command: string,
          args?: Record<string, unknown>,
        ) => Promise<T>;
      };
      event?: {
        listen?: <T = unknown>(
          event: string,
          callback: (event: { payload: T }) => void,
        ) => Promise<() => void>;
      };
    };
  }
}

export {};
