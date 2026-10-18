import { createContext, useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { ArrowLeft } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/** Where a page's own header actions render; App provides it around the views.
 * `undefined` means no page header at all (a view rendered on its own). */
const ActionsSlot = createContext<HTMLElement | null | undefined>(undefined);

export function PageHeaderActionsSlot({
  value,
  children,
}: {
  value: HTMLElement | null;
  children: ReactNode;
}) {
  return <ActionsSlot.Provider value={value}>{children}</ActionsSlot.Provider>;
}

/**
 * Renders a view's actions into the page header, ahead of App's overflow menu
 * and refresh button, so actions stay in the header while their state stays
 * in the view that owns it. Outside a page header they render in place.
 */
export function PageHeaderActions({ children }: { children: ReactNode }) {
  const slot = useContext(ActionsSlot);
  if (slot === undefined)
    return <div className="flex flex-wrap items-center gap-2">{children}</div>;
  return slot ? createPortal(children, slot) : null;
}

export interface PageHeaderBack {
  label: string;
  ariaLabel?: string;
  onClick: () => void;
}

interface Props {
  /** The sidebar label, word for word. */
  title: ReactNode;
  /** One plain line in secondary text. */
  subtitle?: ReactNode;
  /** A back button for sub-pages, such as Catalog under Servers. */
  back?: PageHeaderBack;
  /** Optional logo or icon tile after the back button. */
  leading?: ReactNode;
  /** Header actions in order: search, secondary actions, overflow, refresh. */
  actions?: ReactNode;
  /** Receives the element that `PageHeaderActions` portals into. */
  actionsSlotRef?: (element: HTMLElement | null) => void;
  className?: string;
}

/**
 * The one header every page uses. The title keeps its natural width; when it
 * and the actions do not fit on one line, the actions wrap below the title
 * instead of squeezing it.
 */
export function PageHeader({
  title,
  subtitle,
  back,
  leading,
  actions,
  actionsSlotRef,
  className,
}: Props) {
  return (
    <header
      className={cn(
        "app-header flex shrink-0 flex-wrap items-center gap-x-4 gap-y-3 border-b bg-sidebar px-6 py-4",
        className,
      )}
    >
      <div className="flex max-w-full min-w-0 grow items-center gap-3">
        {back && (
          <>
            <Button
              variant="ghost"
              size="sm"
              className="-ml-2 shrink-0 text-muted-foreground"
              onClick={back.onClick}
              aria-label={back.ariaLabel ?? back.label}
            >
              <ArrowLeft className="size-4" /> {back.label}
            </Button>
            <div className="h-7 w-px shrink-0 bg-border" aria-hidden="true" />
          </>
        )}
        {leading}
        <div className="min-w-0">
          <h1 className="truncate text-[22px] font-semibold tracking-tight">{title}</h1>
          {subtitle && (
            <p className="truncate text-meta text-muted-foreground">{subtitle}</p>
          )}
        </div>
      </div>
      <div className="page-header-actions flex min-w-0 flex-wrap items-center gap-2">
        <div ref={actionsSlotRef} className="contents" />
        {actions}
      </div>
    </header>
  );
}
