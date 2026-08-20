import * as React from "react";
import { Info, Monitor, Moon, Sparkles, Sun } from "lucide-react";

import { Alert, AlertDescription } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { ColorSwatchPicker, hexToHsl, hslToHex } from "@/components/ColorSwatchPicker";
import { SegmentedControl } from "@/components/SegmentedControl";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { useAppearance } from "@/components/AccentThemeProvider";

/**
 * The three customization axes from docs/UI_GUIDELINES.md, as controls.
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
    theme,
    setTheme,
    blurLevel,
    setBlurLevel,
    reduceMotion,
    setReduceMotion,
    systemReduceMotion,
    reset,
  } = useAppearance();

  return (
    <div className="space-y-4">
      <Alert>
        <Info className="h-4 w-4" aria-hidden="true" />
        <AlertDescription>
          Appearance settings are saved on this device only. They will not follow
          you to another browser or machine.
        </AlertDescription>
      </Alert>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Theme</CardTitle>
          <CardDescription>
            Follow your system, or pin one palette regardless of what it says.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <SegmentedControl
            label="Theme"
            value={theme}
            onChange={setTheme}
            options={[
              { value: "light", label: "Light", icon: <Sun aria-hidden="true" /> },
              { value: "dark", label: "Dark", icon: <Moon aria-hidden="true" /> },
              {
                value: "system",
                label: "System",
                icon: <Monitor aria-hidden="true" />,
              },
            ]}
          />
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
        <CardContent className="space-y-4">
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
        <CardContent className="space-y-4">
          <div className="space-y-2 rounded-md border p-3">
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

          <div className="flex items-start justify-between gap-4 rounded-md border p-3">
            <div className="space-y-0.5">
              <Label htmlFor="motion-toggle">Reduce motion</Label>
              <p className="text-sm text-muted-foreground">
                {systemReduceMotion
                  ? "Your system is set to reduce motion, so this is already on everywhere and cannot be turned off here."
                  : "Replaces movement and scaling with instant changes. Nothing that indicates state disappears."}
              </p>
            </div>
            <Switch
              id="motion-toggle"
              // Shown as on when the OS asks for it, because it *is* on. The
              // switch is then disabled rather than merely ignored: a control
              // that visibly does nothing when clicked reads as broken, and
              // this setting is a floor the app must not lower.
              checked={reduceMotion || systemReduceMotion}
              disabled={systemReduceMotion}
              onCheckedChange={setReduceMotion}
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
    <div className="space-y-2">
      <Label htmlFor="accent-hex">Custom colour</Label>
      <div className="flex items-center gap-2">
        <div
          aria-hidden="true"
          className="h-9 w-9 shrink-0 rounded-md border"
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
