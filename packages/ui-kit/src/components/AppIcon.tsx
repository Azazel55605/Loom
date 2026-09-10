import * as React from "react";
import type { TablerIcon } from "@tabler/icons-react";

import { cn } from "@loom/ui-kit/lib/utils";
import { genericIcon } from "@loom/ui-kit/lib/generic-icons";
import { resolveIconReference, type IconCatalogEntry } from "@loom/ui-kit/lib/icon-catalog";
import {
  BRAND_ICON_LOADERS,
  SIMPLE_ICON_LOADERS,
  loadTablerComponents,
} from "@loom/ui-kit/lib/icon-loaders";

/**
 * Draws any icon reference in the four-prefix convention documented on
 * `ConnectorMetadata::icon` — `lucide:`, `brand:`, `tabler:`, `simple-icons:`.
 *
 * ## Resolution
 *
 * `icon` first, then `fallback`, then a hard `lucide:server`. A reference that
 * names something this build does not have — an unknown prefix, a key outside a
 * source's curated set, an unprefixed string from an older client — falls
 * through to the next candidate rather than rendering nothing or throwing. What
 * counts as "has" is `icon-catalog.ts`, answered synchronously before anything
 * is loaded.
 *
 * ## Four sources, two rendering styles
 *
 * `lucide:` and `tabler:` are line drawings that inherit `currentColor`, so they
 * follow the theme like every other icon in the app. `brand:` and
 * `simple-icons:` are someone else's marks and sit on a neutral rounded plate:
 * a dashboard-icons logo keeps its own palette, and a Simple Icons logo — which
 * upstream ships as a single uncoloured path — is filled with the foreground
 * colour. Neither is ever recoloured with the accent. See `docs/UI_GUIDELINES.md`:
 * the accent colours *our* surfaces, not someone else's mark.
 *
 * ## Loading
 *
 * Everything except `lucide:` is `import()`ed on demand — one chunk per
 * vendored SVG, one chunk for the Tabler subset — and cached at module scope,
 * so only the first render of a given icon waits. While it is in flight the
 * space is held by an empty box of the final size, not the fallback icon, which
 * would flash a server and then swap. A chunk that fails to load leaves the
 * placeholder: a failed icon fetch is not a reason to take down the card it
 * belongs to, and there is no error boundary here to catch it.
 */

type LoadedIcon = { kind: "svg"; markup: string } | { kind: "component"; Component: TablerIcon };

/** Resolved assets by full reference, so only the first render of each waits. */
const assetCache = new Map<string, LoadedIcon>();

/**
 * Rejects anything that is not plainly an SVG document.
 *
 * Markup is inlined with `dangerouslySetInnerHTML`, which is what lets a logo be
 * sized by its container instead of living in an `<img>` box. That is only safe
 * because the input is a file vendored into this repository and reviewed when
 * it was added — never a value from the API, a connector, or a user: a
 * reference string only ever *selects* one of those files, through a
 * written-out loader map. This check is the belt to that braces.
 */
function isPlainSvg(markup: string): boolean {
  const trimmed = markup.trim();
  return trimmed.startsWith("<svg") && !/<\s*script/i.test(trimmed);
}

async function loadIcon(entry: IconCatalogEntry): Promise<LoadedIcon | null> {
  if (entry.source === "tabler") {
    const { TABLER_COMPONENTS } = await loadTablerComponents();
    const Component = TABLER_COMPONENTS[entry.name];
    return Component === undefined ? null : { kind: "component", Component };
  }
  const loaders = entry.source === "brand" ? BRAND_ICON_LOADERS : SIMPLE_ICON_LOADERS;
  const loader = loaders[entry.name];
  if (loader === undefined) return null;
  const { default: markup } = await loader();
  return isPlainSvg(markup) ? { kind: "svg", markup } : null;
}

/** The loaded asset for a lazily-loaded reference, or `null` while it loads (and
 *  always for `lucide:`, which needs no loading). */
function useLoadedIcon(entry: IconCatalogEntry): LoadedIcon | null {
  const key = entry.source === "lucide" ? null : entry.reference;
  const [state, setState] = React.useState<{ key: string | null; icon: LoadedIcon | null }>(
    () => ({ key, icon: key === null ? null : (assetCache.get(key) ?? null) }),
  );

  React.useEffect(() => {
    if (key === null) return;
    const cached = assetCache.get(key);
    if (cached !== undefined) {
      setState({ key, icon: cached });
      return;
    }
    let cancelled = false;
    void loadIcon(entry)
      .then((icon) => {
        if (icon === null) return;
        assetCache.set(key, icon);
        if (!cancelled) setState({ key, icon });
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
    // `entry` is the catalog's own object, so it changes exactly when `key` does.
  }, [key, entry]);

  if (key === null) return null;
  // A reference that just changed must not show the previous icon for a frame.
  return state.key === key ? state.icon : (assetCache.get(key) ?? null);
}

export function AppIcon({
  icon,
  fallback = null,
  size = 20,
  className,
}: {
  /** The reference to draw. */
  icon: string | null | undefined;
  /** Drawn when `icon` is absent or names nothing this build has. */
  fallback?: string | null;
  /** Edge length in pixels. The icon is square. */
  size?: number;
  className?: string;
}) {
  const entry = resolveIconReference([icon, fallback]);
  const loaded = useLoadedIcon(entry);
  const box = { width: size, height: size } as const;

  if (entry.source === "brand" || entry.source === "simple-icons") {
    const markup = loaded?.kind === "svg" ? loaded.markup : null;
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-[0.25rem]",
          "bg-muted/40 p-[0.1em] [&>svg]:h-full [&>svg]:w-full",
          // Simple Icons ship one uncoloured path; fill it with the foreground
          // colour rather than the browser's default black. Dashboard-icons
          // logos carry their own fills, which this does not touch.
          entry.source === "simple-icons" && "text-foreground [&>svg]:fill-current",
          className,
        )}
        style={box}
        role="img"
        aria-hidden="true"
        // Vendored, in-repo, shape-checked. See `isPlainSvg`.
        {...(markup === null ? {} : { dangerouslySetInnerHTML: { __html: markup } })}
      />
    );
  }

  if (entry.source === "tabler") {
    if (loaded?.kind !== "component") {
      return <span className={cn("inline-block shrink-0", className)} style={box} aria-hidden="true" />;
    }
    const { Component } = loaded;
    return (
      <Component
        size={size}
        className={cn("shrink-0 text-muted-foreground", className)}
        style={box}
        aria-hidden="true"
      />
    );
  }

  // `resolveIconReference` only returns catalogued `lucide:` names.
  const { Component } = genericIcon(entry.name)!;
  return (
    <Component
      className={cn("shrink-0 text-muted-foreground", className)}
      style={box}
      aria-hidden="true"
    />
  );
}
