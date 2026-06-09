import type { RuntimeEvent } from "./localRuntimeClient";

export function eventFallbackDedupKey(event: RuntimeEvent): string | null {
  if (event.event_type === "model.stream_delta") {
    return null;
  }
  const payload = (event.payload ?? {}) as Record<string, unknown>;
  const idLike = String(
    payload.tool_call_id ??
      payload.permission_id ??
      payload.plan_approval_id ??
      payload.call_id ??
      payload.stream_id ??
      "",
  ).trim();
  if (!idLike) {
    return null;
  }
  return `fallback:${event.event_type}:${idLike}`;
}

function canCoalesceStreamDelta(left: RuntimeEvent, right: RuntimeEvent): boolean {
  if (left.event_type !== "model.stream_delta" || right.event_type !== "model.stream_delta") {
    return false;
  }
  const leftPayload = left.payload ?? {};
  const rightPayload = right.payload ?? {};
  return (
    String(leftPayload.stream_id ?? "default") === String(rightPayload.stream_id ?? "default") &&
    String(leftPayload.delta_kind ?? "content") === String(rightPayload.delta_kind ?? "content") &&
    Boolean(leftPayload.runtime_sanitized) === Boolean(rightPayload.runtime_sanitized)
  );
}

export function coalesceAdjacentRuntimeEvents(events: RuntimeEvent[]): RuntimeEvent[] {
  if (events.length < 2) {
    return events;
  }
  const coalesced: RuntimeEvent[] = [];
  for (const event of events) {
    const previous = coalesced[coalesced.length - 1];
    if (!previous || !canCoalesceStreamDelta(previous, event)) {
      coalesced.push(event);
      continue;
    }
    const previousPayload = previous.payload ?? {};
    const payload = event.payload ?? {};
    coalesced[coalesced.length - 1] = {
      ...event,
      event_id: previous.event_id,
      sequence: event.sequence ?? previous.sequence,
      payload: {
        ...previousPayload,
        ...payload,
        preview: `${String(previousPayload.preview ?? "")}${String(payload.preview ?? "")}`,
      },
    };
  }
  return coalesced;
}
