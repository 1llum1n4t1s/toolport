import { useEffect, useState } from "react";
import {
  addClientLocation,
  getClientLocationTypes,
  type ClientLocationType,
} from "@/lib/api";
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
  const [types, setTypes] = useState<ClientLocationType[]>([]);
  const [clientType, setClientType] = useState("claude-code");
  const [folder, setFolder] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!open) return;
    let current = true;
    setError(null);
    getClientLocationTypes()
      .then((rows) => {
        if (current) setTypes(rows);
      })
      .catch(() => {
        if (current)
          setError("Could not load client types. Close this dialog and try again.");
      });
    return () => {
      current = false;
    };
  }, [open]);
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
          <DialogTitle>Add client location</DialogTitle>
          <DialogDescription>
            Choose the folder this client uses for its configuration. Locations stay on
            this machine. Connecting is a separate step.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-4"
          onSubmit={async (event) => {
            event.preventDefault();
            setSaving(true);
            setError(null);
            try {
              const id = await addClientLocation(clientType, folder);
              await onAdded(id);
              setFolder("");
              onOpenChange(false);
            } catch (error) {
              setError(error instanceof Error ? error.message : String(error));
            } finally {
              setSaving(false);
            }
          }}
        >
          <label className="flex flex-col gap-2 text-sm">
            Client type
            <select
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
          </label>
          <label className="flex flex-col gap-2 text-sm">
            Config folder
            <Input
              value={folder}
              onChange={(event) => setFolder(event.target.value)}
              placeholder="~/.claude2"
              disabled={saving}
              autoFocus
            />
          </label>
          {selected && (
            <p className="text-xs text-muted-foreground">
              Toolport will use {selected.configFile} in this folder. For Codex, choose
              its CODEX_HOME folder.
            </p>
          )}
          {error && (
            <p role="alert" className="text-sm text-destructive">
              {error}
            </p>
          )}
          <Button type="submit" disabled={saving || !folder.trim() || !selected}>
            {saving ? "Adding…" : "Add location"}
          </Button>
        </form>
      </DialogContent>
    </Dialog>
  );
}
