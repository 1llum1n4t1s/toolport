import { getLocale, msg, translate, type SourceMessage } from "./i18n";
// Same wording as `removed_features_message` and friends in src-tauri/src/registry.rs.
const LABELS: Record<string, SourceMessage> = {
  agentRules: msg("Agent rules"),
  agentPermissions: msg("Agent permissions"),
  activityHooks: msg("Agent activity hooks"),
  routines: msg("Routines"),
  agentControl: msg("Agent control"),
};

export const GO_BACK_TO_1X_URL =
  "https://github.com/btsouth/toolport/blob/main/docs/upgrading-to-2.md#go-back-to-124";

export function removedFeatureNames(features: string[]): string[] {
  return features.map((id) => (LABELS[id] ? translate(LABELS[id]) : id));
}

export function removedFeaturesMessage(features: string[]): string {
  const names = removedFeatureNames(features);
  const list = new Intl.ListFormat(getLocale(), {
    style: "long",
    type: "conjunction",
  }).format(names);
  return translate(
    "Toolport 2.0 no longer includes {features}, which you used in 1.x. Your settings for them are saved in the exports folder, and files Toolport wrote for them were left as they were. Tell us if you need one back, or go back to 1.24.",
    { features: list },
  );
}

export function removedFeaturesIssueUrl(features: string[]): string {
  const names = removedFeatureNames(features).join(", ");
  const title = translate("I need {features} in Toolport 2.0", { features: names });
  const body = translate("Toolport 2.0 removed: {features}.\n\nWhat I used it for:\n\n", {
    features: names,
  });
  return `https://github.com/btsouth/toolport/issues/new?title=${encodeURIComponent(title)}&body=${encodeURIComponent(body)}`;
}
