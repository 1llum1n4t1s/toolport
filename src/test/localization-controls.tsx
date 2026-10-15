// Development-only probes for browser localization checks.
import { useState } from "react";
import { useI18n, LANGUAGES, type LanguageChoice } from "@/lib/i18n";
import { TeamSharePreview } from "@/components/TeamSharePreview";
import ExecutionReview from "@/components/ExecutionReview";
import type { Registry, ServerEntry } from "@/lib/types";
import type { TeamPushPreview } from "@/lib/api";
import { fmtDollars } from "@/lib/utils";

const preview: TeamPushPreview = {
  baseVersion: 1,
  localFingerprint: "fixture-fingerprint",
  added: ["fixture-added"],
  changed: [],
  removed: [],
  definitions: [
    {
      id: "added",
      name: "fixture-added",
      transport: "stdio",
      change: "Added",
      fields: [{ label: "Command", value: "fixture-command --keep-raw" }],
    },
  ],
  selections: [],
};

export function LocalizationControls({
  server,
}: {
  server: ServerEntry;
  registry: Registry;
}) {
  const { language, locale, setLanguage, loading, error, saveError, t } = useI18n();
  const [draft, setDraft] = useState("");
  return (
    <main className="p-6 flex flex-col gap-6 text-foreground bg-background">
      <h1 data-testid="language-heading">{t("Language")}</h1>
      <select
        id="fixture-language"
        aria-label="Fixture language"
        aria-busy={loading}
        value={language}
        onChange={(event) => void setLanguage(event.target.value as LanguageChoice)}
      >
        <option value="system">{t("System language")}</option>
        {LANGUAGES.map(({ code, label }) => (
          <option value={code} key={code}>
            {label}
          </option>
        ))}
      </select>
      <input
        data-testid="draft"
        aria-label="Fixture draft"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
      />
      {error && (
        <p role="alert" data-testid="dictionary-error">
          {t("Couldn't load this language. Reload Toolport to retry.")}
        </p>
      )}
      {saveError && (
        <p role="alert" data-testid="save-error">
          {t("Couldn't save the language setting. Try again.")}
        </p>
      )}
      <output data-testid="number">
        {new Intl.NumberFormat(locale).format(1234.56)}
      </output>
      <output data-testid="date">
        {new Intl.DateTimeFormat(locale, { timeZone: "UTC", dateStyle: "long" }).format(
          new Date("2026-10-14T12:00:00Z"),
        )}
      </output>
      <output data-testid="currency">{fmtDollars(1234.56)}</output>
      <section data-testid="share-preview">
        <TeamSharePreview preview={preview} />
      </section>
      <section data-testid="execution-review">
        <ExecutionReview server={server} onReady={() => {}} />
      </section>
    </main>
  );
}
