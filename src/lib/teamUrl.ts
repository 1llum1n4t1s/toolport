import { translate } from "@/lib/i18n";
/** The hosted Toolport Teams **app**, prefilled as the default server URL. Self-hosters
 * replace it with their own server. */
export const HOSTED_TEAMS_URL = "https://teams.toolport.app";

/** The public **explainer** page — deliberately not [`HOSTED_TEAMS_URL`].
 *
 * Onboarding's "What is Toolport for Teams?" link targets this: someone reading it has
 * no team and no invite code yet, so sending them to the app would land them on a sign-in
 * for something they have not been told about. The two lived as three separate string
 * literals across two components, which is how they drifted (SBS-461). */
export const TEAMS_MARKETING_URL = "https://toolport.app/teams";

/** The pricing block on the explainer page. The exact numbers live there, not in
 * the app: the app ships on a release cadence and Stripe does not, so an in-app
 * dollar figure is a stale figure waiting to happen. The Teams tab states the
 * *shape* (free for a small team, paid past that, same price hosted or self-hosted)
 * and links here for the number. */
export const TEAMS_PRICING_URL = `${TEAMS_MARKETING_URL}#pricing`;

/** The self-hosting instructions on the explainer page. Self-hosting is not a
 * downgrade path here: the free tier is the same size either way, so this link is
 * offered next to the hosted one rather than buried under it. */
export const TEAMS_SELFHOST_URL = `${TEAMS_MARKETING_URL}#selfhost`;

/** Self-serve team creation, which exists only on the web. The desktop app has no
 * create-a-team flow, so "Create a free team" hands off to the hosted app.
 *
 * `intent=create-team` survives the OAuth/email round trip and drops the person on
 * team creation instead of the manage view; `from` is an attribution key, so
 * `app-teams-tab` separates people who found Teams inside the app they already
 * installed from the marketing-page funnel. Both are parsed in the hosted app's
 * `web/onboarding.js` (`authContext`). */
export const TEAMS_CREATE_URL = `${HOSTED_TEAMS_URL}/?intent=create-team&from=app-teams-tab`;

export function teamUrlError(raw: string): string | null {
  const value = raw.trim();
  if (!value) return translate("Server URL is required.");

  let url: URL;
  try {
    url = new URL(value);
  } catch {
    return translate("Team server URL must start with https://.");
  }

  if (url.protocol === "https:") return null;
  if (url.protocol !== "http:")
    return translate("Team server URL must start with https://.");

  const host = url.hostname.toLowerCase();
  const loopback =
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host === "127.0.0.1" ||
    host === "::1" ||
    host === "[::1]";

  return loopback
    ? null
    : translate(
        "Team server URL must use https:// unless it is loopback HTTP for local development.",
      );
}
