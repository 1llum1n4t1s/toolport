import type { ServerEntry } from "./types";

// Exported for unit tests: security-relevant classifier behind the
// "Runs a shell command" warning above.
export function runsShell(command: string | null, args: string[] = []): boolean {
  if (!command) return false;
  let cmd = command;
  let argv = args;
  // Backend `normalize_invocation`: a packed `"bash -c '…'"` with empty args is
  // split, but only when the first token is a bare program name (no path) — the
  // same rule `isDownloadLauncher` mirrors, so a path with spaces stays intact.
  if (args.length === 0) {
    const parts = command.split(/\s+/).filter(Boolean);
    const first = parts[0] ?? "";
    if (parts.length > 1 && !first.includes("/") && !first.includes("\\")) {
      cmd = first;
      argv = parts.slice(1);
    }
  }
  const base = cmd
    .replace(/\\/g, "/")
    .split("/")
    .pop()!
    .toLowerCase()
    .replace(/\.(exe|cmd|bat)$/, "");
  // `env` just launches a command; classify that instead. GNU env options that
  // take a separate operand (-u NAME, -C DIR) must be skipped with their value,
  // or `env -u FOO bash -c '…'` would classify FOO and miss the shell. Unknown
  // options (notably -S, which repacks a whole command line) classify as shell
  // conservatively rather than silently dropping the warning.
  if (base === "env") {
    for (let i = 0; i < argv.length; i++) {
      const a = argv[i];
      if (a.startsWith("--")) {
        const name = a.split("=")[0];
        if (name === "--ignore-environment") continue;
        if (name === "--unset" || name === "--chdir") {
          if (!a.includes("=")) i++;
          continue;
        }
        return true;
      }
      if (a.startsWith("-")) {
        if (a === "-" || a === "-i") continue; // both mean ignore-environment
        if (a === "-u" || a === "-C") {
          i++;
          continue;
        }
        return true;
      }
      if (a.includes("=")) continue; // NAME=VALUE assignment
      return runsShell(a, argv.slice(i + 1));
    }
    return false;
  }
  return ["cmd", "sh", "bash", "zsh", "fish", "powershell", "pwsh"].includes(base);
}

// Exported for unit tests: security-relevant classifier behind the
// "Connects to a private or internal address" warning above.
export function isPrivateHostUrl(url: string | null | undefined): boolean {
  if (!url) return false;
  let host: string;
  try {
    // WHATWG keeps a trailing dot on named hosts (localhost.) but strips it on
    // IPv4 literals — strip for both so loopback warnings stay consistent.
    host = new URL(url).hostname
      .toLowerCase()
      .replace(/^\[|\]$/g, "")
      .replace(/\.$/, "");
  } catch {
    return false;
  }
  if (
    host === "localhost" ||
    host.endsWith(".localhost") ||
    host === "::1" ||
    host === "0:0:0:0:0:0:0:1"
  ) {
    return true;
  }
  // IPv4-mapped IPv6 — WHATWG may emit dotted or hex form (::ffff:127.0.0.1 / ::ffff:7f00:1)
  const v4MappedDotted = host.match(/^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/i);
  if (v4MappedDotted) {
    return isPrivateIpv4(v4MappedDotted[1]);
  }
  const v4MappedHex = host.match(/^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/i);
  if (v4MappedHex) {
    const hi = parseInt(v4MappedHex[1], 16);
    const lo = parseInt(v4MappedHex[2], 16);
    const dotted = `${(hi >> 8) & 0xff}.${hi & 0xff}.${(lo >> 8) & 0xff}.${lo & 0xff}`;
    return isPrivateIpv4(dotted);
  }
  // IPv6: loopback, unspecified, link-local fe80::/10, ULA fc00::/7
  if (host.includes(":")) {
    if (host === "::" || host === "0:0:0:0:0:0:0:0") return true;
    const first = parseInt(host.split(":")[0] || "0", 16);
    if (Number.isNaN(first)) return false;
    if ((first & 0xffc0) === 0xfe80) return true; // link-local
    if ((first & 0xfe00) === 0xfc00) return true; // unique-local
    return false;
  }
  return isPrivateIpv4(host);
}

/** Team-synced local commands and LAN URLs stay off until the member confirms.
 * Mirrors `ServerEntry::needs_team_enable_review` so the Servers Switch and
 * Enable all cannot skip the Teams review dialog.
 *
 * Deliberately an AFFORDANCE, not the security boundary. It cannot match the Rust
 * check exactly: `host_is_private` resolves named hosts (and treats an unresolvable
 * one as private), and a renderer cannot do DNS, so `http://internal-service/mcp`
 * pointing at 10.0.0.5 reads as public here. `set_server_enabled` decides for real
 * and refuses without an explicit reviewed flag, so a miss here costs a clear error
 * rather than an unreviewed enable. */
export function needsTeamEnableReview(
  server: Pick<
    ServerEntry,
    "source" | "transport" | "command" | "url" | "personalSyncEntry" | "teamEnableReview"
  > &
    Partial<Pick<ServerEntry, "env" | "headerKeys" | "launch">>,
): boolean {
  if (
    (server.source?.startsWith("team:") || server.source === "shared") &&
    (server.env?.some((e) => e.source) ||
      server.headerKeys?.some((h) => h.source) ||
      server.launch?.inputs.some((i) => i.source))
  )
    return true;
  if (server.personalSyncEntry) return server.teamEnableReview === true;
  if (!server.source?.startsWith("team:")) return false;
  if (server.transport === "stdio" || !!server.command) return true;
  // Anything that is not a plain https:// URL to a dotted public name is treated as
  // needing review: a bare hostname is an intranet name far more often than not, and
  // over-prompting is the safe direction for a dialog whose whole job is consent.
  return isPrivateHostUrl(server.url) || !isPublicLookingHttpsUrl(server.url);
}

/** True only for `https://` URLs whose host is a dotted name or a public literal IP.
 * A bare hostname (`https://internal-service/mcp`) and any plaintext `http://` fall
 * to false, so [`needsTeamEnableReview`] asks. */
function isPublicLookingHttpsUrl(url: string | null | undefined): boolean {
  if (!url) return false;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  if (parsed.protocol !== "https:") return false;
  const host = parsed.hostname.replace(/^\[|\]$/g, "");
  if (isPrivateHostUrl(url)) return false;
  return host.includes(".") && !host.endsWith(".local") && !host.endsWith(".internal");
}

/** The fields a member actually reviews before enabling a team server. `args` is
 * optional so an entry that omits it compares equal to one with an empty list. */
export type ReviewedFields = {
  transport: ServerEntry["transport"];
  command: ServerEntry["command"];
  args?: ServerEntry["args"];
  url: ServerEntry["url"];
  env?: ServerEntry["env"];
  headerKeys?: ServerEntry["headerKeys"];
  launch?: ServerEntry["launch"];
  cwd?: ServerEntry["cwd"];
  inheritEnv?: ServerEntry["inheritEnv"];
};

/** Compared on confirm so a team push landing mid-dialog cannot swap the definition
 * underneath the member (the confirmation carries only the server id). */
export function sameReviewedDefinition(a: ReviewedFields, b: ReviewedFields): boolean {
  return (
    a.transport === b.transport &&
    (a.command ?? "") === (b.command ?? "") &&
    (a.url ?? "") === (b.url ?? "") &&
    JSON.stringify(a.args ?? []) === JSON.stringify(b.args ?? []) &&
    JSON.stringify(a.env ?? []) === JSON.stringify(b.env ?? []) &&
    JSON.stringify(a.headerKeys ?? []) === JSON.stringify(b.headerKeys ?? []) &&
    JSON.stringify(a.launch ?? null) === JSON.stringify(b.launch ?? null) &&
    (a.inheritEnv ?? false) === (b.inheritEnv ?? false) &&
    (a.cwd ?? "") === (b.cwd ?? "")
  );
}

/** Mirror src-tauri/src/oauth.rs ip_is_private for IPv4. */
function isPrivateIpv4(host: string): boolean {
  const match = host.match(/^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/);
  if (!match) return false;
  const a = Number(match[1]);
  const b = Number(match[2]);
  const c = Number(match[3]);
  const d = Number(match[4]);
  if ([a, b, c, d].some((n) => n > 255)) return false;
  return (
    a === 127 || // loopback
    a === 10 || // RFC1918
    // 0.0.0.0/8 "this network". Deliberately broader than Rust's is_unspecified(),
    // which is only 0.0.0.0. Warning on the whole block is the safe direction here.
    a === 0 ||
    (a === 192 && b === 168) ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 169 && b === 254) || // link-local
    (a === 100 && (b & 0xc0) === 64) || // CGNAT 100.64/10
    (a === 255 && b === 255 && c === 255 && d === 255) // broadcast
  );
}
