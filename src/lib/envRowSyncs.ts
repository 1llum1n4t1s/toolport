import { riskySyncEnv, syncsByDefault } from "./personalSync";

/** What "Same on every machine" shows for a plain value. A saved or chosen
 * answer wins. A name the server did not have yet gets sync's default; an old
 * value nobody marked stays on this machine. */
export function envRowSyncs(
  row: { key: string; value: string; portable?: boolean },
  saved: { key: string }[],
): boolean {
  if (riskySyncEnv(row.key)) return false;
  if (row.portable !== undefined) return row.portable;
  const key = row.key.trim();
  return !saved.some((e) => e.key === key) && syncsByDefault(key, row.value);
}
