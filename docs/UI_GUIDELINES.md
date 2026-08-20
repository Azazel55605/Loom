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

> **Status: implemented in web-frontend and Desktop.** All three now have real controls,
> under Settings → Appearance, driven by `AccentThemeProvider`, alongside a
> light/dark/system palette choice. What follows
> describes working behavior, not intent. Mobile still carries the older
> accent-only provider and gains the rest when it consumes `@loom/ui-kit`.

### Persistence is per device

Preferences live in `localStorage` under `loom-accent-color`, `loom-blur-level`,
`loom-reduce-motion`, and `loom-theme`. They therefore **do not follow a
user to another browser or machine**, and the Appearance panel says so on screen
rather than letting someone assume otherwise.

That is the current model, not a settled one. Syncing preferences through the
backend — columns on `users`, or a preferences table, plus a contract for
reading and writing them — is a plausible enhancement and explicitly **not yet
decided**. It is worth noting what the local model buys in the meantime: the
settings apply before the first paint, with no request and no authenticated
round trip, so there is no flash of the wrong accent on load and they work on a
screen reached before any session exists. A synced model would need to keep that
property rather than trade it away, most likely by treating the local value as a
cache and the server as the source of truth.

The choice also has to survive `localStorage` being unavailable — private
browsing and strict cookie settings make it throw on access. Reads and writes
are best-effort: losing persistence costs the preference across reloads, never
the ability to use the app.

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
surfaces with solid ones.

In practice the setting is a **level**, not a switch, and each level is one
root class of which exactly one is applied:

| Level | Class | Effect |
| --- | --- | --- |
| `off` | `.reduce-transparency` | Solid surfaces, no blur. Cheapest to render. |
| `standard` | `.force-transparency` | Elevated surfaces frosted — dialogs, popovers, the header. |
| `extra` | `.blur-extra` | Heavier blur, secondary surfaces joined in, over an accent-derived wash. |

Three classes rather than a boolean because `prefers-reduced-transparency` is
only a *default* here: an explicit choice overrides it in either direction,
which is what "let the user override it explicitly" requires.

Two tokens carry it, not one. `--surface-alpha` is for elevated surfaces and
`--panel-alpha` for secondary inset ones (tab bars, segmented controls), so
turning the effect up does not make a small inset panel as translucent as a
dialog. Secondary surfaces use the `.surface-panel` class, which is solid at
every level except `extra` — no runtime class toggling needed, because the
token does the work.

### Blur needs something behind it

The mistake worth recording, because it is invisible until you look for it:
`backdrop-filter` samples what is *behind* an element, so **over a flat
background it produces no visible change at all**. Frosted glass with nothing
behind it looks exactly like plain colour. This is why the `extra` level also
paints a soft, slowly varying wash — three low-alpha radial gradients derived
from `--accent`, so the effect follows the user's colour rather than
introducing a second one.

Two corollaries, both of which cost a round of debugging here:

- The wash must be **positioned inside the viewport**. Centring the gradients
  on the corners put them off-screen and behind the sticky header, which is a
  wash nobody can see doing none of the work it exists for.
- Any opaque full-height element above it hides it completely. The page ground
  carries an `.app-canvas` class and goes transparent at the `extra` level for
  exactly this reason. This serves two distinct needs and both matter:

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

The in-app "reduce motion" switch layers on top of that signal via a
`.reduce-motion` class; it never competes with it. **The OS setting is a floor,
not a default.** A user may force motion off when the OS has not asked for it,
but nothing in the app may force motion back *on* against an OS request —
reduced motion is an accessibility need for some people, and an app-level
preference is not entitled to override it. The switch reflects this by showing
as on-and-disabled when the OS is asking, rather than appearing to be off or
silently doing nothing when clicked.

### Light and dark

A fourth user-facing choice, alongside the three axes above: **Light**, **Dark**,
or **System**. `System` follows `prefers-color-scheme` and is the default,
because the OS already carries an answer and opening an app in blazing white at
night is a bad first impression nobody asked for.

Dark is a token swap, not a separate stylesheet — the `.dark` class redefines
the same custom properties, so any component built from the tokens inverts
without knowing dark mode exists. Two things do need explicit attention:

- **`color-scheme` is set on the root** alongside the class. Without it the
  browser keeps painting its own surfaces — scrollbars, the overscroll
  canvas — from the light palette.
- **Translucency is not portable between palettes.** The alpha that reads as
  frosted over white reads as washed out over near-black, so the dark palette
  carries its own `--surface-alpha`. Those overrides have to match *two*
  classes (`.dark.force-transparency`) rather than one: `.dark` appears earlier
  in the sheet than the level classes, so a single-class rule would win on
  source order and flatten the dark value back to the light one.

Status colours are deliberately **not** re-derived per palette beyond a
lightness lift — the hues stay put, so "healthy" reads as the same colour in
both.

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
