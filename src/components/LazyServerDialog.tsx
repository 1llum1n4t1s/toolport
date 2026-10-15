import { lazy, Suspense, type ComponentProps } from "react";

type ServerDialogProps = ComponentProps<
  typeof import("@/components/ServerDialog").ServerDialog
>;

const ServerDialog = lazy(() =>
  import("@/components/ServerDialog").then((module) => ({
    default: module.ServerDialog,
  })),
);

// Keep editing and snippet parsing out of the initial Servers bundle.
export function LazyServerDialog(props: ServerDialogProps) {
  return (
    <Suspense fallback={props.trigger ?? null}>
      <ServerDialog {...props} />
    </Suspense>
  );
}
