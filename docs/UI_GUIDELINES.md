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
things people actually want to change — palette, accent, blur, typography, density, and motion — are
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

### Responsive layout is shared behavior

Responsive behavior belongs in `packages/ui-kit`, alongside the component it
changes. Do not fork a shared component into Web, Desktop, and Mobile versions
just to rearrange it at a breakpoint. Platform wrappers inject navigation and
transport concerns; Tailwind breakpoints, responsive component props, and
pointer media queries adapt the same component to its available space.

Below the `md` breakpoint, persistent sidebars become focus-trapped off-canvas
`Sheet` drawers that start closed and close after navigation. Dashboard grids
reduce their column count as space contracts and stack cards full-width on
phone-sized containers, while preserving the canonical desktop geometry.
Controls that depend on pointer precision need at least a 44 by 44 pixel touch
target, and controls that cannot usefully shrink — such as settings tabs —
scroll horizontally inside their own region instead of widening the page.

## Customization axes

All appearance axes are **user-adjustable at runtime**, not hardcoded values.
Treat them as inputs to the design, not decisions inside it. Any new component
must respond without extra wiring — if it only looks right with one palette,
accent, font, blur level, animation level, or density, it is not finished.

> **Status: implemented in web-frontend, Desktop, and Mobile.** Every axis has a real control,
> under Settings → Appearance, driven by `AccentThemeProvider`, alongside a
> light/dark/system palette choice. What follows describes working behavior,
> not intent. All clients consume the same provider from `@loom/ui-kit`.

### Persistence is per device

Preferences live in `localStorage` under `loom-accent-color`, `loom-blur-level`,
`loom-animation-level`, `loom-density`, `loom-font-size`, `loom-font-family`, and
`loom-background-theme`. They therefore **do not follow a user to another browser
or machine**, and the Appearance panel says so on screen rather than letting
someone assume otherwise. Legacy motion, palette, blur, and two-tier density
keys are migrated without unexpectedly changing an existing visual choice.

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

### Display density

**Comfortable** is the default, **Compact** is the original tighter mode, and
**Dense** is a deliberately more aggressive tier for information-heavy screens.
The root `data-density` attribute selects CSS custom properties for stat-tile
font size, card padding, dashboard-grid spacing, table row/cell padding, and
table text size. Components consume those properties instead of branching on
density themselves. The former two-tier value named `dense` migrates once to
`compact`; the migration is versioned so choosing the new Dense remains stable.

Density must never shrink an interactive hit area. Buttons, icon buttons,
checkboxes, switches, segmented-control options, interactive table rows, and
dashboard resize handles retain a minimum 44 by 44 pixel target independently
of the density properties. If a visual glyph or control stays small, its
transparent hit area supplies the difference; table or card padding is never
relied on to make an action reachable. The shared `--touch-target-size: 44px`
uses physical CSS pixels rather than rem, so changing the root font scale cannot
silently shrink that guarantee either.

### Typography

Font size is a root rem scale with **Small**, **Medium** (default), and **Large**
choices. Core typography uses rem-derived Tailwind sizes; even intentionally tiny
badges use rem rather than pixels, so the scale reaches the entire application.

Font family has three locally bundled choices: the existing system-ui stack,
**IBM Plex Sans**, and **Nunito**. IBM Plex Sans was chosen instead of its
Condensed cut: it is visibly narrower in dense tables while remaining easier to
read at Loom's small status-label sizes. IBM Plex Sans and Nunito are distributed
under the OFL and loaded from the application bundle through Fontsource; no font
CDN or runtime network dependency is permitted.

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

The in-app selector has **Full**, **Reduced**, and **None** levels. Reduced
removes movement/scaling while preserving the state being communicated; None
disables transitions and animations entirely. `prefers-reduced-motion` remains
a floor, not a default: selecting Full can never override an OS request for
Reduced, while None may make it stricter. Stat-value flashes, dashboard edit
transitions, and initial tile staggering all obey the effective level.

Use the shared scale from `packages/ui-kit/src/lib/motion.ts` and its mirrored
CSS custom properties rather than adding local durations:

| Tier | Full | Reduced | Intended use |
| ---- | ---- | ------- | ------------ |
| Fast | 120 ms | 80 ms | press, focus, tooltip, and hover feedback |
| Base | 200 ms | 120 ms | tabs, collapsibles, dialogs, and ordinary layout changes |
| Slow | 320 ms | 160 ms | page/dashboard navigation and emphasized state changes |

Full uses the shared standard/emphasized curves and may use small amounts of
travel or scale. Reduced uses `ease-out`, retains short structural opacity
feedback, and removes travel, scale, hover lift, stagger, and indefinite pulse.
None makes every transition and animation instant. Navigation should therefore
be the most prominent motion, component transitions quieter, and
micro-interactions the subtlest. CSS-only work uses the `--motion-*` properties;
JavaScript that genuinely needs a timeout reads `useAnimationLevel()`. Do not
use JavaScript merely to reproduce a CSS `data-state` transition.

### Background themes

Five complete neutral token sets are available: **Midnight** (near-black AMOLED),
**Slate** (the default dark-navy palette), **Charcoal** (warm dark gray),
**Daylight** (clean white), and **Cream** (warm off-white). They redefine the
background, card, muted, border, input, foreground, and contrast tokens while
leaving `--accent` untouched, so every background combines with every accent.

Themes are token swaps, not separate stylesheets — components invert without
knowing which preset is active. Two things do need explicit attention:

- **`color-scheme` is set on the root** alongside the class. Without it the
  browser keeps painting its own surfaces — scrollbars, the overscroll
  canvas — from the light palette.
- **Translucency is not portable between palettes.** Each preset supplies its
  own standard/extra surface alpha and secondary-panel alpha. Blur therefore
  stays frosted and legible across true black, dark gray, white, and cream
  rather than applying one dark-tuned tint everywhere.

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

## Icons

Two sources, and the distinction is not cosmetic:

- **Generic icons** come from `lucide-react`, through the curated
  `GENERIC_ICONS` set in `packages/ui-kit/src/lib/generic-icons.ts`. They are
  line drawings that inherit `currentColor`, so they follow the accent and the
  theme like everything else. Use these unless you are identifying a specific
  product.
- **Brand icons** are SVGs vendored per connector type under
  `packages/ui-kit/src/assets/icons/brand/`. They are **not tinted** — a logo
  rendered in the user's accent colour is no longer that logo. The accent
  colours our surfaces; it does not recolour someone else's mark.

Both are reached through one component, `ConnectorIcon`, which resolves the
`brand:<key>` / `lucide:<name>` reference convention and falls back rather than
failing. Never import a brand SVG directly, and never add one without reading
[`THIRD_PARTY_ICONS.md`](./THIRD_PARTY_ICONS.md) first — the vendored set is
Apache-2.0 and carries attribution obligations that a casual copy-paste breaks.

Do not add icons to `GENERIC_ICONS` casually either. It is small on purpose: it
is the set a connector author can rely on and the set the icon picker offers,
and a catalog with a thousand members is a set with no contract.

## Keeping the registry current

Adding or modifying a component means updating
[`COMPONENTS.md`](./COMPONENTS.md) **in the same change**. See
[`AGENT_INSTRUCTIONS.md`](./AGENT_INSTRUCTIONS.md).
