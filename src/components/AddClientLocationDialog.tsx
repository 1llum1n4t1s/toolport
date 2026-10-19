import { technicalText } from "@/lib/i18n";
import { translate, useI18n } from "@/lib/i18n";
import { useEffect, useId, useState } from "react";
import {
  addClientLocation,
  getClientLocationTypes,
  type ClientLocationType,
} from "@/lib/api";
import { SkeletonRows } from "@/components/ui/skeleton";
import { ErrorCallout } from "@/components/ui/status-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function AddClientLocationDialog({
  open,
  onOpenChange,
  onAdded,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onAdded: (id: string) => Promise<void>;
}) {
  const { t } = useI18n();
  const typeId = useId();
  const folderId = useId();
  const [types, setTypes] = useState<ClientLocationType[]>([]);
  const [clientType, setClientType] = useState("claude-code");
  const [folder, setFolder] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [errorSource, setErrorSource] = useState<"load" | "submit" | null>(null);
  const [saving, setSaving] = useState(false);
  const [loading, setLoading] = useState(true);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!open) return;
    let current = true;
    // eslint-disable-next-line react-hooks/set-state-in-effect -- Clear the previous dialog error as the client-type request starts.
    setError(null);
    setErrorSource(null);
    setLoading(true);
    getClientLocationTypes()
      .then((rows) => {
        if (current) setTypes(rows);
      })
      .catch(() => {
        if (current) {
          setErrorSource("load");
          setError(translate("Could not load client types."));
        }
      })
      .finally(() => {
        if (current) setLoading(false);
      });
    return () => {
      current = false;
    };
  }, [open, retry]);
  const selected = types.find((type) => type.id === clientType);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!saving) onOpenChange(next);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("Add client location")}</DialogTitle>
          <DialogDescription>
            {t(
              "Choose the folder this client uses for its configuration. Locations stay on this machine. Connecting is a separate step.",
            )}
          </DialogDescription>
        </DialogHeader>
        {loading && <SkeletonRows label={t("Loading…")} />}
        <form
          className="flex flex-col gap-4"
          onSubmit={async (event) => {
            event.preventDefault();
            setSaving(true);
            setError(null);
            setErrorSource(null);
            try {
              const id = await addClientLocation(clientType, folder);
              await onAdded(id);
              setFolder("");
              onOpenChange(false);
            } catch (error) {
              setErrorSource("submit");
              setError(error instanceof Error ? error.message : String(error));
            } finally {
              setSaving(false);
            }
          }}
        >
          <div className="flex flex-col gap-2 text-sm">
            <label htmlFor={typeId}>{t("Client type")}</label>
            <select
              id={typeId}
              className="h-9 rounded-md border border-input bg-background px-3"
              value={clientType}
              onChange={(event) => setClientType(event.target.value)}
              disabled={saving || types.length === 0}
            >
              {types.map((type) => (
                <option key={type.id} value={type.id}>
                  {type.name}
                </option>
              ))}
            </select>
          </div>
          <div className="flex flex-col gap-2 text-sm">
            <label htmlFor={folderId}>{t("Config folder")}</label>
            <Input
              id={folderId}
              value={folder}
              onChange={(event) => setFolder(event.target.value)}
              placeholder={technicalText("~/.claude2")}
              disabled={saving}
              autoFocus
            />
          </div>
          {selected && (
            <p className="text-meta text-muted-foreground">
              {t(
                "Toolport will use {file} in this folder. For Codex, choose its CODEX_HOME folder.",
                { file: selected.configFile },
              )}
            </p>
          )}
          {error && (
            <ErrorCallout
              title={t("Add client location")}
              cause={error}
              onRetry={
                errorSource === "load" ? () => setRetry((value) => value + 1) : undefined
              }
            />
          )}
          <Button type="submit" disabled={saving || !folder.trim() || !selected}>
            {saving ? t("Adding…") : t("Add location")}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
