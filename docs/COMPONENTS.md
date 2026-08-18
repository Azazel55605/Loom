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

**No components exist yet** — `apps/` has not been scaffolded and the shared UI
kit in `crates/core` has not been started. The table below is seeded with its
header only. Fill it in as the first components land.

| Component | Source | Variants | Used in | Notes |
| --------- | ------ | -------- | ------- | ----- |
