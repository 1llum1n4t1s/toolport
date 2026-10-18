import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("animate-pulse rounded-md bg-muted", className)}
      {...props}
    />
  );
}

/** Loads that finish within this show nothing, so a fast load never flashes (spec §11). */
const LOADING_DELAY_MS = 300;

function useDelayed(ms: number): boolean {
  const [shown, setShown] = useState(ms <= 0);
  useEffect(() => {
    if (ms <= 0) return;
    const timer = window.setTimeout(() => setShown(true), ms);
    return () => window.clearTimeout(timer);
  }, [ms]);
  return shown;
}

/**
 * The one loading state (spec §11): three skeleton rows in a row group, shown only after
 * `LOADING_DELAY_MS`. Spinners belong only inside a button that is working.
 */
function SkeletonRows({
  rows = 3,
  label,
  delayMs = LOADING_DELAY_MS,
  className,
}: {
  rows?: number;
  label: string;
  delayMs?: number;
  className?: string;
}) {
  const shown = useDelayed(delayMs);
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={label}
      data-slot="skeleton-rows"
      className={cn(
        "divide-y overflow-hidden rounded-xl border bg-card",
        !shown && "invisible",
        className,
      )}
    >
      {Array.from({ length: rows }).map((_, i) => (
        <div key={i} className="flex h-14 items-center gap-3 px-4">
          <Skeleton className="size-7 shrink-0" />
          <div className="flex min-w-0 flex-1 flex-col gap-2">
            <Skeleton className="h-3 w-1/3" />
            <Skeleton className="h-2.5 w-1/2" />
          </div>
        </div>
      ))}
    </div>
  );
}

export { Skeleton, SkeletonRows };
