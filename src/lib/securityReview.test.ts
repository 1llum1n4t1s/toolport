import { describe, expect, it } from "vitest";
import type { SecurityEvent } from "./api";
import {
  dismissalKey,
  dismissalIndex,
  eventSeverity,
  securityKey,
  selectSecurityEvents,
} from "./securityReview";

// Previous production selector, retained here only for equivalent-output timing.
function previousSelector(events: SecurityEvent[], dismissed: Set<string>) {
  const kept: SecurityEvent[] = [];
  for (const event of [...events].sort((a, b) => b.ts - a.ts)) {
    if (
      !kept.some(
        (other) =>
          other.type === event.type &&
          other.server === event.server &&
          (other.profile || "") === (event.profile || "") &&
          other.tool === event.tool &&
          other.change === event.change &&
          (other.severity ?? eventSeverity(other)) ===
            (event.severity ?? eventSeverity(event)) &&
          (event.type !== "tool_drift" || other.new_fp === event.new_fp) &&
          Math.abs(other.ts - event.ts) <= 600_000,
      )
    )
      kept.push(event);
  }
  return kept.filter((event) => {
    const identity = securityKey(event);
    if (
      event.type === "tool_drift" &&
      !event.new_fp &&
      event.blocked === false &&
      dismissed.has(identity)
    )
      return false;
    if (event.type !== "tool_drift" && eventSeverity(event) !== "high")
      return !dismissed.has(identity);
    const prefix = `${identity}@`;
    for (const marker of dismissed) {
      if (marker.startsWith(prefix) && Number(marker.slice(prefix.length)) >= event.ts)
        return false;
    }
    return true;
  });
}

const event = (tool: string, ts = 1000): SecurityEvent => ({
  type: "tool_drift",
  server: "srv",
  tool,
  change: "changed",
  severity: "warn",
  new_fp: `v3:${ts}`,
  blocked: false,
  ts,
});

describe("security review selection", () => {
  it("preserves legacy reviews without hiding current or blocked definitions", () => {
    const old = { ...event("srv__read"), new_fp: undefined };
    const markers = new Set([securityKey(old)]);
    expect(selectSecurityEvents([old], markers)).toEqual([]);
    expect(selectSecurityEvents([{ ...old, blocked: null }], markers)).toHaveLength(1);
    expect(selectSecurityEvents([event("srv__read")], markers)).toHaveLength(1);
  });
  it("indexes markers by identity and retains later occurrences", () => {
    const reviewed = event("srv__read");
    const markers = new Set([dismissalKey(reviewed)]);
    expect(selectSecurityEvents([reviewed, event("srv__read", 1001)], markers)).toEqual([
      event("srv__read", 1001),
    ]);
    expect(dismissalIndex(markers).get(securityKey(reviewed))).toBe(1000);
  });
  it("keeps a shared change visible when another profile blocks it after review", () => {
    const shared = {
      ...event("srv__read"),
      profiles: ["personal", "work"],
      blocked: true,
      blocked_profiles: ["work"],
    };
    const markers = new Set([dismissalKey(shared)]);
    expect(selectSecurityEvents([shared], markers)).toEqual([shared]);
    expect(
      selectSecurityEvents(
        [{ ...shared, blocked: false, blocked_profiles: [] }],
        markers,
      ),
    ).toEqual([]);
  });
  it("selects the same rows on a 2000-event fixture with indexed lookups", () => {
    const events = Array.from({ length: 2000 }, (_, i) => event(`srv__read_${i}`));
    const dismissed = new Set(
      Array.from({ length: 2000 }, (_, i) => dismissalKey(event(`srv__read_${i}`, 999))),
    );
    const start = performance.now();
    const before = previousSelector(events, dismissed);
    const middle = performance.now();
    const after = selectSecurityEvents(events, dismissed);
    const end = performance.now();
    console.log(
      `2000-event selector: before ${(middle - start).toFixed(2)}ms, after ${(end - middle).toFixed(2)}ms`,
    );
    expect(after).toEqual(before);
    expect(after).toHaveLength(2000);
  });
});
