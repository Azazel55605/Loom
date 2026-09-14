import * as React from "react";
import { Check, ChevronLeft, ChevronRight, Loader2 } from "lucide-react";

import { IconPickerPopover } from "@loom/ui-kit/components/IconPicker";
import {
  isPlacementActionClickComplete,
  isPlacementActionComplete,
  isPlacementActionStateComplete,
  placementActionNeedsDisplayStep,
  PlacementActionEditor,
} from "@loom/ui-kit/components/PlacementActionEditor";
import { Button } from "@loom/ui-kit/components/ui/button";
import { DialogFooter } from "@loom/ui-kit/components/ui/dialog";
import { Input } from "@loom/ui-kit/components/ui/input";
import { Label } from "@loom/ui-kit/components/ui/label";
import { Progress } from "@loom/ui-kit/components/ui/progress";
import type { PlacementAction } from "@loom/ui-kit/lib/api";

const STEPS = ["Click behavior", "State awareness", "Display"] as const;

/**
 * The shared create/edit flow for connector-less dashboard buttons.
 *
 * Name and tile icon stay outside the changing step panel so an existing
 * button's identity is always visible and editable. The state-button-only
 * presentation fields occupy step three; navigation, stateless actions and
 * Switch rendering finish earlier without an empty ceremonial step.
 */
export function ButtonTileWizard({
  name,
  icon,
  action,
  currentDashboardId,
  disabled = false,
  pending = false,
  submitLabel,
  error,
  resetKey,
  onNameChange,
  onIconChange,
  onActionChange,
  onCancel,
}: {
  name: string;
  icon: string | null;
  action: PlacementAction | null;
  currentDashboardId: string;
  disabled?: boolean;
  pending?: boolean;
  submitLabel: string;
  error?: React.ReactNode;
  /** Changes whenever the hosting dialog starts a fresh editing session. */
  resetKey: string;
  onNameChange: (name: string) => void;
  onIconChange: (icon: string | null) => void;
  onActionChange: (action: PlacementAction | null) => void;
  onCancel: () => void;
}) {
  const [step, setStep] = React.useState(0);

  React.useEffect(() => setStep(0), [resetKey]);

  const nameComplete = name.trim() !== "";
  const clickComplete = isPlacementActionClickComplete(action);
  const stateComplete = isPlacementActionStateComplete(action);
  const needsDisplay = placementActionNeedsDisplayStep(action);
  const complete = nameComplete && isPlacementActionComplete(action);
  const isFinalStep = step === 2 || (step === 1 && !needsDisplay);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-4">
      <div className="grid shrink-0 gap-3 sm:grid-cols-[minmax(0,1fr)_minmax(12rem,0.65fr)]">
        <div className="flex flex-col gap-1.5">
          <Label htmlFor="button-tile-name">Name</Label>
          <Input
            id="button-tile-name"
            value={name}
            disabled={disabled}
            placeholder="Network"
            autoFocus
            onChange={(event) => onNameChange(event.target.value)}
          />
          <p className="text-xs text-muted-foreground">The label shown on the tile.</p>
        </div>
        <div className="flex flex-col gap-1.5">
          <Label>Tile icon</Label>
          <IconPickerPopover
            value={icon}
            defaultIcon={null}
            defaultLabel="No icon"
            label="Button tile icon"
            disabled={disabled}
            onChange={onIconChange}
          />
        </div>
      </div>

      <div className="flex shrink-0 flex-col gap-2">
        <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
          <span>Step {step + 1} of {STEPS.length}</span>
          <span>{STEPS[step]}</span>
        </div>
        <Progress value={((step + 1) / STEPS.length) * 100} />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain pr-1">
        {step === 0 ? (
          <PlacementActionEditor
            value={action}
            onChange={onActionChange}
            currentDashboardId={currentDashboardId}
            required
            disabled={disabled}
            section="click"
          />
        ) : null}
        {step === 1 ? (
          <PlacementActionEditor
            value={action}
            onChange={onActionChange}
            currentDashboardId={currentDashboardId}
            required
            disabled={disabled}
            section="state"
          />
        ) : null}
        {step === 2 ? (
          <PlacementActionEditor
            value={action}
            onChange={onActionChange}
            currentDashboardId={currentDashboardId}
            required
            disabled={disabled}
            section="display"
          />
        ) : null}
        {error}
      </div>

      <DialogFooter className="shrink-0 gap-2 sm:gap-2">
        {step === 0 ? (
          <Button type="button" variant="outline" disabled={disabled} onClick={onCancel}>
            Cancel
          </Button>
        ) : (
          <Button
            type="button"
            variant="outline"
            disabled={disabled}
            onClick={() => setStep((current) => Math.max(0, current - 1))}
          >
            <ChevronLeft data-icon="inline-start" aria-hidden="true" />
            Back
          </Button>
        )}

        {isFinalStep ? (
          <Button type="submit" disabled={disabled || pending || !complete}>
            {pending ? (
              <Loader2 data-icon="inline-start" className="animate-spin" aria-hidden="true" />
            ) : (
              <Check data-icon="inline-start" aria-hidden="true" />
            )}
            {submitLabel}
          </Button>
        ) : (
          <Button
            type="button"
            disabled={
              disabled ||
              (step === 0 && (!nameComplete || !clickComplete)) ||
              (step === 1 && !stateComplete)
            }
            onClick={() => setStep((current) => Math.min(2, current + 1))}
          >
            Next
            <ChevronRight data-icon="inline-end" aria-hidden="true" />
          </Button>
        )}
      </DialogFooter>
    </div>
  );
}
