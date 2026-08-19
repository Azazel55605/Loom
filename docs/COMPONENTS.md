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
now duplicated across `web-frontend`, `desktop` and `mobile` — that duplication is
precisely what the shared kit is meant to remove, and the "Used in" column below
is what makes it visible. Components consumed by more than one client should
move into the kit once it exists.

| Component | Source | Variants | Used in | Notes |
| --------- | ------ | -------- | ------- | ----- |
| `Card` | shadcn | — | web-frontend, desktop, mobile | Includes `CardHeader`, `CardTitle`, `CardDescription`, `CardContent`, `CardFooter`. Uses the `.surface-elevated` class so the blur/reduced-transparency tokens apply. |
| `Badge` | shadcn-extended | `default`, `secondary`, `destructive`, `outline`, `healthy`, `degraded`, `down`, `unknown` | web-frontend, desktop, mobile | `default` resolves from the accent token via Tailwind's `primary` colour. The four health variants were added in web-frontend for connector status and resolve from the `--status-*` tokens in `index.css` — semantic status colours with fixed hues, deliberately **not** accent-derived, so "healthy" keeps meaning healthy when the user picks a red accent. Desktop and mobile still carry the unextended copy; they gain the variants when they gain a connector view. |
| `Skeleton` | shadcn | — | web-frontend | Loading state for the connector list (placeholder cards matching the real layout, so it does not jump when data lands). Animation is suppressed by the global `prefers-reduced-motion` rule in `index.css`. |
| `Alert` | shadcn | `default`, `destructive` | web-frontend, desktop, mobile | Includes `AlertTitle`, `AlertDescription`. Error state for an unreachable backend; icon from `lucide-react`. |
| `Input` | shadcn | — | web-frontend, desktop, mobile | Used as-is for the server URL field (desktop, mobile) and the login form (web-frontend). Themed replacement for a native `<input>`, per docs/UI_GUIDELINES.md. The web-frontend copy is byte-identical to the desktop one. |
| `Button` | shadcn | `default`, `destructive`, `outline`, `secondary`, `ghost`, `link` (sizes `default`, `sm`, `lg`, `icon`) | web-frontend, desktop, mobile | Submit control for the server URL form (desktop, mobile); login submit, sign-out, and connector action buttons (web-frontend). The web-frontend copy is byte-identical to the desktop one. |
| `ServerUrlField` | custom | — | desktop, mobile | Thin composition of `Input` + `Button`, not a new primitive — it adds the form wrapper (submit on Enter) and labelling only. Exists because desktop builds must reach a user-chosen server at runtime, whereas web-frontend bakes its API URL in at build time. No Radix primitive needed: the underlying elements are already shadcn components. |
| `Label` | shadcn | — | web-frontend | Wraps `@radix-ui/react-label`, which handles the click-to-focus association a bare `<label>` only gets right by convention. Used through `FormLabel` rather than directly. |
| `Form` | shadcn | — | web-frontend | react-hook-form bindings: `Form`, `FormField`, `FormItem`, `FormLabel`, `FormControl`, `FormDescription`, `FormMessage`, `useFormField`. Taken as-is for the accessibility wiring — it associates an error message with the input that produced it via `aria-describedby`/`aria-invalid`, which is exactly what rots when written per form by hand. Validation is zod through `@hookform/resolvers`. |
| `Toaster` | shadcn | — | web-frontend | Sonner, mounted once in `main.tsx`. Sonner is shadcn/ui's current toast component upstream, having replaced the older `Toast`/`useToast` pair. Restyled from the shared tokens (`.surface-elevated`, `--border`, accent) rather than Sonner's own palette, so the blur and reduced-transparency settings reach it. Respects `prefers-reduced-motion`. |
| `AppShell` | custom | — | web-frontend | The signed-in chrome: sticky header with the product name, the backend's core version, the current user, and sign-out. Built from `Button` + `Badge` plus layout; no Radix primitive because it owns no interactive behaviour of its own. Uses `.surface-elevated` so the blur tokens apply to the header. |
| `ConnectorCard` | custom | — | web-frontend | One connector: name, id, icon identifier, version, a health `Badge`, the last-checked time, and one `Button` per `ConnectorAction` returned by the API. Composition of `Card` + `Badge` + `Button` + `Alert`; no Radix primitive needed. Distinguishes a connector reporting `down` (a successful status check) from one Loom could not read at all (`status: null` + `statusError`), which are different states and render differently. Action buttons show a per-button spinner while pending and report the outcome through `Toaster`. |
| `LoginForm` | custom | — | web-frontend | The login card in `src/pages/LoginPage.tsx`, built from `Form` + `Input` + `Button` + `Alert` inside a `Card`. Not a separate exported component — it is the page's only content and has no second consumer, so extracting it would add indirection without removing duplication. Listed here because it is where the `Form` pattern is exercised. |
| `AccentThemeProvider` | custom | — | web-frontend, desktop, mobile | Theme provider, not a visual component. Writes the `--accent` HSL triplet to the document root and exposes `useAccentTheme()`. No Radix primitive needed — it renders no interactive UI. |
