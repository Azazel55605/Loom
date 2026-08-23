import * as React from "react";

import { cn } from "@loom/ui-kit/lib/utils";
import { genericIcon } from "@loom/ui-kit/lib/generic-icons";

/**
 * Draws the icon for a connector, resolving the `brand:`/`lucide:` reference
 * convention documented on `ConnectorMetadata::icon`.
 *
 * ## Resolution order
 *
 * 1. `iconOverride` — the user's per-instance choice, so two Docker hosts on
 *    one dashboard can be told apart.
 * 2. `typeIcon` — what the connector type itself declares.
 * 3. `FALLBACK_ICON` — a generic server.
 *
 * A reference that names something this build does not have (`brand:` with no
 * vendored file, `lucide:` outside the curated set, an unprefixed string from
 * an older connector) falls through to the next step rather than rendering
 * nothing or throwing. A connector is not broken because its icon is missing,
 * and the icon is not the place to find that out.
 *
 * ## Brand icons are not tinted
 *
 * A `lucide:` icon is a line drawing that inherits `currentColor`, so it picks
 * up the accent and the theme like every other icon in the app. A `brand:` icon
 * is a logo with its own palette — Docker's blue is what makes it recognisable
 * at 20px — so it is rendered at its natural colours and given a neutral
 * rounded plate to sit on rather than being recoloured. See
 * `docs/UI_GUIDELINES.md` on the accent axis: the accent colours *our* surfaces,
 * not someone else's mark.
 *
 * ## Loading
 *
 * Brand SVGs are `import()`ed on demand, so a deployment that has vendored
 * twenty of them ships none of the nineteen it is not drawing. Resolved markup
 * is cached at module scope, so the second card showing the same brand renders
 * synchronously. While the first one is in flight the space is held with an
 * empty box of the final size — not the fallback icon, which would flash a
 * server and then swap to a whale.
 */

/** Used when nothing else resolves. Must be a member of `GENERIC_ICONS`. */
const FALLBACK_ICON = "server";

/**
 * Every vendored brand SVG, keyed by filename stem.
 *
 * Written out rather than produced by `import.meta.glob` so this file is
 * bundler-agnostic and so the set is greppable: `docs/THIRD_PARTY_ICONS.md`
 * lists exactly these, and a vendored file with no entry here is dead weight
 * that a reader can find. Adding an icon is two lines — the file, and a line
 * here — plus its row in that document.
 */
const BRAND_ICONS: Record<string, () => Promise<{ default: string }>> = {
  docker: () => import("../assets/icons/brand/docker.svg?raw"),
};

/** Resolved brand markup, so only the first render of a given brand waits. */
const brandCache = new Map<string, string>();

/**
 * Rejects anything that is not plainly an SVG document.
 *
 * The markup is inlined with `dangerouslySetInnerHTML`, which is what lets a
 * logo be sized and positioned by its container instead of living in an `<img>`
 * box. That is only safe because the input is a file vendored into this
 * repository and reviewed when it was added — never a value from the API, a
 * connector, or a user. This check is the belt to that braces: a file that was
 * swapped for something else stops being rendered rather than being executed.
 */
function isPlainSvg(markup: string): boolean {
  const trimmed = markup.trim();
  return trimmed.startsWith("<svg") && !/<\s*script/i.test(trimmed);
}

/** Splits `"brand:docker"` into its two halves. Anything unprefixed is not a
 *  reference in the convention and resolves to nothing. */
function parse(reference: string | null): { scheme: string; name: string } | null {
  if (reference === null) return null;
  const separator = reference.indexOf(":");
  if (separator <= 0) return null;
  return {
    scheme: reference.slice(0, separator),
    name: reference.slice(separator + 1),
  };
}

export function ConnectorIcon({
  typeIcon,
  iconOverride,
  size = 20,
  className,
}: {
  /** `metadata.icon` — what the connector *type* declares. */
  typeIcon: string | null;
  /** `iconOverride` — the user's choice for this instance, if any. */
  iconOverride: string | null;
  /** Edge length in pixels. The icon is square. */
  size?: number;
  className?: string;
}) {
  // First reference that names something this build actually has. Written as a
  // loop over the priority order rather than as nested ternaries so that "an
  // override naming a missing brand falls back to the type's icon" is the
  // obvious reading rather than a subtle one.
  let resolved: { scheme: string; name: string } | null = null;
  for (const candidate of [iconOverride, typeIcon]) {
    const parsed = parse(candidate);
    if (parsed === null) continue;
    if (parsed.scheme === "brand" && parsed.name in BRAND_ICONS) {
      resolved = parsed;
      break;
    }
    if (parsed.scheme === "lucide" && genericIcon(parsed.name) !== undefined) {
      resolved = parsed;
      break;
    }
  }

  const brandKey = resolved?.scheme === "brand" ? resolved.name : null;
  const [markup, setMarkup] = React.useState<string | null>(() =>
    brandKey === null ? null : (brandCache.get(brandKey) ?? null),
  );

  React.useEffect(() => {
    if (brandKey === null) {
      setMarkup(null);
      return;
    }
    const cached = brandCache.get(brandKey);
    if (cached !== undefined) {
      setMarkup(cached);
      return;
    }

    let cancelled = false;
    void BRAND_ICONS[brandKey]()
      .then((module) => {
        if (!isPlainSvg(module.default)) return;
        brandCache.set(brandKey, module.default);
        if (!cancelled) setMarkup(module.default);
      })
      // A chunk that fails to load leaves `markup` null, which draws the
      // placeholder. Deliberately not escalated: a failed icon fetch is not a
      // reason to take down the card it belongs to, and there is no error
      // boundary here to catch it if it were thrown.
      .catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [brandKey]);

  const box = { width: size, height: size } as const;

  if (brandKey !== null) {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-[0.25rem]",
          // The logo keeps its own colours; only the plate behind it is themed.
          "bg-muted/40 p-[0.1em] [&>svg]:h-full [&>svg]:w-full",
          className,
        )}
        style={box}
        role="img"
        aria-hidden="true"
        // Vendored, in-repo, shape-checked above. See `isPlainSvg`.
        {...(markup === null ? {} : { dangerouslySetInnerHTML: { __html: markup } })}
      />
    );
  }

  const generic = genericIcon(resolved?.name ?? FALLBACK_ICON) ?? genericIcon(FALLBACK_ICON)!;
  const { Component } = generic;

  return (
    <Component
      className={cn("shrink-0 text-muted-foreground", className)}
      style={box}
      aria-hidden="true"
    />
  );
}
