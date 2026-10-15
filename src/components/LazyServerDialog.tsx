import {
  cloneElement,
  isValidElement,
  lazy,
  Suspense,
  useState,
  type ComponentProps,
  type MouseEventHandler,
} from "react";

type ServerDialogProps = ComponentProps<
  typeof import("@/components/ServerDialog").ServerDialog
>;

const ServerDialog = lazy(() =>
  import("@/components/ServerDialog").then((module) => ({
    default: module.ServerDialog,
  })),
);

// Remember clicks made while the editing chunk is still loading.
export function LazyServerDialog(props: ServerDialogProps) {
  const [requested, setRequested] = useState(false);
  const original = isValidElement<{ onClick?: MouseEventHandler }>(props.trigger)
    ? props.trigger
    : null;
  const trigger = original
    ? cloneElement(original, {
        onClick: (event) => {
          original.props.onClick?.(event);
          if (!event.defaultPrevented) setRequested(true);
        },
      })
    : (props.trigger ?? null);
  return (
    <Suspense fallback={trigger}>
      <ServerDialog {...props} autoOpen={props.autoOpen || requested} />
    </Suspense>
  );
}
