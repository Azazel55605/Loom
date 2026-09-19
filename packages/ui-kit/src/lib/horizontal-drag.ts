/**
 * Elements that own the horizontal part of a gesture starting inside them.
 *
 * A page-level swipe detector — the kiosk shell's dashboard rotation is the
 * one in the tree — cannot tell a deliberate page swipe from a `Slider` drag by
 * looking at the movement alone: both are a finger travelling sideways, and
 * both travel far further than any sane swipe threshold. The only honest
 * signal is *where the finger went down*, so the detector asks that question
 * here instead of guessing from distance.
 *
 * Radix's slider settles its drag on pointer events and never touches touch
 * events, so nothing inside it stops a `touchstart` from reaching an outer
 * listener. There is no propagation for the outer listener to respect; it has
 * to look at the target itself.
 *
 * The selector covers, in order: anything that opts in explicitly via the
 * attribute (our `Slider`), any ARIA slider thumb including ones we do not
 * render ourselves, react-grid-layout's resize grips, and the card drag handle.
 * The last two only exist while a dashboard is in layout-edit mode, which a
 * kiosk account reaches only when it owns or can edit the dashboard on screen —
 * unusual, but not impossible, and a resize that turns into a page swipe would
 * be the same bug.
 */

/** Marks an element whose own handling of a sideways drag is the only one. */
export const HORIZONTAL_DRAG_ATTRIBUTE = "data-horizontal-drag";

/** The only drag surface on a placement card while editing a layout. */
export const DRAG_HANDLE_CLASS = "loom-drag-handle";

const HORIZONTAL_DRAG_SELECTOR = [
  `[${HORIZONTAL_DRAG_ATTRIBUTE}]`,
  '[role="slider"]',
  ".react-resizable-handle",
  `.${DRAG_HANDLE_CLASS}`,
].join(", ");

/** Whether `target` sits inside an element that handles its own sideways drag. */
export function ownsHorizontalDrag(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return target.closest(HORIZONTAL_DRAG_SELECTOR) !== null;
}
