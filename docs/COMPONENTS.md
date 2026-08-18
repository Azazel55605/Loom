<!--
  LIVING REGISTRY — update this file in the same change as the code.

  Any time a UI component is added or modified anywhere in `apps/` or in a
  shared UI kit under `crates/core`, this table must be updated to match. This
  is not optional and is not a follow-up task: treat it exactly like updating a
  changelog. A component change with no registry change is an incomplete change.

  Column meanings:

  - Component — the exported component name, e.g. `Button`, `CommandPalette`.
  - Source    — one of:
                  `shadcn`          used as-is from shadcn/ui
                  `shadcn-extended` a shadcn component plus new CVA variants
                  `custom`          built from scratch, following shadcn's
                                    construction pattern (Radix primitive
                                    underneath, CVA variants, shared tokens)
                See docs/UI_GUIDELINES.md for the sourcing priority order —
                `custom` is a last resort and should be rare.
  - Variants  — the CVA variants available, e.g. `default, destructive, ghost`.
                Use `—` if the component has none.
  - Used in   — which client apps consume it: `web-frontend`, `desktop`,
                `mobile`, or a combination. This column is the point of the
                file: the shared UI kit exists to keep the three platforms
                consistent, so a component used by only one app is either
                genuinely platform-specific or an inconsistency worth noticing.
                Keep it accurate — a stale "Used in" hides exactly the drift
                this registry is meant to surface.
  - Notes     — anything a future maintainer needs: the Radix primitive it
                wraps, accessibility caveats, why it was built custom, known
                limitations.
-->

# Component registry

The living inventory of every UI component in Loom. Conventions governing how
these are built live in [`UI_GUIDELINES.md`](./UI_GUIDELINES.md) and change
rarely; this file changes constantly.

The shared UI kit in `crates/core` has not been started yet, so each client
carries its own copy under `apps/<app>/src/components/`. Several components are
now duplicated between `web-frontend` and `desktop` — that duplication is
precisely what the shared kit is meant to remove, and the "Used in" column below
is what makes it visible. Components consumed by more than one client should
move into the kit once it exists.

| Component | Source | Variants | Used in | Notes |
| --------- | ------ | -------- | ------- | ----- |
| `Card` | shadcn | — | web-frontend, desktop | Includes `CardHeader`, `CardTitle`, `CardDescription`, `CardContent`, `CardFooter`. Uses the `.surface-elevated` class so the blur/reduced-transparency tokens apply. |
| `Badge` | shadcn | `default`, `secondary`, `destructive`, `outline` | web-frontend, desktop | `default` resolves from the accent token via Tailwind's `primary` colour. |
| `Skeleton` | shadcn | — | web-frontend | Loading state for the health fetch. Animation is suppressed by the global `prefers-reduced-motion` rule in `index.css`. |
| `Alert` | shadcn | `default`, `destructive` | web-frontend, desktop | Includes `AlertTitle`, `AlertDescription`. Error state for an unreachable backend; icon from `lucide-react`. |
| `Input` | shadcn | — | desktop | Used as-is for the server URL field. Themed replacement for a native `<input>`, per docs/UI_GUIDELINES.md. |
| `Button` | shadcn | `default`, `destructive`, `outline`, `secondary`, `ghost`, `link` (sizes `default`, `sm`, `lg`, `icon`) | desktop | Submit control for the server URL form. |
| `ServerUrlField` | custom | — | desktop | Thin composition of `Input` + `Button`, not a new primitive — it adds the form wrapper (submit on Enter) and labelling only. Exists because desktop builds must reach a user-chosen server at runtime, whereas web-frontend bakes its API URL in at build time. No Radix primitive needed: the underlying elements are already shadcn components. |
| `AccentThemeProvider` | custom | — | web-frontend, desktop | Theme provider, not a visual component. Writes the `--accent` HSL triplet to the document root and exposes `useAccentTheme()`. No Radix primitive needed — it renders no interactive UI. |
