import * as React from "react";
import { Check, Info, Sparkles } from "lucide-react";

import { Alert, AlertDescription } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import { ColorSwatchPicker, hexToHsl, hslToHex } from "@loom/ui-kit/components/ColorSwatchPicker";
import { SegmentedControl } from "@loom/ui-kit/components/SegmentedControl";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import {
  useAppearance,
  type BackgroundTheme,
  type FontFamily,
} from "@loom/ui-kit/components/AccentThemeProvider";

/**
 * The customization axes from docs/UI_GUIDELINES.md, as controls.
 *
 * Nothing here has a save button. Each change applies immediately and is
 * written to `localStorage` on the spot — which is the honest interaction for
 * settings whose entire effect is visible the instant they change. A save
 * button would ask the user to confirm something they can already see.
 */
export function AppearancePanel() {
  const {
    accent,
    setAccent,
    backgroundTheme,
    setBackgroundTheme,
    blurLevel,
    setBlurLevel,
    animationLevel,
    setAnimationLevel,
    systemReduceMotion,
    density,
    setDensity,
    fontSizeScale,
    setFontSizeScale,
    fontFamily,
    setFontFamily,
    reset,
  } = useAppearance();

  return (
    <div className="flex flex-col gap-4">
      <Alert>
        <Info className="h-4 w-4" aria-hidden="true" />
        <AlertDescription>
          Appearance settings are saved on this device only. They will not follow
          you to another browser or machine.
        </AlertDescription>
      </Alert>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Background theme</CardTitle>
          <CardDescription>
            Choose the neutral canvas and surfaces independently from your accent colour.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <BackgroundThemePicker value={backgroundTheme} onChange={setBackgroundTheme} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Display density</CardTitle>
          <CardDescription>
            Comfortable, Compact, and Dense progressively reduce non-interactive
            spacing. Controls keep the same touch-friendly hit areas.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <SegmentedControl
            label="Display density"
            value={density}
            onChange={setDensity}
            options={[
              { value: "comfortable", label: "Comfortable" },
              { value: "compact", label: "Compact" },
              { value: "dense", label: "Dense" },
            ]}
          />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Typography</CardTitle>
          <CardDescription>
            Scale text throughout Loom and choose a locally bundled interface typeface.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          <div className="flex flex-col gap-2">
            <Label>Font size</Label>
            <SegmentedControl
              label="Font size"
              value={fontSizeScale}
              onChange={setFontSizeScale}
              options={[
                { value: "small", label: "Small" },
                { value: "medium", label: "Medium" },
                { value: "large", label: "Large" },
              ]}
            />
          </div>
          <FontFamilyPicker value={fontFamily} onChange={setFontFamily} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Accent colour</CardTitle>
          <CardDescription>
            Drives every accent-derived shade in the interface — buttons, focus
            rings, selected states.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <ColorSwatchPicker value={accent} onChange={setAccent} />
          <CustomAccentField value={accent} onChange={setAccent} />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Surfaces and motion</CardTitle>
          <CardDescription>
            Both are performance settings as much as visual ones.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          <div className="flex flex-col gap-2 rounded-md border p-3">
            <Label id="blur-level-label">Blurred surfaces</Label>
            <p className="text-sm text-muted-foreground">
              {blurLevel === "off" &&
                "Surfaces are solid. Easiest to read, and cheapest to render on weaker hardware."}
              {blurLevel === "standard" &&
                "Dialogs, popovers and the header get a frosted backdrop."}
              {blurLevel === "extra" &&
                "Frosted everywhere, over a soft wash of your accent colour — so the glass has something to refract. Costs the most to render."}
            </p>
            <SegmentedControl
              label="Blurred surfaces"
              value={blurLevel}
              onChange={setBlurLevel}
              options={[
                { value: "off", label: "Off" },
                { value: "standard", label: "Standard" },
                {
                  value: "extra",
                  label: "Extra",
                  icon: <Sparkles aria-hidden="true" />,
                },
              ]}
            />
          </div>

          <div className="flex flex-col gap-2 rounded-md border p-3">
            <Label>Animation</Label>
            <p className="text-sm text-muted-foreground">
              {systemReduceMotion
                ? "Your system requests reduced motion, so Full is displayed as Reduced in practice. You can still choose None."
                : "Reduced removes movement and scaling. None disables transitions and animations entirely."}
            </p>
            <SegmentedControl
              label="Animation level"
              value={animationLevel}
              onChange={setAnimationLevel}
              options={[
                { value: "full", label: "Full" },
                { value: "reduced", label: "Reduced" },
                { value: "none", label: "None" },
              ]}
            />
          </div>
        </CardContent>
      </Card>

      <div>
        <Button variant="outline" size="sm" onClick={reset}>
          Reset to defaults
        </Button>
      </div>
    </div>
  );
}

const BACKGROUND_PRESETS: Array<{
  value: BackgroundTheme;
  label: string;
  background: string;
  surface: string;
}> = [
  { value: "midnight", label: "Midnight", background: "#080a10", surface: "#0e111a" },
  { value: "slate", label: "Slate", background: "#0f172a", surface: "#172036" },
  { value: "charcoal", label: "Charcoal", background: "#1d1b1a", surface: "#252220" },
  { value: "daylight", label: "Daylight", background: "#ffffff", surface: "#f1f5f9" },
  { value: "cream", label: "Cream", background: "#f5f0df", surface: "#fcf9ef" },
];

function BackgroundThemePicker({
  value,
  onChange,
}: {
  value: BackgroundTheme;
  onChange: (theme: BackgroundTheme) => void;
}) {
  return (
    <div role="group" aria-label="Background theme" className="flex flex-wrap gap-2">
      {BACKGROUND_PRESETS.map((preset) => {
        const selected = preset.value === value;
        return (
          <Button
            key={preset.value}
            type="button"
            variant="outline"
            aria-pressed={selected}
            onClick={() => onChange(preset.value)}
            className="h-auto gap-2 px-3"
          >
            <span
              aria-hidden="true"
              className="relative size-6 overflow-hidden rounded-full border shadow-sm"
              style={{ backgroundColor: preset.background }}
            >
              <span
                className="absolute inset-x-1 bottom-1 h-2 rounded-full"
                style={{ backgroundColor: preset.surface }}
              />
            </span>
            {preset.label}
            {selected ? <Check aria-hidden="true" /> : null}
          </Button>
        );
      })}
    </div>
  );
}

const FONT_FAMILIES: Array<{
  value: FontFamily;
  label: string;
  css: string;
}> = [
  { value: "default", label: "Default", css: "ui-sans-serif, system-ui, sans-serif" },
  { value: "compact", label: "IBM Plex Sans", css: '"IBM Plex Sans", sans-serif' },
  { value: "rounded", label: "Nunito", css: '"Nunito", sans-serif' },
];

function FontFamilyPicker({
  value,
  onChange,
}: {
  value: FontFamily;
  onChange: (family: FontFamily) => void;
}) {
  return (
    <div className="flex flex-col gap-2">
      <Label>Font family</Label>
      <div role="group" aria-label="Font family" className="grid gap-2 sm:grid-cols-3">
        {FONT_FAMILIES.map((option) => (
          <Button
            key={option.value}
            type="button"
            variant={value === option.value ? "secondary" : "outline"}
            aria-pressed={value === option.value}
            onClick={() => onChange(option.value)}
            className="h-auto min-h-16 flex-col items-start whitespace-normal px-3 py-2 text-left"
            style={{ fontFamily: option.css }}
          >
            <span className="font-semibold">{option.label}</span>
            <span className="text-xs font-normal text-muted-foreground">Loom at a glance</span>
          </Button>
        ))}
      </div>
      <p className="text-sm text-muted-foreground">
        IBM Plex Sans is the narrower option for compact layouts; Nunito provides a softer,
        rounded alternative. Both are bundled for offline use.
      </p>
    </div>
  );
}

/**
 * A hex field for an accent outside the presets.
 *
 * Local state rather than writing through on every keystroke: a half-typed
 * `#2f` is a valid three-digit hex, so writing through would repaint the whole
 * interface a wrong colour mid-word. It commits only once the value parses as a
 * complete colour, and shows what it is about to apply next to the field.
 */
function CustomAccentField({
  value,
  onChange,
}: {
  value: string;
  onChange: (accent: string) => void;
}) {
  const [draft, setDraft] = React.useState(() => hslToHex(value) ?? "");

  // Follow the presets: picking a swatch should update what this field shows,
  // since it is displaying the same setting from the other direction.
  React.useEffect(() => {
    setDraft(hslToHex(value) ?? "");
  }, [value]);

  const parsed = hexToHsl(draft);
  const invalid = draft.trim() !== "" && parsed === null;

  function commit(next: string) {
    setDraft(next);
    const hsl = hexToHsl(next);
    if (hsl !== null) onChange(hsl);
  }

  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor="accent-hex">Custom colour</Label>
      <div className="flex items-center gap-2">
        <div
          aria-hidden="true"
          className="size-9 shrink-0 rounded-md border"
          // Previews the value being typed, falling back to the applied accent
          // so the swatch is never blank.
          style={{ backgroundColor: `hsl(${parsed ?? value})` }}
        />
        <Input
          id="accent-hex"
          value={draft}
          onChange={(event) => commit(event.target.value)}
          placeholder="#3b82f6"
          spellCheck={false}
          autoComplete="off"
          aria-invalid={invalid}
          aria-describedby="accent-hex-hint"
          className="max-w-40 font-mono"
        />
      </div>
      <p
        id="accent-hex-hint"
        className={invalid ? "text-sm text-destructive" : "text-sm text-muted-foreground"}
      >
        {invalid
          ? "Enter a hex colour such as #3b82f6."
          : "Three- or six-digit hex. Applies as soon as it is complete."}
      </p>
    </div>
  );
}
