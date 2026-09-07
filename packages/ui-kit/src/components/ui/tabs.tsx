import * as React from "react";
import * as TabsPrimitive from "@radix-ui/react-tabs";

import { cn } from "@loom/ui-kit/lib/utils";

const Tabs = TabsPrimitive.Root;

type TabIndicatorGeometry = {
  left: number;
  width: number;
  visible: boolean;
};

const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, children, ...props }, forwardedRef) => {
  const listRef = React.useRef<React.ElementRef<typeof TabsPrimitive.List>>(null);
  const [indicator, setIndicator] = React.useState<TabIndicatorGeometry>({
    left: 0,
    width: 0,
    visible: false,
  });

  React.useImperativeHandle(forwardedRef, () => listRef.current!, []);

  const measureIndicator = React.useCallback(() => {
    const active = listRef.current?.querySelector<HTMLElement>(
      '[role="tab"][data-state="active"]',
    );
    if (active === undefined || active === null) {
      setIndicator((current) =>
        current.visible ? { ...current, visible: false } : current,
      );
      return;
    }

    const next = {
      left: active.offsetLeft,
      width: active.offsetWidth,
      visible: true,
    };
    setIndicator((current) =>
      current.left === next.left &&
      current.width === next.width &&
      current.visible === next.visible
        ? current
        : next,
    );
  }, []);

  React.useLayoutEffect(() => {
    const list = listRef.current;
    if (list === null) return;

    let animationFrame = 0;
    const scheduleMeasurement = () => {
      window.cancelAnimationFrame(animationFrame);
      animationFrame = window.requestAnimationFrame(measureIndicator);
    };
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(scheduleMeasurement);
    const observeSizes = () => {
      resizeObserver?.disconnect();
      resizeObserver?.observe(list);
      for (const trigger of list.querySelectorAll<HTMLElement>('[role="tab"]')) {
        resizeObserver?.observe(trigger);
      }
    };
    const mutationObserver = new MutationObserver((mutations) => {
      if (mutations.some((mutation) => mutation.type === "childList")) observeSizes();
      scheduleMeasurement();
    });

    measureIndicator();
    observeSizes();
    mutationObserver.observe(list, {
      attributes: true,
      attributeFilter: ["data-state"],
      childList: true,
      subtree: true,
    });
    window.addEventListener("resize", scheduleMeasurement);

    return () => {
      window.cancelAnimationFrame(animationFrame);
      window.removeEventListener("resize", scheduleMeasurement);
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
    };
  }, [measureIndicator]);

  return (
    <TabsPrimitive.List
      ref={listRef}
      className={cn(
        // `surface-panel` rather than `bg-muted`: identical while blur is off or
        // standard, and frosted at the "extra" level without a second class.
        "surface-panel relative isolate inline-flex h-auto min-h-[var(--touch-target-size)] items-center justify-center rounded-lg p-1 text-muted-foreground",
        className,
      )}
      {...props}
    >
      <span
        aria-hidden="true"
        className="loom-tabs-indicator pointer-events-none absolute inset-y-1 left-0 z-0 rounded-md bg-card shadow"
        data-visible={indicator.visible}
        style={{
          width: indicator.width,
          transform: `translate3d(${indicator.left}px, 0, 0)`,
        }}
      />
      {children}
    </TabsPrimitive.List>
  );
});
TabsList.displayName = TabsPrimitive.List.displayName;

const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      "relative z-10 inline-flex min-h-[var(--touch-target-size)] min-w-[var(--touch-target-size)] items-center justify-center gap-2 whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 data-[state=active]:text-foreground [&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
      className,
    )}
    {...props}
  />
));
TabsTrigger.displayName = TabsPrimitive.Trigger.displayName;

const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn(
      "mt-4 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
      className,
    )}
    {...props}
  />
));
TabsContent.displayName = TabsPrimitive.Content.displayName;

export { Tabs, TabsList, TabsTrigger, TabsContent };
