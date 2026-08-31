import * as React from "react";

import { TooltipProvider } from "@loom/ui-kit/components/ui/tooltip";

/**
 * Owns the customization axes from docs/UI_GUIDELINES.md — accent colour, blur,
 * motion, and density — plus the light/dark palette, and persists them per device.
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
 * `index.css` already honours `prefers-reduced-transparency` and
 * `prefers-reduced-motion` on its own. This provider reads them too, because a
 * *control* has to show the state the user is actually in: a motion switch that
 * reads "off" while the OS is forcing motion off would be lying, and a blur
 * switch has to know what the default was in order to have one.
 */

/** Default accent: bare HSL channels, matching the token in index.css. */
export const DEFAULT_ACCENT = "217 91% 60%";

/** Storage keys. Prefixed like every other Loom key on the origin. */
const ACCENT_KEY = "loom-accent-color";
const BLUR_LEVEL_KEY = "loom-blur-level";
const REDUCE_MOTION_KEY = "loom-reduce-motion";
const THEME_KEY = "loom-theme";
const DENSITY_KEY = "loom-density";

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

/** Which palette to paint. `system` follows `prefers-color-scheme`. */
export type ThemePreference = "light" | "dark" | "system";

/** The palette actually in effect once `system` has been resolved. */
export type ResolvedTheme = "light" | "dark";

/** How much non-interactive information fits into a surface. */
export type DisplayDensity = "comfortable" | "dense";

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
  /** Which palette the user asked for, including `system`. */
  theme: ThemePreference;
  setTheme: (theme: ThemePreference) => void;
  /** The palette actually applied, with `system` resolved against the OS. */
  resolvedTheme: ResolvedTheme;
  /** How much of the interface is frosted. */
  blurLevel: BlurLevel;
  setBlurLevel: (level: BlurLevel) => void;
  /** The user's own "reduce motion" choice, independent of the OS. */
  reduceMotion: boolean;
  setReduceMotion: (reduce: boolean) => void;
  /**
   * Whether the OS is asking for reduced motion.
   *
   * Exposed so the panel can explain why its switch is forced on rather than
   * appearing broken.
   */
  systemReduceMotion: boolean;
  /** What is actually applied: the stricter of the user and OS settings. */
  effectiveReduceMotion: boolean;
  /** Non-interactive spacing and typography density. */
  density: DisplayDensity;
  setDensity: (density: DisplayDensity) => void;
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

  const [theme, setThemeState] = React.useState<ThemePreference>(() => {
    const stored = readStored(THEME_KEY);
    return stored === "light" || stored === "dark" || stored === "system"
      ? stored
      : // `system` rather than `light`: the OS already carries an answer, and
        // opening a new app in blazing white at night is a bad first
        // impression the user never asked for.
        "system";
  });

  const [systemDark, setSystemDark] = React.useState(() =>
    mediaMatches("(prefers-color-scheme: dark)"),
  );

  React.useEffect(() => {
    const query = window.matchMedia("(prefers-color-scheme: dark)");
    const listener = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener("change", listener);
    return () => query.removeEventListener("change", listener);
  }, []);

  const resolvedTheme: ResolvedTheme =
    theme === "system" ? (systemDark ? "dark" : "light") : theme;

  const [reduceMotion, setReduceMotionState] = React.useState(
    () => readStored(REDUCE_MOTION_KEY) === "true",
  );

  const [density, setDensityState] = React.useState<DisplayDensity>(() => {
    const stored = readStored(DENSITY_KEY) === "dense" ? "dense" : "comfortable";
    // Density includes react-grid-layout spacing, which must be read from CSS
    // during the first descendant render. Apply the selector before children
    // mount so a persisted Dense choice cannot paint one Comfortable frame.
    if (typeof document !== "undefined") document.documentElement.dataset.density = stored;
    return stored;
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
  const effectiveReduceMotion = reduceMotion || systemReduceMotion;

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
    // Tells the browser which palette its own surfaces belong to — scrollbars,
    // the canvas behind a rubber-band scroll, and any control we have not
    // themed. Without it a dark page keeps a light scrollbar.
    root.style.colorScheme = resolvedTheme;
  }, [resolvedTheme]);

  React.useEffect(() => {
    // Only ever added, never used to switch motion back on: the class layers on
    // top of the `prefers-reduced-motion` rule in index.css rather than
    // competing with it.
    document.documentElement.classList.toggle("reduce-motion", reduceMotion);
  }, [reduceMotion]);

  React.useEffect(() => {
    document.documentElement.dataset.density = density;
  }, [density]);

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

  const setTheme = React.useCallback((next: ThemePreference) => {
    setThemeState(next);
    writeStored(THEME_KEY, next);
  }, []);

  const setReduceMotion = React.useCallback((next: boolean) => {
    setReduceMotionState(next);
    writeStored(REDUCE_MOTION_KEY, String(next));
  }, []);

  const setDensity = React.useCallback((next: DisplayDensity) => {
    document.documentElement.dataset.density = next;
    setDensityState(next);
    writeStored(DENSITY_KEY, next);
  }, []);

  const reset = React.useCallback(() => {
    for (const key of [
      ACCENT_KEY,
      BLUR_LEVEL_KEY,
      LEGACY_BLUR_KEY,
      REDUCE_MOTION_KEY,
      THEME_KEY,
      DENSITY_KEY,
    ]) {
      clearStored(key);
    }
    setAccentState(defaultAccent);
    setBlurLevelState(
      mediaMatches("(prefers-reduced-transparency: reduce)") ? "off" : "standard",
    );
    setReduceMotionState(false);
    setThemeState("system");
    document.documentElement.dataset.density = "comfortable";
    setDensityState("comfortable");
  }, [defaultAccent]);

  const value = React.useMemo<AppearanceContextValue>(
    () => ({
      accent,
      setAccent,
      theme,
      setTheme,
      resolvedTheme,
      blurLevel,
      setBlurLevel,
      reduceMotion,
      setReduceMotion,
      systemReduceMotion,
      effectiveReduceMotion,
      density,
      setDensity,
      reset,
    }),
    [
      accent,
      setAccent,
      theme,
      setTheme,
      resolvedTheme,
      blurLevel,
      setBlurLevel,
      reduceMotion,
      setReduceMotion,
      systemReduceMotion,
      effectiveReduceMotion,
      density,
      setDensity,
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
