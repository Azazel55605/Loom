# UI Guidelines

> **Conventions and philosophy.** This document changes rarely — it describes
> *how* we build UI, not *what* we have built. The living inventory of actual
> components is [`COMPONENTS.md`](./COMPONENTS.md), which is updated with every
> component change.
>
> Nothing under `apps/` is scaffolded yet, so this is the standard the clients
> will be built to rather than a description of existing code.

## Philosophy

Loom's UI should feel **fluid, modern, and highly customizable**. Fluid meaning
motion and layout respond to the user rather than snapping between states;
modern meaning it looks like software from this decade; customizable meaning the
things people actually want to change — accent color, blur, motion — are
user-adjustable at runtime, not constants a developer picked.

The primary component source is **[shadcn/ui](https://ui.shadcn.com/)**: Radix
UI primitives for behavior and accessibility, Tailwind CSS for styling, and
[class-variance-authority](https://cva.style/) (CVA) for variant management.
This is not merely a library we install — it is the construction pattern
everything else in the UI imitates.

### No native UI, anywhere

**No native or default OS/browser UI components in any client** — Web/frontend,
Desktop, or Mobile. Every interactive element is themed and behaves identically
across platforms.

This is a hard rule because Loom ships the same interface through a browser, a
Tauri desktop window, and a Tauri mobile app. A native `<select>` renders as a
Windows combobox, a macOS popup, a GTK menu, and a full-screen iOS wheel — four
different products from one codebase. Themed equivalents keep one product, and
keep the accent color and blur settings meaningful everywhere rather than
applying to some controls and silently skipping others.

## Customization axes

All three are **user-adjustable at runtime**, not hardcoded values. Treat them
as inputs to the design, not decisions inside it. Any new component must respond
to all three without extra wiring — if a component only looks right at one
accent color or one blur level, it is not finished.

### Accent color

A single **HSL-based CSS custom property** is the source of truth, and every
derived shade and interaction state (hover, active, focus ring, disabled,
subtle backgrounds) is computed from it. This follows shadcn's existing theming
convention: channel values stored bare so they can be recombined with varying
alpha and lightness.

```css
:root {
  --accent: 217 91% 60%;        /* hue saturation lightness — no hsl() wrapper */
}
.thing {
  background: hsl(var(--accent));
  border-color: hsl(var(--accent) / 0.4);   /* alpha without a second variable */
}
```

Never hardcode a hex color for anything accent-derived, and never introduce a
second "accent-ish" variable — one user-facing choice must drive the whole
palette, or changing it will leave stray mismatched elements behind.

> Note: shadcn's newer Tailwind v4 setup expresses these tokens in **OKLCH**
> rather than HSL. HSL is our convention for now, per this document; if the
> clients are scaffolded on Tailwind v4, revisit this choice deliberately and
> record it rather than mixing both conventions.

### Blur

Elevated and glass surfaces — dialogs, popovers, sheets, command palettes,
sticky headers — use `backdrop-blur`. **Intensity is adjustable**, driven by a
token rather than per-component Tailwind classes, so the whole interface moves
together.

There must be a **"reduced transparency" fallback** that replaces translucent
surfaces with solid ones. This serves two distinct needs and both matter:

- *Accessibility* — text over a blurred background is harder to read, and some
  users need opaque surfaces to use the interface at all.
- *Performance* — `backdrop-filter` is expensive, particularly on mobile GPUs
  and on large surfaces. On weaker hardware this is the difference between
  fluid and janky.

Honor the OS-level signal (`prefers-reduced-transparency`) as the default, and
let the user override it explicitly. A component must remain legible with blur
fully disabled — meaning contrast comes from the solid fallback color, never
from the blur itself.

### Animation

Motion should be consistent and purposeful: it explains what changed and where
something came from. Decoration that doesn't communicate is noise.

- **Default to `tailwindcss-animate`** for shadcn-native components. It covers
  enter/exit transitions driven by Radix's data-state attributes, which is the
  large majority of what the UI needs, at no runtime cost.
- **Escalate to [Framer Motion](https://motion.dev/) only where genuinely
  needed** — shared-element transitions, gesture-driven interactions,
  physics-based or interruptible animation, orchestrated sequences. Reaching for
  it because it is more familiar is not a reason; it is a runtime dependency and
  a bundle cost, and mixing two motion systems in one component is worse than
  either alone.

**All motion must respect `prefers-reduced-motion`.** This means reduced, not
merely faster: replace movement and scaling with an opacity change or an instant
state swap. Nothing that conveys state may become invisible when motion is off —
if the only indication a dialog opened is that it slid in, the dialog is broken
for those users.

## Component sourcing rule

In strict priority order. Do not skip a step because a later one is more
interesting.

### a. Use the existing shadcn/ui component as-is

If shadcn has a component that fits, use it unmodified. This is the expected
outcome for most needs.

### b. Extend a shadcn component

If a component is close but not exact, **add a variant via CVA** to the existing
component. Extend the `variants` map — do not fork the file, do not wrap it in a
bespoke component that reimplements its behavior, and do not override its styles
from the outside with competing classes.

### c. Build a new component from scratch

Only if nothing fits. A new component **must** follow shadcn's construction
pattern, so the codebase reads as one system:

- **A Radix primitive underneath** for any interactive or accessible behavior —
  focus management, keyboard navigation, ARIA wiring, dismissal, portalling.
  Hand-rolled versions of these are where accessibility bugs live. If Radix has
  a primitive for it, that primitive is the foundation.
- **CVA for variant management** — no ad-hoc conditional class strings.
- **The same design token set** — accent, blur, radius, spacing, motion. No new
  one-off color or timing values.
- **The same file and folder convention** as shadcn's own components:
  co-located in the UI directory, typed props extending the underlying element's
  props, `forwardRef` where a ref should reach the DOM node, and the shared
  `cn()` class merger so consumers can still pass `className`.

A component that meets this bar is indistinguishable from a shadcn component in
use. That is the target.

### d. Never use native form controls

No native `<select>`, `<dialog>`, browser-default checkbox, radio, date picker,
file input, tooltip, or context menu. **Always the themed equivalent** — Radix
Select, Dialog, Checkbox, and so on.

The point is not that native controls are ugly. It is that they cannot be
themed consistently across the three platforms, they ignore the accent and blur
settings, and they make the interface look assembled rather than designed. The
one thing native controls do genuinely better — platform-standard accessibility
— is exactly what Radix primitives are built to reproduce, which is why they are
mandatory in step (c).

## Keeping the registry current

Adding or modifying a component means updating
[`COMPONENTS.md`](./COMPONENTS.md) **in the same change**. See
[`AGENT_INSTRUCTIONS.md`](./AGENT_INSTRUCTIONS.md).
