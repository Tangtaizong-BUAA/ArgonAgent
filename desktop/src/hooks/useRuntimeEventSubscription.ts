import { useCallback, useEffect, useRef, useState } from "react";
import type { MutableRefObject } from "react";
import {
  getRuntimeSnapshot,
  streamRuntimeEvents,
  subscribeRuntimeEvents,
  type RuntimeEvent,
} from "@/runtime/localRuntimeClient";
import { shouldApplyRuntimeSnapshotState } from "@/runtime/runtimeEventViewModel";
import type { RuntimeBootstrap } from "@/types/runtime";

interface UseRuntimeEventSubscriptionArgs {
  bootstrap: RuntimeBootstrap | null;
  sessionId: string | null;
  cursorRef: MutableRefObject<number>;
  onCursorChange: (cursor: number) => void;
  onEvents: (events: RuntimeEvent[]) => void;
  onSnapshotState: (state: string) => void;
  onError: (message: string) => void;
}

async function withRuntimeTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  label: string,
): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | null = null;
  const timeoutPromise = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      reject(new Error(`${label} timeout after ${timeoutMs}ms`));
    }, timeoutMs);
  });
  try {
    return await Promise.race([promise, timeoutPromise]);
  } finally {
    if (timer) {
      clearTimeout(timer);
    }
  }
}

export function useRuntimeEventSubscription({
  bootstrap,
  sessionId,
  cursorRef,
  onCursorChange,
  onEvents,
  onSnapshotState,
  onError,
}: UseRuntimeEventSubscriptionArgs) {
  const [tauriPushActive, setTauriPushActive] = useState(false);
  const pollingRef = useRef(false);
  const tauriSubscribedSessionRef = useRef<string | null>(null);
  const tauriUnlistenRef = useRef<(() => void) | null>(null);
  const pushedEventsHandlerRef = useRef<(events: RuntimeEvent[]) => void>(() => {});

  const applyPushedRuntimeEvents = useCallback((events: RuntimeEvent[]) => {
    if (events.length === 0) {
      return;
    }
    let nextCursor = cursorRef.current;
    for (const event of events) {
      if (typeof event.sequence === "number") {
        nextCursor = Math.max(nextCursor, event.sequence);
      }
    }
    if (nextCursor !== cursorRef.current) {
      cursorRef.current = nextCursor;
      onCursorChange(nextCursor);
    }
    onEvents(events);
  }, [cursorRef, onCursorChange, onEvents]);

  useEffect(() => {
    pushedEventsHandlerRef.current = applyPushedRuntimeEvents;
  }, [applyPushedRuntimeEvents]);

  const clearRuntimeSubscription = useCallback(() => {
    if (tauriUnlistenRef.current) {
      tauriUnlistenRef.current();
      tauriUnlistenRef.current = null;
    }
    tauriSubscribedSessionRef.current = null;
    setTauriPushActive(false);
  }, []);

  const ensureTauriSubscription = useCallback(async (runtime: RuntimeBootstrap, targetSessionId: string) => {
    if (runtime.transport !== "tauri") {
      return;
    }
    if (tauriSubscribedSessionRef.current === targetSessionId && tauriUnlistenRef.current) {
      return;
    }
    if (tauriUnlistenRef.current) {
      tauriUnlistenRef.current();
      tauriUnlistenRef.current = null;
    }
    tauriSubscribedSessionRef.current = targetSessionId;
    try {
      const unlisten = await subscribeRuntimeEvents(runtime, { session_id: targetSessionId }, (events) =>
        pushedEventsHandlerRef.current(events),
      );
      if (tauriSubscribedSessionRef.current !== targetSessionId) {
        unlisten();
        return;
      }
      tauriUnlistenRef.current = unlisten;
      setTauriPushActive(true);
    } catch (error) {
      setTauriPushActive(false);
      onError(String((error as Error).message ?? error));
    }
  }, [onError]);

  const pollRuntimeEvents = useCallback(async (sessionIdOverride?: string) => {
    const effectiveSessionId = sessionIdOverride ?? sessionId;
    if (!bootstrap || !effectiveSessionId || pollingRef.current) {
      return;
    }
    pollingRef.current = true;
    try {
      if (!(bootstrap.transport === "tauri" && tauriPushActive)) {
        const drainedEvents: RuntimeEvent[] = [];
        let nextCursor = cursorRef.current;
        for (let page = 0; page < 25; page += 1) {
          const streamed = await withRuntimeTimeout(
            streamRuntimeEvents(bootstrap, {
              session_id: effectiveSessionId,
              cursor: nextCursor,
            }),
            10_000,
            "runtime.stream_events",
          );
          if (streamed.events.length > 0) {
            drainedEvents.push(...streamed.events);
          }
          const advancedCursor = Number(streamed.next_cursor ?? nextCursor);
          if (!streamed.has_more || advancedCursor === nextCursor) {
            nextCursor = advancedCursor;
            break;
          }
          nextCursor = advancedCursor;
        }
        if (nextCursor !== cursorRef.current) {
          cursorRef.current = nextCursor;
          onCursorChange(nextCursor);
        }
        if (drainedEvents.length > 0) {
          onEvents(drainedEvents);
        }
      }
      const snapshot = await withRuntimeTimeout(
        getRuntimeSnapshot(bootstrap, { session_id: effectiveSessionId }),
        10_000,
        "runtime.get_snapshot",
      );
      if (shouldApplyRuntimeSnapshotState(Number(snapshot.event_count ?? 0), cursorRef.current)) {
        onSnapshotState(snapshot.state);
      }
    } catch (error) {
      onError(String((error as Error).message ?? error));
      onSnapshotState("failed");
    } finally {
      pollingRef.current = false;
    }
  }, [
    bootstrap,
    cursorRef,
    onCursorChange,
    onError,
    onEvents,
    onSnapshotState,
    sessionId,
    tauriPushActive,
  ]);

  useEffect(() => {
    if (!sessionId || !bootstrap) {
      return;
    }
    const intervalMs = bootstrap.transport === "tauri" && tauriPushActive ? 1200 : 400;
    const timer = setInterval(() => {
      void pollRuntimeEvents();
    }, intervalMs);
    return () => clearInterval(timer);
  }, [bootstrap, pollRuntimeEvents, sessionId, tauriPushActive]);

  useEffect(() => {
    if (!sessionId || !bootstrap || bootstrap.transport !== "tauri") {
      return;
    }
    void ensureTauriSubscription(bootstrap, sessionId);
  }, [bootstrap, ensureTauriSubscription, sessionId]);

  useEffect(() => {
    return () => {
      clearRuntimeSubscription();
    };
  }, [clearRuntimeSubscription]);

  return {
    clearRuntimeSubscription,
    ensureRuntimeSubscription: ensureTauriSubscription,
    pollRuntimeEvents,
    tauriPushActive,
  };
}
