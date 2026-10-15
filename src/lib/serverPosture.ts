import { translate } from "@/lib/i18n";
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
    ? translate("Reachability status unavailable")
    : enabled === 0
      ? translate("No servers enabled")
      : probing
        ? translate("Checking server reachability")
        : checked < enabled
          ? translate("Reachability check incomplete")
          : attention === 0
            ? connected === 1
              ? translate("{connected} enabled server reachable", { connected })
              : translate("{connected} enabled servers reachable", { connected })
            : translate("{connected} of {enabled} enabled servers reachable", {
                connected,
                enabled,
              });
  const detail = !backendReachable
    ? checked > 0
      ? attention > 0
        ? attention === 1
          ? translate(
              "Last known: {connected} reachable; {attention} needs a quick check. Status may be out of date.",
              { connected, attention },
            )
          : translate(
              "Last known: {connected} reachable; {attention} need a quick check. Status may be out of date.",
              { connected, attention },
            )
        : translate("Last known: {connected} reachable. Status may be out of date.", {
            connected,
          })
      : translate("The last health check did not complete.")
    : enabled === 0
      ? disabled === 1
        ? translate("{disabled} server turned off.", { disabled })
        : translate("{disabled} servers turned off.", { disabled })
      : probing
        ? translate("{checked} of {enabled} checked so far.", { checked, enabled })
        : checked < enabled
          ? translate("{checked} of {enabled} checked. Refresh to try again.", {
              checked,
              enabled,
            })
          : attention > 0
            ? attention === 1
              ? translate("{attention} needs a quick check.", { attention })
              : translate("{attention} need a quick check.", { attention })
            : disabled > 0
              ? translate("{disabled} turned off.", { disabled })
              : translate("Every enabled server is ready.");
  return { healthy, title, detail };
}
