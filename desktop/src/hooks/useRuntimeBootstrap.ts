import { useEffect } from "react";
import type { Dispatch, SetStateAction } from "react";
import {
  listRuntimeCommands,
  resolveRuntimeBootstrap,
} from "@/runtime/localRuntimeClient";
import {
  normalizeProjectPath,
  stablePathProjectId,
} from "@/runtime/runStore";
import type { RuntimeBootstrap } from "@/types/runtime";

interface UseRuntimeBootstrapArgs {
  buildMark: string;
  fallbackCommands: string[];
  projectPickStorageKey: string;
  setBootstrap: Dispatch<SetStateAction<RuntimeBootstrap | null>>;
  setRuntimeError: Dispatch<SetStateAction<string | null>>;
  setSelectedProjectId: Dispatch<SetStateAction<string>>;
  setSelectedSourceWorkspaceRoot: Dispatch<SetStateAction<string>>;
  setSlashCommands: Dispatch<SetStateAction<string[]>>;
  setWorkspaceRoot: Dispatch<SetStateAction<string>>;
}

export function useRuntimeBootstrap({
  buildMark,
  fallbackCommands,
  projectPickStorageKey,
  setBootstrap,
  setRuntimeError,
  setSelectedProjectId,
  setSelectedSourceWorkspaceRoot,
  setSlashCommands,
  setWorkspaceRoot,
}: UseRuntimeBootstrapArgs) {
  useEffect(() => {
    let cancelled = false;
    resolveRuntimeBootstrap()
      .then((payload) => {
        if (cancelled) {
          return;
        }
        setBootstrap(payload);
        setWorkspaceRoot(payload.workspaceRoot);
        const storedProjectPath = normalizeProjectPath(localStorage.getItem(projectPickStorageKey)?.trim() || "");
        const preferredProjectPath = storedProjectPath || normalizeProjectPath(payload.workspaceRoot);
        setSelectedSourceWorkspaceRoot(preferredProjectPath);
        setSelectedProjectId(stablePathProjectId(preferredProjectPath));
        console.info("[ArgonAgent] bootstrap", {
          build: buildMark,
          transport: payload.transport,
          workspaceRoot: payload.workspaceRoot,
          preferredProjectPath,
          tauriBridgeReady: Boolean(window.__TAURI__?.core?.invoke && window.__TAURI__?.event?.listen),
        });
        if (payload.transport === "tauri" && !window.__TAURI__?.core?.invoke) {
          console.warn("[ArgonAgent] tauri transport selected but invoke bridge is unavailable.");
        }
        void listRuntimeCommands(payload)
          .then((commands) => {
            if (cancelled) {
              return;
            }
            if (commands.length > 0) {
              setSlashCommands(commands);
            }
          })
          .catch(() => {
            if (!cancelled) {
              setSlashCommands(fallbackCommands);
            }
          });
      })
      .catch((error) => {
        if (cancelled) {
          return;
        }
        setRuntimeError(String((error as Error).message ?? error));
        setSlashCommands(fallbackCommands);
      });
    return () => {
      cancelled = true;
    };
  }, [
    buildMark,
    fallbackCommands,
    projectPickStorageKey,
    setBootstrap,
    setRuntimeError,
    setSelectedProjectId,
    setSelectedSourceWorkspaceRoot,
    setSlashCommands,
    setWorkspaceRoot,
  ]);
}
