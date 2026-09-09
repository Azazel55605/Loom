import * as React from "react";

import { BackHandlerStack } from "@loom/ui-kit/lib/back-handler-stack";

/** Preserve Radix's controlled/uncontrolled Root contract while registering it. */
export function useBackAwareOpenState(
  controlledOpen: boolean | undefined,
  defaultOpen: boolean | undefined,
  onOpenChange: ((open: boolean) => void) | undefined,
): readonly [boolean, (open: boolean) => void] {
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(
    defaultOpen ?? false,
  );
  const isControlled = controlledOpen !== undefined;
  const open = isControlled ? controlledOpen : uncontrolledOpen;

  const setOpen = React.useCallback(
    (nextOpen: boolean) => {
      if (!isControlled) setUncontrolledOpen(nextOpen);
      onOpenChange?.(nextOpen);
    },
    [isControlled, onOpenChange],
  );
  const setOpenRef = React.useRef(setOpen);
  setOpenRef.current = setOpen;

  React.useEffect(() => {
    if (!open) return undefined;
    return BackHandlerStack.register(() => setOpenRef.current(false));
  }, [open]);

  return [open, setOpen] as const;
}
