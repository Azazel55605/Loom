import * as React from "react";

import { TooltipProvider } from "@loom/ui-kit/components/ui/tooltip";

/**
 * Owns the customization axes from docs/UI_GUIDELINES.md — accent colour, blur,
 * motion, density, typography, and background palette — and persists them per device.
 *
 * This is the mechanism, not the policy. Each axis is a CSS custom property or
 * a root-level class or data attribute defined in `styles.css`; nothing
 * downstream knows a user chose it, so components respond without extra wiring.
 *
 * ## Persistence is per device, deliberately for now
 *
 * Values live in `localStorage`, so they do not follow a user to another
 * browser or machine. That is a real limitation and worth stating rather than
 * hiding — the Appearance panel says so on screen. Syncing them would mean
 * columns on `users` and a backend contract, which is a decision to make
 * deliberately rather than one to arrive at by accident; see UI_GUIDELINES.md.
 *
 * ## Why the OS signals are read here and not only in CSS
 *
 * `styles.css` already honours `prefers-reduced-transparency` and
 * `prefers-reduced-motion` on its own. This provider reads them too, because a
 * *control* has to explain the state the user is actually in: a Full selection
 * while the OS is forcing Reduced needs to say so, and a blur control has to
 * know what the default was in order to have one.
 */

/** Default accent: bare HSL channels, matching the token in styles.css. */
export const DEFAULT_ACCENT = "217 91% 60%";

/** Storage keys. Prefixed like every other Loom key on the origin. */
const ACCENT_KEY = "loom-accent-color";
const BLUR_LEVEL_KEY = "loom-blur-level";
const ANIMATION_LEVEL_KEY = "loom-animation-level";
const LEGACY_REDUCE_MOTION_KEY = "loom-reduce-motion";
const LEGACY_THEME_KEY = "loom-theme";
const DENSITY_KEY = "loom-density";
const DENSITY_VERSION_KEY = "loom-density-version";
const FONT_SIZE_KEY = "loom-font-size";
const FONT_FAMILY_KEY = "loom-font-family";
const BACKGROUND_THEME_KEY = "loom-background-theme";

/**
 * Superseded by [`BLUR_LEVEL_KEY`], read once so an existing choice survives.
 *
 * The setting used to be a boolean. Dropping the old key would silently reset
 * everyone who had turned blur off — a small thing, but the kind of small thing
 * that makes preferences feel unreliable.
 */
const LEGACY_BLUR_KEY = "loom-blur-enabled";

/** How much of the interface is frosted. */
export type BlurLevel = "off" | "standard" | "extra";

/** The light/dark character of the selected background preset. */
export type ResolvedTheme = "light" | "dark";

/** How much non-interactive information fits into a surface. */
export type DisplayDensity = "comfortable" | "compact" | "dense";

/** Base rem scale applied across the interface. */
export type FontSizeScale = "small" | "medium" | "large";

/** Curated, locally bundled UI typeface. */
export type FontFamily = "default" | "compact" | "rounded";

/** User-selected motion ceiling. The OS may make it stricter. */
export type AnimationLevel = "full" | "reduced" | "none";

/** Complete neutral palette, independent from the interactive accent. */
export type BackgroundTheme = "midnight" | "slate" | "charcoal" | "daylight" | "cream";

/** The root class applied for each blur level. Exactly one is ever present. */
const BLUR_CLASSES: Record<BlurLevel, string> = {
  off: "reduce-transparency",
  standard: "force-transparency",
  extra: "blur-extra",
};

type AppearanceContextValue = {
  /** The accent as bare HSL channels, e.g. `"217 91% 60%"`. */
  accent: string;
  setAccent: (accent: string) => void;
  /** Whether the selected background preset is fundamentally light or dark. */
  resolvedTheme: ResolvedTheme;
  /** How much of the interface is frosted. */
  blurLevel: BlurLevel;
  setBlurLevel: (level: BlurLevel) => void;
  animationLevel: AnimationLevel;
  setAnimationLevel: (level: AnimationLevel) => void;
  /**
   * Whether the OS is asking for reduced motion.
   *
   * Exposed so the panel can explain why Full resolves to Reduced.
   */
  systemReduceMotion: boolean;
  /** What is actually applied: the stricter of the user and OS settings. */
  effectiveReduceMotion: boolean;
  /** Effective motion after applying the OS accessibility floor. */
  effectiveAnimationLevel: AnimationLevel;
  /** Non-interactive spacing and typography density. */
  density: DisplayDensity;
  setDensity: (density: DisplayDensity) => void;
  fontSizeScale: FontSizeScale;
  setFontSizeScale: (scale: FontSizeScale) => void;
  fontFamily: FontFamily;
  setFontFamily: (family: FontFamily) => void;
  backgroundTheme: BackgroundTheme;
  setBackgroundTheme: (theme: BackgroundTheme) => void;
  /** Returns every axis to its default, clearing the stored values. */
  reset: () => void;
};

const AppearanceContext = React.createContext<AppearanceContextValue | null>(null);

/** Reads a stored string, tolerating storage being unavailable. */
function readStored(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    // Private browsing and strict cookie settings can make localStorage throw
    // on access. That costs persistence, not function.
    return null;
  }
}

function writeStored(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // As above: best-effort. The setting still applies for this session.
  }
}

function clearStored(key: string): void {
  try {
    window.localStorage.removeItem(key);
  } catch {
    /* best-effort */
  }
}

/** Whether a media query currently matches, safe to call before mount. */
function mediaMatches(query: string): boolean {
  return typeof window !== "undefined" && window.matchMedia(query).matches;
}

/**
 * Bare HSL channels, e.g. `"217 91% 60%"`.
 *
 * Validated on read because this is persisted data from a previous version of
 * the app as much as from this one, and an unparseable value written into
 * `--accent` would cascade into every accent-derived colour at once — producing
 * an interface with no visible accent and no obvious cause.
 */
export function isValidAccent(value: string): boolean {
  return /^\d{1,3}\s+\d{1,3}%\s+\d{1,3}%$/.test(value.trim());
}

export function AccentThemeProvider({
  children,
  defaultAccent = DEFAULT_ACCENT,
}: {
  children: React.ReactNode;
  defaultAccent?: string;
}) {
  // Initialised from storage inside `useState` rather than in an effect: doing
  // it in an effect would paint one frame with the default accent before
  // correcting itself, which reads as a flash of the wrong colour on every load.
  const [accent, setAccentState] = React.useState(() => {
    const stored = readStored(ACCENT_KEY);
    return stored !== null && isValidAccent(stored) ? stored : defaultAccent;
  });

  const [blurLevel, setBlurLevelState] = React.useState<BlurLevel>(() => {
    const stored = readStored(BLUR_LEVEL_KEY);
    if (stored === "off" || stored === "standard" || stored === "extra") return stored;

    // Fall back to the boolean this setting used to be, so an existing choice
    // is carried forward rather than silently reset.
    const legacy = readStored(LEGACY_BLUR_KEY);
    if (legacy === "true") return "standard";
    if (legacy === "false") return "off";

    // No stored choice at all: follow the OS, which is what UI_GUIDELINES.md
    // asks for. Once the user picks, their choice wins in both directions —
    // unlike motion, where the OS signal is a floor rather than a default.
    return mediaMatches("(prefers-reduced-transparency: reduce)") ? "off" : "standard";
  });

  const [backgroundTheme, setBackgroundThemeState] = React.useState<BackgroundTheme>(() => {
    const stored = readStored(BACKGROUND_THEME_KEY);
    let value: BackgroundTheme;
    if (
      stored === "midnight" ||
      stored === "slate" ||
      stored === "charcoal" ||
      stored === "daylight" ||
      stored === "cream"
    ) {
      value = stored;
    } else {
      // Preserve explicit choices from the superseded Light/Dark/System
      // selector. An unresolved System preference becomes Slate, the new
      // stable default, rather than changing whenever the OS does.
      value = readStored(LEGACY_THEME_KEY) === "light" ? "daylight" : "slate";
    }
    if (typeof document !== "undefined") {
      const root = document.documentElement;
      const isDark = value !== "daylight" && value !== "cream";
      root.dataset.backgroundTheme = value;
      root.classList.toggle("dark", isDark);
      root.style.colorScheme = isDark ? "dark" : "light";
    }
    return value;
  });

  const resolvedTheme: ResolvedTheme =
    backgroundTheme === "daylight" || backgroundTheme === "cream" ? "light" : "dark";

  const [animationLevel, setAnimationLevelState] = React.useState<AnimationLevel>(() => {
    const stored = readStored(ANIMATION_LEVEL_KEY);
    const value =
      stored === "full" || stored === "reduced" || stored === "none"
        ? stored
        : readStored(LEGACY_REDUCE_MOTION_KEY) === "true"
          ? "reduced"
          : "full";
    if (typeof document !== "undefined") {
      const effective =
        value === "none" || value === "reduced"
          ? value
          : mediaMatches("(prefers-reduced-motion: reduce)")
            ? "reduced"
            : "full";
      document.documentElement.dataset.animationLevel = effective;
    }
    return value;
  });

  const [density, setDensityState] = React.useState<DisplayDensity>(() => {
    const stored = readStored(DENSITY_KEY);
    const version = readStored(DENSITY_VERSION_KEY);
    let value: DisplayDensity;
    if (version !== "3") {
      // Version 2 called today's Compact values "Dense". Persist the migrated
      // value and a schema marker so a newly selected v3 Dense value remains
      // Dense on future launches.
      value = stored === "dense" ? "compact" : "comfortable";
      writeStored(DENSITY_KEY, value);
      writeStored(DENSITY_VERSION_KEY, "3");
    } else {
      value =
        stored === "compact" || stored === "dense" || stored === "comfortable"
          ? stored
          : "comfortable";
    }
    // Density includes react-grid-layout spacing, which must be read from CSS
    // during the first descendant render. Apply the selector before children
    // mount so a persisted Dense choice cannot paint one Comfortable frame.
    if (typeof document !== "undefined") document.documentElement.dataset.density = value;
    return value;
  });

  const [fontSizeScale, setFontSizeScaleState] = React.useState<FontSizeScale>(() => {
    const stored = readStored(FONT_SIZE_KEY);
    const value = stored === "small" || stored === "large" ? stored : "medium";
    if (typeof document !== "undefined") document.documentElement.dataset.fontSize = value;
    return value;
  });

  const [fontFamily, setFontFamilyState] = React.useState<FontFamily>(() => {
    const stored = readStored(FONT_FAMILY_KEY);
    const value = stored === "compact" || stored === "rounded" ? stored : "default";
    if (typeof document !== "undefined") document.documentElement.dataset.fontFamily = value;
    return value;
  });

  // Tracked live rather than read once: someone changing the OS setting while
  // the tab is open should see the switch update, not a stale answer.
  const [systemReduceMotion, setSystemReduceMotion] = React.useState(() =>
    mediaMatches("(prefers-reduced-motion: reduce)"),
  );

  React.useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const listener = (event: MediaQueryListEvent) => setSystemReduceMotion(event.matches);
    query.addEventListener("change", listener);
    return () => query.removeEventListener("change", listener);
  }, []);

  // The OS setting is a floor, not a default: a user may force motion *off*
  // when the OS has not asked for it, but must never be able to force it back
  // *on* against an OS request. Reduced motion is an accessibility need for
  // some people, and an app-level preference is not entitled to override it.
  const effectiveAnimationLevel: AnimationLevel =
    animationLevel === "none"
      ? "none"
      : animationLevel === "reduced" || systemReduceMotion
        ? "reduced"
        : "full";
  const effectiveReduceMotion = effectiveAnimationLevel !== "full";

  React.useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
  }, [accent]);

  React.useEffect(() => {
    const root = document.documentElement;
    // Exactly one class, always present. "No opinion" is not a reachable state
    // once the level has been resolved from storage or the OS, so each class
    // states its level outright rather than leaning on the media query.
    for (const [level, className] of Object.entries(BLUR_CLASSES)) {
      root.classList.toggle(className, level === blurLevel);
    }
  }, [blurLevel]);

  React.useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle("dark", resolvedTheme === "dark");
    root.dataset.backgroundTheme = backgroundTheme;
    // Tells the browser which palette its own surfaces belong to — scrollbars,
    // the canvas behind a rubber-band scroll, and any control we have not
    // themed. Without it a dark page keeps a light scrollbar.
    root.style.colorScheme = resolvedTheme;
  }, [backgroundTheme, resolvedTheme]);

  React.useEffect(() => {
    document.documentElement.dataset.animationLevel = effectiveAnimationLevel;
  }, [effectiveAnimationLevel]);

  React.useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

  React.useEffect(() => {
    document.documentElement.dataset.fontSize = fontSizeScale;
  }, [fontSizeScale]);

  React.useEffect(() => {
    document.documentElement.dataset.fontFamily = fontFamily;
  }, [fontFamily]);

  const setAccent = React.useCallback((next: string) => {
    const value = next.trim();
    if (!isValidAccent(value)) return;
    setAccentState(value);
    writeStored(ACCENT_KEY, value);
  }, []);

  const setBlurLevel = React.useCallback((next: BlurLevel) => {
    setBlurLevelState(next);
    writeStored(BLUR_LEVEL_KEY, next);
  }, []);

  const setAnimationLevel = React.useCallback((next: AnimationLevel) => {
    setAnimationLevelState(next);
    writeStored(ANIMATION_LEVEL_KEY, next);
  }, []);

  const setDensity = React.useCallback((next: DisplayDensity) => {
    document.documentElement.dataset.density = next;
    setDensityState(next);
    writeStored(DENSITY_KEY, next);
    writeStored(DENSITY_VERSION_KEY, "3");
  }, []);

  const setFontSizeScale = React.useCallback((next: FontSizeScale) => {
    document.documentElement.dataset.fontSize = next;
    setFontSizeScaleState(next);
    writeStored(FONT_SIZE_KEY, next);
  }, []);

  const setFontFamily = React.useCallback((next: FontFamily) => {
    document.documentElement.dataset.fontFamily = next;
    setFontFamilyState(next);
    writeStored(FONT_FAMILY_KEY, next);
  }, []);

  const setBackgroundTheme = React.useCallback((next: BackgroundTheme) => {
    const root = document.documentElement;
    const isDark = next !== "daylight" && next !== "cream";
    root.dataset.backgroundTheme = next;
    root.classList.toggle("dark", isDark);
    root.style.colorScheme = isDark ? "dark" : "light";
    setBackgroundThemeState(next);
    writeStored(BACKGROUND_THEME_KEY, next);
  }, []);

  const reset = React.useCallback(() => {
    for (const key of [
      ACCENT_KEY,
      BLUR_LEVEL_KEY,
      LEGACY_BLUR_KEY,
      ANIMATION_LEVEL_KEY,
      LEGACY_REDUCE_MOTION_KEY,
      LEGACY_THEME_KEY,
      DENSITY_KEY,
      DENSITY_VERSION_KEY,
      FONT_SIZE_KEY,
      FONT_FAMILY_KEY,
      BACKGROUND_THEME_KEY,
    ]) {
      clearStored(key);
    }
    setAccentState(defaultAccent);
    setBlurLevelState(
      mediaMatches("(prefers-reduced-transparency: reduce)") ? "off" : "standard",
    );
    setAnimationLevelState("full");
    setBackgroundThemeState("slate");
    setFontSizeScaleState("medium");
    setFontFamilyState("default");
    const root = document.documentElement;
    root.dataset.density = "comfortable";
    root.dataset.fontSize = "medium";
    root.dataset.fontFamily = "default";
    root.dataset.backgroundTheme = "slate";
    root.dataset.animationLevel = mediaMatches("(prefers-reduced-motion: reduce)")
      ? "reduced"
      : "full";
    root.classList.add("dark");
    root.style.colorScheme = "dark";
    setDensityState("comfortable");
  }, [defaultAccent]);

  const value = React.useMemo<AppearanceContextValue>(
    () => ({
      accent,
      setAccent,
      resolvedTheme,
      blurLevel,
      setBlurLevel,
      animationLevel,
      setAnimationLevel,
      systemReduceMotion,
      effectiveReduceMotion,
      effectiveAnimationLevel,
      density,
      setDensity,
      fontSizeScale,
      setFontSizeScale,
      fontFamily,
      setFontFamily,
      backgroundTheme,
      setBackgroundTheme,
      reset,
    }),
    [
      accent,
      setAccent,
      resolvedTheme,
      blurLevel,
      setBlurLevel,
      animationLevel,
      setAnimationLevel,
      systemReduceMotion,
      effectiveReduceMotion,
      effectiveAnimationLevel,
      density,
      setDensity,
      fontSizeScale,
      setFontSizeScale,
      fontFamily,
      setFontFamily,
      backgroundTheme,
      setBackgroundTheme,
      reset,
    ],
  );

  // `TooltipProvider` lives here rather than in each app's entry point: it is
  // required context for every Radix tooltip in the kit, it holds no
  // appearance state of its own, and mounting it once at the outermost ui-kit
  // provider is what lets a component deep in the tree use a tooltip without
  // three apps having to opt in first.
  return (
    <AppearanceContext.Provider value={value}>
      <TooltipProvider delayDuration={200}>{children}</TooltipProvider>
    </AppearanceContext.Provider>
  );
}

/**
 * Read or change the appearance settings.
 *
 * Throws outside an `AccentThemeProvider`, which is a wiring bug rather than a
 * runtime condition worth handling.
 */
export function useAppearance(): AppearanceContextValue {
  const context = React.useContext(AppearanceContext);
  if (!context) {
    throw new Error("useAppearance must be used within an AccentThemeProvider");
  }
  return context;
}
