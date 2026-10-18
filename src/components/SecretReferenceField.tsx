import { translate, useI18n } from "@/lib/i18n";
import { useState } from "react";
import { testSecretReference } from "@/lib/api";
import { SECRET_PROVIDERS, referenceProvider } from "@/lib/secretRefs";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";

export function SecretReferenceField({
  serverId,
  value,
  onChange,
}: {
  serverId: string;
  value: string;
  onChange: (reference: string) => void;
}) {
  const { t } = useI18n();
  const [result, setResult] = useState("");
  const [busy, setBusy] = useState(false);
  const provider = referenceProvider(value) ?? SECRET_PROVIDERS[0];
  function change(reference: string) {
    setResult("");
    onChange(reference);
  }
  async function test() {
    setBusy(true);
    setResult("");
    try {
      await testSecretReference(serverId, value);
      setResult(
        translate(
          "Success in the desktop app environment. The MCP client gateway may use different environment variables or PATH.",
        ),
      );
    } catch (error) {
      setResult(
        typeof error === "object" && error && "message" in error
          ? String(error.message)
          : translate(
              "Could not test the reference. Check the provider locally and retry.",
            ),
      );
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="flex w-full flex-col gap-2">
      <label className="text-meta">
        {t("Provider")}
        <select
          aria-label={t("Password manager provider")}
          className="ml-2 rounded border bg-background p-1"
          value={provider.scheme}
          disabled={busy}
          onChange={(e) =>
            change(SECRET_PROVIDERS.find((p) => p.scheme === e.target.value)!.scheme)
          }
        >
          {SECRET_PROVIDERS.map((p) => (
            <option key={p.scheme} value={p.scheme}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <div className="flex gap-2">
        <Input
          aria-label={t("Secret reference")}
          value={value}
          disabled={busy}
          placeholder={provider.example}
          autoComplete="off"
          onChange={(e) => change(e.target.value)}
        />
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={busy || !value}
          onClick={() => void test()}
        >
          {busy ? t("Testing…") : t("Test")}
        </Button>
      </div>
      <p className="text-meta text-muted-foreground">
        {t(
          "Test uses the desktop app environment. The MCP client gateway may use different environment variables or PATH. Only the reference syncs. Sign in to this provider on each machine.",
        )}
      </p>
      {result && (
        <p role="status" className="text-meta">
          {result}
        </p>
      )}
    </div>
  );
}
