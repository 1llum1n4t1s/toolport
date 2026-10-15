import { getLocale, translate } from "./i18n";
import type { KeptV1Safety } from "@/lib/types";

// Same wording as `KeptV1Safety::summary` in src-tauri/src/registry.rs.
export function keptSafetySummary(kept: KeptV1Safety | undefined): string | null {
  if (!kept) return null;
  const parts = [
    kept.holdUntrusted && translate("asks before calls from shared or registry servers"),
    kept.denyDestructive && translate("hides destructive tools"),
    kept.quarantineOnDrift && translate("pauses tools whose definitions change"),
    kept.blockOnInjection && translate("blocks results that look like prompt injection"),
  ].filter((part): part is string => Boolean(part));
  if (parts.length === 0) return null;
  const list = new Intl.ListFormat(getLocale(), {
    style: "long",
    type: "conjunction",
  }).format(parts);
  return translate("Kept from 1.x: Toolport also {protections}.", { protections: list });
}
