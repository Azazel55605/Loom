import { Check } from "lucide-react";

import { cn } from "@loom/ui-kit/lib/utils";

/**
 * Accent presets as a grid of swatches, plus the conversion helpers the custom
 * hex field needs.
 *
 * The stored form is always bare HSL channels (`"217 91% 60%"`), because that is
 * what `--accent` takes and what lets every derived shade recombine it with
 * different alpha and lightness — see docs/UI_GUIDELINES.md. Hex exists only at
 * the edge, where people type colours.
 *
 * Not a Radix primitive: this is a single-select group of buttons, and the
 * accessible construct for that is a radio group. It is built here as buttons
 * with `aria-pressed` rather than pulling in `@radix-ui/react-radio-group` for
 * one control — a decision worth revisiting the moment a second single-select
 * group appears, at which point the primitive earns its place.
 */

/** One preset. Names are what a screen reader announces, so they are real
 *  colour names rather than hex strings. */
export type AccentPreset = {
  name: string;
  /** Bare HSL channels, the stored form. */
  value: string;
};

/**
 * The presets.
 *
 * Eight hues spread around the wheel at a consistent saturation and lightness,
 * so no preset is conspicuously darker or more washed out than its neighbours —
 * and so each one keeps enough contrast against both the light and dark
 * backgrounds to be legible as a button colour.
 */
export const ACCENT_PRESETS: AccentPreset[] = [
  { name: "Blue", value: "217 91% 60%" },
  { name: "Indigo", value: "245 79% 63%" },
  { name: "Violet", value: "271 81% 64%" },
  { name: "Pink", value: "330 81% 60%" },
  { name: "Red", value: "0 84% 60%" },
  { name: "Amber", value: "38 92% 50%" },
  { name: "Emerald", value: "160 84% 39%" },
  { name: "Teal", value: "189 94% 43%" },
];

/** `"217 91% 60%"` → `#2f7fed`. Returns null for anything unparseable. */
export function hslToHex(hsl: string): string | null {
  const match = /^(\d{1,3})\s+(\d{1,3})%\s+(\d{1,3})%$/.exec(hsl.trim());
  if (match === null) return null;

  const hue = Number(match[1]);
  const saturation = Number(match[2]) / 100;
  const lightness = Number(match[3]) / 100;

  const chroma = (1 - Math.abs(2 * lightness - 1)) * saturation;
  const secondary = chroma * (1 - Math.abs(((hue / 60) % 2) - 1));
  const offset = lightness - chroma / 2;

  const [red, green, blue] = (() => {
    if (hue < 60) return [chroma, secondary, 0];
    if (hue < 120) return [secondary, chroma, 0];
    if (hue < 180) return [0, chroma, secondary];
    if (hue < 240) return [0, secondary, chroma];
    if (hue < 300) return [secondary, 0, chroma];
    return [chroma, 0, secondary];
  })();

  const toHex = (channel: number) =>
    Math.round((channel + offset) * 255)
      .toString(16)
      .padStart(2, "0");

  return `#${toHex(red)}${toHex(green)}${toHex(blue)}`;
}

/** `#2f7fed` or `#37e` → `"217 91% 60%"`. Returns null for anything else. */
export function hexToHsl(hex: string): string | null {
  const value = hex.trim().replace(/^#/, "");
  // Both the three- and six-digit forms, since people type either.
  const expanded =
    value.length === 3
      ? value
          .split("")
          .map((character) => character + character)
          .join("")
      : value;

  if (!/^[0-9a-fA-F]{6}$/.test(expanded)) return null;

  const red = parseInt(expanded.slice(0, 2), 16) / 255;
  const green = parseInt(expanded.slice(2, 4), 16) / 255;
  const blue = parseInt(expanded.slice(4, 6), 16) / 255;

  const max = Math.max(red, green, blue);
  const min = Math.min(red, green, blue);
  const delta = max - min;
  const lightness = (max + min) / 2;

  let hue = 0;
  if (delta !== 0) {
    if (max === red) hue = ((green - blue) / delta) % 6;
    else if (max === green) hue = (blue - red) / delta + 2;
    else hue = (red - green) / delta + 4;
  }
  hue = Math.round(hue * 60);
  if (hue < 0) hue += 360;

  const saturation =
    delta === 0 ? 0 : delta / (1 - Math.abs(2 * lightness - 1));

  return `${hue} ${Math.round(saturation * 100)}% ${Math.round(lightness * 100)}%`;
}

export function ColorSwatchPicker({
  value,
  onChange,
}: {
  /** The selected accent, as bare HSL channels. */
  value: string;
  onChange: (accent: string) => void;
}) {
  return (
    <div role="group" aria-label="Accent colour presets" className="flex flex-wrap gap-2">
      {ACCENT_PRESETS.map((preset) => {
        const selected = preset.value === value;
        return (
          <button
            key={preset.value}
            type="button"
            aria-pressed={selected}
            title={preset.name}
            onClick={() => onChange(preset.value)}
            // The ring is the same focus-ring convention the rest of the UI
            // uses, offset so it reads as a halo rather than a border — a
            // border would change the swatch's apparent size when selected.
            className={cn(
              "flex h-8 w-8 items-center justify-center rounded-full transition-transform",
              "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background",
              selected && "ring-2 ring-ring ring-offset-2 ring-offset-background",
            )}
            style={{ backgroundColor: `hsl(${preset.value})` }}
          >
            {/* Selection is not conveyed by the ring alone: a ring around a
                coloured circle is easy to miss, and impossible to see for
                someone who cannot distinguish the hues in the first place. */}
            {selected && (
              <Check className="h-4 w-4 text-white drop-shadow" aria-hidden="true" />
            )}
            <span className="sr-only">
              {preset.name}
              {selected ? " (selected)" : ""}
            </span>
          </button>
        );
      })}
    </div>
  );
}
