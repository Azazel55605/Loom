import { useAppearance, type AnimationLevel } from "@loom/ui-kit/components/AccentThemeProvider";

export type MotionTiming = {
  fast: number;
  base: number;
  slow: number;
  easing: string;
};

/**
 * Loom's single JavaScript-side motion scale, mirrored by the CSS custom
 * properties in styles.css. Components should use this only when a timing has
 * to be known by JavaScript; ordinary visual transitions stay in CSS.
 */
export const MOTION_TIMINGS: Record<AnimationLevel, MotionTiming> = {
  full: {
    fast: 120,
    base: 200,
    slow: 320,
    easing: "cubic-bezier(0.2, 0.8, 0.2, 1)",
  },
  reduced: {
    fast: 80,
    base: 120,
    slow: 160,
    easing: "ease-out",
  },
  none: { fast: 0, base: 0, slow: 0, easing: "linear" },
};

/** The effective level already includes the OS reduced-motion floor. */
export function useAnimationLevel() {
  const { effectiveAnimationLevel } = useAppearance();
  return {
    level: effectiveAnimationLevel,
    timing: MOTION_TIMINGS[effectiveAnimationLevel],
    decorative: effectiveAnimationLevel === "full",
  } as const;
}
