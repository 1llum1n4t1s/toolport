/** A factual reachability baseline. Security posture remains in Settings; this only
 * summarizes the health probe and never presents a stale or partial check as healthy. */
export function serverPostureCopy({
  backendReachable,
  probing,
  enabled,
  checked,
  connected,
  attention,
  disabled,
}: {
  backendReachable: boolean;
  probing: boolean;
  enabled: number;
  checked: number;
  connected: number;
  attention: number;
  disabled: number;
}) {
  const complete = enabled > 0 && checked === enabled && !probing;
  const healthy = backendReachable && complete && attention === 0;
  const title = !backendReachable
    ? "Reachability status unavailable"
    : enabled === 0
      ? "No servers enabled"
      : probing
        ? "Checking server reachability"
        : checked < enabled
          ? "Reachability check incomplete"
          : attention === 0
            ? `${connected} enabled server${connected === 1 ? "" : "s"} reachable`
            : `${connected} of ${enabled} enabled servers reachable`;
  const detail = !backendReachable
    ? checked > 0
      ? attention > 0
        ? `Last known: ${connected} reachable; ${attention} need${attention === 1 ? "s" : ""} a quick check. Status may be out of date.`
        : `Last known: ${connected} reachable. Status may be out of date.`
      : "The last health check did not complete."
    : enabled === 0
      ? `${disabled} server${disabled === 1 ? "" : "s"} turned off.`
      : probing
        ? `${checked} of ${enabled} checked so far.`
        : checked < enabled
          ? `${checked} of ${enabled} checked. Refresh to try again.`
          : attention > 0
            ? `${attention} need${attention === 1 ? "s" : ""} a quick check.`
            : disabled > 0
              ? `${disabled} turned off.`
              : "Every enabled server is ready.";
  return { healthy, title, detail };
}
