import type { QuarantinedTool } from "@/lib/api";

export function QuarantineDefinition({ tool }: { tool: QuarantinedTool }) {
  if (!tool.definition) {
    return (
      <p className="mt-1 text-xs text-muted-foreground">
        No saved definition. Approving may require one more review of the current
        definition.
      </p>
    );
  }
  return (
    <details className="mt-1 min-w-0 text-xs">
      <summary className="cursor-pointer text-muted-foreground">
        Captured definition
      </summary>
      <pre className="mt-1 max-h-64 overflow-auto whitespace-pre-wrap break-words rounded border border-border bg-background p-2 font-mono text-[11px]">
        {JSON.stringify(tool.definition, null, 2)}
      </pre>
    </details>
  );
}
