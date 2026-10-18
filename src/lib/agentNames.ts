/**
 * Up to 8 characters of an agent's name, for under its column logo in the Skills grid
 * ("Claude", "Copilot"). The GTK shell's `short_agent_name` in linux_native/skills.rs matches.
 */
export function shortAgentName(name: string): string {
  let short = name.replace(/\s+CLI$/i, "");
  if (short.length > 8) short = short.split(/\s+/)[0];
  return short.length > 8 ? `${short.slice(0, 7)}…` : short;
}
