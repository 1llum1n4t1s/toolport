import { useState } from "react";
import { getQuarantineDefinition, type QuarantinedTool } from "@/lib/api";

export function QuarantineDefinition({ tool }: { tool: QuarantinedTool }) {
  return (
    <Definition key={`${tool.profile}/${tool.tool}/${tool.record_id}`} tool={tool} />
  );
}

function Definition({ tool }: { tool: QuarantinedTool }) {
  const [expanded, setExpanded] = useState(false);
  const [definition, setDefinition] = useState(tool.definition);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (!(tool.has_definition ?? Boolean(tool.definition))) {
    return (
      <p className="mt-1 text-xs text-muted-foreground">
        No saved definition. Approving captures the current definition for one more
        review. The tool stays blocked until then.
      </p>
    );
  }
  return (
    <details
      className="mt-1 min-w-0 text-xs"
      onToggle={(event) => {
        const open = event.currentTarget.open;
        setExpanded(open);
        if (open && !definition && !loading) {
          setLoading(true);
          setError(null);
          void getQuarantineDefinition(tool.profile, tool.tool, tool.record_id)
            .then(setDefinition)
            .catch((err: unknown) => setError(String(err)))
            .finally(() => setLoading(false));
        }
      }}
    >
      <summary className="cursor-pointer text-muted-foreground">
        Captured definition
      </summary>
      {expanded && (
        <>
          {loading && <p className="mt-1 text-muted-foreground">Loading definition…</p>}
          {error && (
            <p role="alert" className="mt-1 text-destructive">
              {error}
            </p>
          )}
          {definition && (
            <pre className="mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-background p-2 font-mono text-[11px]">
              {JSON.stringify(definition, null, 2)}
            </pre>
          )}
        </>
      )}
    </details>
  );
}
