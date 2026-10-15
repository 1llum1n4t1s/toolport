import { translate } from "@/lib/i18n";
import type { TeamPushPreview } from "@/lib/api";

/** Whether sharing this preview uploads anything to the Team. */
export function teamShareUploads(preview: TeamPushPreview) {
  return preview.added.length + preview.changed.length + preview.removed.length > 0;
}

/** The confirm button's label, or null when there is nothing to upload or switch.
 * The GTK preview applies the same rule. */
export function teamShareAction(preview: TeamPushPreview): string | null {
  if (teamShareUploads(preview)) return translate("Share selected");
  if (preview.selections.some((s) => s.local.outcome === "switched"))
    return translate("Use Team copies");
  return null;
}
