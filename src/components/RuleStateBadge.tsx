import { useI18n, msg, type SourceMessage } from "@/lib/i18n";
import { AlertTriangle, Ban, Check, Clock, Minus } from "lucide-react";
import type { InstructionsApplyState } from "@/lib/types";

/**
 * How each per-client instructions state renders in the Teams tab (org instructions, spec W4).
 */
const RULE_STATE_META: Record<
  InstructionsApplyState,
  { label: SourceMessage; className: string; Icon: typeof Check }
> = {
  applied: { label: msg("Applied"), className: "text-success", Icon: Check },
  stale: { label: msg("Not applied yet"), className: "text-warning", Icon: Clock },
  blocked_override: {
    label: msg("Blocked by a local override"),
    className: "text-warning",
    Icon: Ban,
  },
  too_long: {
    label: msg("Too long for this client"),
    className: "text-warning",
    Icon: AlertTriangle,
  },
  unsupported: {
    label: msg("Copy manually"),
    className: "text-muted-foreground",
    Icon: Minus,
  },
  error: {
    label: msg("Write error"),
    className: "text-destructive",
    Icon: AlertTriangle,
  },
};

/** Why a client is in this state, in one sentence. Surfaced as the badge's tooltip. */
const EXPLANATION: Record<InstructionsApplyState, SourceMessage> = {
  applied: msg("This client's rules file is up to date."),
  stale: msg("The current rules are not on disk for this client yet."),
  blocked_override: msg(
    "This client has a local override file that makes it ignore the file Toolport writes.",
  ),
  too_long: msg(
    "This client caps its global rules file, and these rules would exceed it.",
  ),
  unsupported: msg(
    "This client has no global rules file Toolport can write. Paste the rules in by hand.",
  ),
  error: msg("The rules file could not be read or written. It was left untouched."),
};

/** The state badge for one client, icon + label. */
export function RuleStateBadge({ state }: { state: InstructionsApplyState }) {
  const { t: tr } = useI18n();
  const meta = RULE_STATE_META[state];
  const Icon = meta.Icon;
  return (
    <span
      title={tr(EXPLANATION[state])}
      className={`flex shrink-0 items-center gap-1 text-xs ${meta.className}`}
    >
      <Icon className="size-3.5" />
      {tr(meta.label)}
    </span>
  );
}
