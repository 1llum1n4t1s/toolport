import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// MSI accepts numeric versions only. Keep the app and NSIS preview version intact.
export function ciMsiConfig(version) {
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$/.exec(version);
  if (
    !match ||
    match.slice(1).some((part, index) => Number(part) > (index < 2 ? 255 : 65535))
  ) {
    throw new Error(`Invalid MSI version: ${version}`);
  }
  return { bundle: { windows: { wix: { version: match.slice(1).join(".") } } } };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { version } = JSON.parse(readFileSync("src-tauri/tauri.conf.json", "utf8"));
  writeFileSync(
    "src-tauri/tauri.ci-msi.conf.json",
    JSON.stringify(ciMsiConfig(version)) + "\n",
  );
}
