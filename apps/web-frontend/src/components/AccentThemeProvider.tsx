import * as React from "react";

/**
 * Applies the accent-color custom property described in docs/UI_GUIDELINES.md.
 *
 * This is the mechanism, not the policy: the accent is a single HSL triplet
 * ("H S% L%", no hsl() wrapper) written to the document root, from which every
 * derived shade and interaction state is computed by Tailwind. Today it is
 * hardcoded to one default, but nothing downstream assumes that — swapping this
 * for a user-persisted setting means changing this file only, not every
 * component.
 *
 * Blur intensity and reduced-transparency are the other two customization axes
 * and are driven the same way (--blur-surface / --surface-alpha in index.css);
 * they get their controls here when user settings land.
 */

/** Default accent: bare HSL channels, matching the token in index.css. */
export const DEFAULT_ACCENT = "217 91% 60%";

type AccentThemeContextValue = {
  accent: string;
  setAccent: (accent: string) => void;
};

const AccentThemeContext = React.createContext<AccentThemeContextValue | null>(null);

export function AccentThemeProvider({
  children,
  defaultAccent = DEFAULT_ACCENT,
}: {
  children: React.ReactNode;
  defaultAccent?: string;
}) {
  const [accent, setAccent] = React.useState(defaultAccent);

  React.useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
  }, [accent]);

  const value = React.useMemo(() => ({ accent, setAccent }), [accent]);

  return (
    <AccentThemeContext.Provider value={value}>{children}</AccentThemeContext.Provider>
  );
}

/** Read or change the current accent. Throws outside an AccentThemeProvider. */
export function useAccentTheme() {
  const context = React.useContext(AccentThemeContext);
  if (!context) {
    throw new Error("useAccentTheme must be used within an AccentThemeProvider");
  }
  return context;
}
