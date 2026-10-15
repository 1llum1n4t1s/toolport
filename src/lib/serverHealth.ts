import { translate } from "./i18n";
import type { ProbeResult } from "./types";
import { errorHeadline } from "./errors";

export function serverFailureLabel(health: ProbeResult): string {
  if (health.authRequired)
    return health.authTarget === "scope"
      ? translate("Permission required")
      : health.authTarget === "service_credential"
        ? translate("Service key required")
        : translate("Needs sign-in");
  if (health.error?.includes("downstream server exited"))
    return translate("Server stopped");
  if (health.error?.includes("Broken pipe")) return translate("Connection closed");
  switch (health.failure?.kind) {
    case "auth":
      return health.failure.target === "scope"
        ? translate("Permission required")
        : translate("Needs sign-in");
    case "timeout":
      return translate("Timed out");
    case "unavailable":
      return translate("Unreachable");
    case "server_error":
      return translate("Server failed");
    case "quota":
      return translate("Rate limited");
    case "cancelled":
      return translate("Check cancelled");
    default:
      return health.error ? errorHeadline(health.error) : translate("Connection failed");
  }
}
