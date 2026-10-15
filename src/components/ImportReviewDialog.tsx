import { useState, type ReactNode } from "react";
import { Check, Loader2, ShieldAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { ImportItem } from "@/lib/types";

import { runsShell, isPrivateHostUrl } from "@/lib/importReview";

interface Props {
  open: boolean;
  items: ImportItem[];
  busy?: boolean;
  title?: string;
  description?: string;
  confirmLabel?: string;
  allowEmpty?: boolean;
  requireCredentials?: boolean;
  details?: string;
  error?: string;
  onOpenChange: (open: boolean) => void;
  onConfirm: (
    keys: string[],
    secretChoices?: Record<string, Record<string, boolean>>,
    credentialInputs?: Record<string, Record<string, string>>,
  ) => void;
}

/** Review and choose detected client servers before adding them to Toolport. */
export function ImportReviewDialog({
  open,
  items,
  busy = false,
  title = "Review servers to import",
  description,
  confirmLabel,
  allowEmpty = false,
  requireCredentials = false,
  details,
  error,
  onOpenChange,
  onConfirm,
}: Props) {
  if (!open) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <ImportReviewContent
        items={items}
        busy={busy}
        title={title}
        description={description}
        confirmLabel={confirmLabel}
        allowEmpty={allowEmpty}
        requireCredentials={requireCredentials}
        details={details}
        error={error}
        onOpenChange={onOpenChange}
        onConfirm={onConfirm}
      />
    </Dialog>
  );
}

function ImportReviewContent({
  items,
  busy = false,
  title = "Review servers to import",
  description,
  confirmLabel,
  allowEmpty = false,
  requireCredentials = false,
  details,
  error,
  onOpenChange,
  onConfirm,
}: Omit<Props, "open">) {
  const keyedItems = items.map((item, index) => ({
    item,
    key: item.key ?? `${item.name}-${index}`,
  }));
  const [selected, setSelected] = useState<Set<string>>(
    () =>
      new Set(keyedItems.filter(({ item }) => !item.unsupported).map(({ key }) => key)),
  );

  const [secretChoices, setSecretChoices] = useState<
    Record<string, Record<string, boolean>>
  >(() =>
    Object.fromEntries(
      items.map((item) => [
        item.name,
        Object.fromEntries((item.credentials ?? []).map((env) => [env.key, env.secret])),
      ]),
    ),
  );
  const [credentialInputs, setCredentialInputs] = useState<
    Record<string, Record<string, string>>
  >({});
  const [editing, setEditing] = useState<{ name: string; key: string } | null>(null);
  const [value, setValue] = useState("");
  const selectedCount = selected.size;
  const missingRequired = requireCredentials
    ? keyedItems.flatMap(({ item, key }) =>
        selected.has(key)
          ? (item.credentials ?? [])
              .filter(
                (env) =>
                  env.required &&
                  !env.present &&
                  !credentialInputs[item.name]?.[env.key]?.trim(),
              )
              .map((env) => `Enter ${env.key} or deselect ${item.name}`)
          : [],
      )
    : [];

  return (
    <DialogContent className="sm:max-w-lg">
      <DialogHeader>
        <DialogTitle>{title}</DialogTitle>
      </DialogHeader>
      <div className="flex flex-col gap-4 py-1">
        <p className="text-xs text-muted-foreground">
          {description ??
            "Review the commands and URLs before adding them. Leave any server unchecked to keep it as it is."}
        </p>
        {details && (
          <details>
            <summary className="text-xs">Details</summary>
            <p className="mt-2 whitespace-pre-wrap break-all text-xs text-muted-foreground">
              {details}
            </p>
          </details>
        )}
        {error && (
          <p role="alert" className="text-sm text-warning">
            {error}
          </p>
        )}
        <div className="flex max-h-72 flex-col gap-2 overflow-y-auto">
          {keyedItems.map(({ item, key }) => {
            const isSelected = selected.has(key);
            return (
              <div key={key}>
                <button
                  type="button"
                  aria-pressed={isSelected}
                  disabled={busy || !!item.unsupported}
                  data-failed={!!error?.includes(item.name)}
                  className={`w-full rounded-md text-left transition-colors ${
                    error?.includes(item.name)
                      ? "ring-1 ring-warning"
                      : isSelected
                        ? "ring-1 ring-success/60"
                        : "opacity-60"
                  }`}
                  onClick={() =>
                    setSelected((previous) => {
                      const next = new Set(previous);
                      if (isSelected) next.delete(key);
                      else next.add(key);
                      return next;
                    })
                  }
                >
                  <ImportRow
                    item={item}
                    selected={isSelected}
                    status={
                      <>
                        {!!item.credentials?.length && (
                          <p className="mt-1 text-xs text-muted-foreground">
                            {item.credentials.some(
                              (env) =>
                                env.required &&
                                !env.present &&
                                !credentialInputs[item.name]?.[env.key],
                            )
                              ? "Needs input"
                              : item.credentials.some(
                                    (env) =>
                                      !env.present &&
                                      !credentialInputs[item.name]?.[env.key],
                                  )
                                ? "Optional"
                                : item.credentials.some(
                                      (env) =>
                                        secretChoices[item.name]?.[env.key] ?? env.secret,
                                    )
                                  ? "Found, goes to keychain"
                                  : "Found"}
                          </p>
                        )}
                        {busy && isSelected && (
                          <p role="status" className="mt-1 flex gap-2 text-xs">
                            <Loader2 className="size-3 animate-spin" />
                            Checking {item.name}...
                          </p>
                        )}
                      </>
                    }
                  />
                </button>
                {!!item.credentials?.length && (
                  <details className="px-3 pb-3 text-xs">
                    <summary className="cursor-pointer">
                      {item.name} credentials and settings
                    </summary>
                    <div className="flex flex-col gap-2 pt-2">
                      {item.credentials.map((env) => (
                        <div key={env.key} className="flex items-center gap-2">
                          <label className="flex items-center gap-2">
                            <input
                              type="checkbox"
                              disabled={
                                busy || !!item.unsupported || env.key.startsWith("__")
                              }
                              checked={secretChoices[item.name]?.[env.key] ?? env.secret}
                              onChange={(e) =>
                                setSecretChoices((previous) => ({
                                  ...previous,
                                  [item.name]: {
                                    ...previous[item.name],
                                    [env.key]: e.target.checked,
                                  },
                                }))
                              }
                            />
                            Keep {env.key} in keychain{" "}
                            {!env.required && (
                              <span className="text-muted-foreground">Optional</span>
                            )}
                            <span className="ml-auto text-muted-foreground">
                              {env.present
                                ? "Found"
                                : credentialInputs[item.name]?.[env.key]
                                  ? "Ready for connection"
                                  : env.required
                                    ? "Missing"
                                    : ""}
                            </span>
                          </label>
                          {!env.present && (
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={busy}
                              onClick={() => {
                                setValue(credentialInputs[item.name]?.[env.key] ?? "");
                                setEditing({ name: item.name, key: env.key });
                              }}
                            >
                              Enter value
                            </Button>
                          )}
                        </div>
                      ))}
                    </div>
                  </details>
                )}
                {item.unsupported && (
                  <p className="px-3 pb-2 text-xs text-warning">
                    Unsupported: {item.unsupported}
                  </p>
                )}
              </div>
            );
          })}
        </div>
      </div>
      {editing && (
        <Dialog
          open
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>{editing.name} credentials</DialogTitle>
            </DialogHeader>
            <p className="text-sm text-muted-foreground">
              This value is used only for the selected server. It is saved only if
              connection succeeds.
            </p>
            <label className="flex flex-col gap-2 text-sm">
              {editing.key}
              <input
                type="password"
                autoComplete="off"
                className="rounded-md border bg-background px-3 py-2"
                value={value}
                onChange={(e) => setValue(e.target.value)}
              />
            </label>
            <DialogFooter>
              <Button variant="ghost" onClick={() => setEditing(null)}>
                Cancel
              </Button>
              <Button
                disabled={!value.trim()}
                onClick={() => {
                  setCredentialInputs((previous) => ({
                    ...previous,
                    [editing.name]: { ...previous[editing.name], [editing.key]: value },
                  }));
                  setValue("");
                  setEditing(null);
                }}
              >
                Use for connection
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      )}
      {missingRequired.map((reason) => (
        <p key={reason} className="text-sm text-warning">
          {reason}
        </p>
      ))}
      <DialogFooter className="justify-between">
        <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={busy}>
          Cancel
        </Button>
        <Button
          onClick={() => {
            if (items.some((item) => item.credentials?.length))
              onConfirm(
                Array.from(selected),
                secretChoices,
                ...(Object.keys(credentialInputs).length
                  ? ([credentialInputs] as const)
                  : ([] as const)),
              );
            else onConfirm(Array.from(selected));
          }}
          disabled={
            busy || missingRequired.length > 0 || (!allowEmpty && selectedCount === 0)
          }
        >
          <Check className="size-4" />
          {confirmLabel ??
            (selectedCount === 0
              ? "Select a server"
              : `Import ${selectedCount} server${selectedCount === 1 ? "" : "s"}`)}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

/** One reviewable server: name, what it runs, and the relevant safety flags. */
export function ImportRow({
  item,
  selected,
  status,
}: {
  item: ImportItem;
  selected?: boolean;
  status?: ReactNode;
}) {
  const runs =
    item.command != null ? [item.command, ...item.args].join(" ") : (item.url ?? "");
  const shell = runsShell(item.command, item.args);
  const privateHost = isPrivateHostUrl(item.url);
  return (
    <div className="rounded-md border px-3 py-2">
      <div className="flex items-center gap-2">
        {selected !== undefined && (
          <span
            aria-hidden="true"
            className={`size-3 rounded-sm border ${
              selected ? "border-success bg-success" : "border-muted-foreground"
            }`}
          />
        )}
        <span className="truncate text-sm font-medium">{item.name}</span>
        {
          <span className="ml-auto shrink-0 text-xs text-muted-foreground">
            {item.updates?.length
              ? "Updates existing server"
              : item.isNew
                ? "New"
                : "In Toolport"}
          </span>
        }
      </div>
      {item.referenceReview?.map((line) => (
        <p key={line} className="mt-1 text-xs break-all">
          {line}
        </p>
      ))}
      {!!item.updates?.length && (
        <p className="mt-1 text-xs text-muted-foreground">
          Changes: {item.updates.join(", ")}
        </p>
      )}
      {runs && (
        <p
          title={runs}
          aria-label={runs}
          className="mt-1 flex min-w-0 font-mono text-xs text-muted-foreground"
        >
          {runs.length > 72 ? (
            <>
              <span className="min-w-0 truncate">{runs.slice(0, -28)}</span>
              <span className="shrink-0">{runs.slice(-28)}</span>
            </>
          ) : (
            runs
          )}
        </p>
      )}
      {!!item.envKeys?.length && !item.credentials?.length && (
        <p className="mt-1 text-xs text-warning">
          Credentials: {item.envKeys.join(", ")}. Review their status before connecting.
        </p>
      )}
      {shell && (
        <p className="mt-1.5 flex items-center gap-1.5 text-xs text-warning">
          <ShieldAlert className="size-3.5 shrink-0" />
          Runs a shell command. Only import setups you trust.
        </p>
      )}
      {privateHost && (
        <p className="mt-1.5 flex items-center gap-1.5 text-xs text-warning">
          <ShieldAlert className="size-3.5 shrink-0" />
          Connects to a private or internal address. Only import setups you trust.
        </p>
      )}
      {status}
    </div>
  );
}
