import { translate } from "./i18n";
import type { ServerEntry } from "./types";

/** What a person calls a launch input. Inputs created from pasted arguments
 * carry a generated label, so name them after the flag they follow. Mirrors
 * `launch_inputs::input_display_name`. */
export function inputDisplayName(server: ServerEntry, key: string): string {
  const launch = server.launch;
  const label = launch?.inputs.find((input) => input.key === key)?.label ?? key;
  if (!launch || !label.startsWith("Imported argument")) return label;
  const index = launch.bindings.find(
    (binding) =>
      binding.parts.length === 1 &&
      binding.parts[0].kind === "input" &&
      binding.parts[0].key === key,
  )?.index;
  if (index === undefined) return label;
  const flag = server.args[index - 1];
  return index > 0 && flag?.startsWith("-")
    ? translate("Value for {flag} (argument {position})", { flag, position: index + 1 })
    : translate("Secret argument {position}", { position: index + 1 });
}
