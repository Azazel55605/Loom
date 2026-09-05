import * as React from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";

import { ApiError, type DashboardPlacement } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Running one placement's stored click behaviour.
 *
 * Shared by the static button tile and the connector tile, because a click on
 * either is the same request against the same endpoint — the tile only decides
 * what is drawn around it. Keeping it here is what stops the two tiles growing
 * two different sets of error wording for the same three failures.
 *
 * ## The two navigate failures stay two failures
 *
 * The backend answers 403 when the clicker cannot open the target and 404 when
 * the target has been deleted, and it does that deliberately — see
 * `docs/adr/0035-placement-actions-and-hidden-dashboards.md`. Collapsing them
 * into one "something went wrong" here would throw away the whole point:
 * "ask the owner for access" and "this link is dead" are different things for a
 * user to do, and only one of them is worth trying.
 *
 * Nothing about a disruptive `connectorAction` needs handling here. The pending
 * operation raised by the backend arrives on the existing status socket like
 * any other, so the overlay a restart shows is the one it has always shown.
 */
export function usePlacementClick({
  dashboardId,
  placement,
  onNavigateDashboard,
}: {
  dashboardId: string;
  placement: DashboardPlacement;
  onNavigateDashboard?: (dashboardId: string) => void;
}) {
  const api = useApiClient();
  const action = placement.placementAction;
  const label = placement.label ?? placement.connector?.name ?? "This tile";

  const click = useMutation({
    mutationFn: () => api.clickDashboardPlacement(dashboardId, placement.id),
    onSuccess: (result) => {
      if ("targetDashboardId" in result) {
        onNavigateDashboard?.(result.targetDashboardId);
        return;
      }
      // A 200 with `success: false` means the service was reached and
      // declined — a different thing from the request failing, and reported
      // the same way every other action on a dashboard reports it.
      if (result.success) {
        toast.success(label, { description: result.message });
      } else {
        toast.warning(`${label} declined`, { description: result.message });
      }
    },
    onError: (error) => {
      if (error instanceof ApiError && action?.type === "navigate") {
        if (error.isForbidden) {
          toast.error("You don't have permission to use this", {
            description:
              "This tile points at a dashboard that has not been shared with you.",
          });
          return;
        }
        if (error.status === 404) {
          toast.error("Not available", {
            description: "The dashboard this tile pointed at no longer exists.",
          });
          return;
        }
      }
      toast.error(`${label} failed`, { description: describeConnectorError(error) });
    },
  });

  /**
   * Whether this tile should behave as a button at all.
   *
   * A navigate tile with nowhere to navigate is not clickable, rather than
   * clickable and silently inert.
   */
  const clickable =
    action !== null &&
    (action.type !== "navigate" || onNavigateDashboard !== undefined);

  return { clickable, pending: click.isPending, run: () => click.mutate() };
}

/**
 * Anything that handles its own click, and must therefore not also trigger the
 * tile's.
 *
 * A connector tile is full of these — action widgets, the expand button, the
 * updates summary — and pressing one is a statement about that control, never
 * about the tile around it.
 */
const INTERACTIVE_SELECTOR =
  'a, button, input, select, textarea, [role="button"], [role="checkbox"], [role="switch"], [role="slider"], [role="radiogroup"], [contenteditable="true"]';

/**
 * The interactive shell around a clickable tile.
 *
 * ## Why not a `<button>` wrapping the card
 *
 * Because a connector tile is allowed to be clickable *and* still carry its own
 * buttons — that is the entire point of composing the action onto an ordinary
 * placement. Nesting those inside a `<button>` is invalid HTML, and browsers
 * resolve it by doing something arbitrary to both. So the shell is a `div` with
 * `role="button"`, `tabIndex`, and an explicit Enter/Space handler, which is the
 * same semantics assembled by hand.
 *
 * Clicks originating inside any interactive descendant are ignored, so pressing
 * a toggle on a clickable tile toggles the toggle and does not also navigate.
 * Keyboard activation is guarded the same way: focus inside a widget means the
 * key belongs to the widget.
 *
 * One code path for the static tile too, which has no inner controls and would
 * be equally correct as a real `<button>`. Two paths that must stay in step is
 * how the two tiles would end up behaving differently for the same click.
 *
 * ## Not while editing
 *
 * `active` is false in layout-edit mode, and the shell then renders its child
 * untouched — no role, no hover, no handler. This matches how the action
 * widgets already behave: a press meant to grab a card must never be able to
 * restart a service or change the page out from under a drag.
 */
export function PlacementClickSurface({
  active,
  pending,
  label,
  onActivate,
  children,
}: {
  active: boolean;
  pending: boolean;
  /** Accessible name, since the card's own content is decorative to the
   *  control. */
  label: string;
  onActivate: () => void;
  children: React.ReactNode;
}) {
  if (!active) return <>{children}</>;

  const fromInsideAControl = (target: EventTarget | null): boolean =>
    target instanceof Element && target.closest(INTERACTIVE_SELECTOR) !== null;

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={label}
      aria-busy={pending || undefined}
      onClick={(event) => {
        if (fromInsideAControl(event.target)) return;
        onActivate();
      }}
      onKeyDown={(event) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        if (fromInsideAControl(event.target)) return;
        // Space scrolls the page by default, which on a control that is a
        // button in all but tag name would be the wrong answer entirely.
        event.preventDefault();
        onActivate();
      }}
      className={cn(
        "group block h-full w-full min-w-0 cursor-pointer",
        "rounded-[var(--radius)] transition-[opacity,box-shadow]",
        "hover:shadow-md focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
        pending && "opacity-70",
      )}
    >
      {children}
    </div>
  );
}
