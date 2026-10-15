import { msg } from "@/lib/i18n";
export const SETTINGS_PAGES = [
  {
    id: "general",
    label: msg("General"),
    description: msg("Startup, appearance and updates."),
  },
  {
    id: "tools",
    label: msg("Tools"),
    description: msg("How agents find and use your tools."),
  },
  {
    id: "safety",
    label: msg("Safety"),
    description: msg("How Toolport handles risky tool calls."),
  },
  {
    id: "access",
    label: msg("Access"),
    description: msg("Limit which servers and tools each client can use."),
  },
  {
    id: "connections",
    label: msg("Connections"),
    description: msg("Your local HTTP endpoint and gateway processes."),
  },
  {
    id: "help",
    label: msg("Help and data"),
    description: msg("Support reports, local files and removing Toolport."),
  },
] as const;
export type SettingsSubpage = (typeof SETTINGS_PAGES)[number]["id"];
