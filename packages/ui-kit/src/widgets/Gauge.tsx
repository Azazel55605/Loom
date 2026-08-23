import { cva, type VariantProps } from "class-variance-authority";

import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import { cn } from "@loom/ui-kit/lib/utils";
import { configNumber, readNumber, type DisplayWidgetProps } from "@loom/ui-kit/widgets/types";

const gaugeRoot = cva("flex min-w-0 flex-col items-center justify-center", {
  variants: {
    size: {
      sm: "[--gauge-size:5rem]",
      md: "[--gauge-size:7rem]",
      lg: "[--gauge-size:9rem]",
    },
  },
  defaultVariants: { size: "md" },
});

export function GaugeSkeleton({ className }: { className?: string }) {
  return <div className={cn("flex flex-col items-center gap-2", className)}><Skeleton className="h-28 w-28 rounded-full" /><Skeleton className="h-3 w-20" /></div>;
}

/** Degrees of sweep. A 270° dial leaves a gap at the bottom, so the two ends of
 *  the scale are visually distinct rather than meeting in a closed ring. */
const SWEEP_DEGREES = 270;
const START_DEGREES = 135;

/** Polar-to-cartesian on a unit-ish circle, in SVG's y-down coordinate space. */
function pointOnArc(cx: number, cy: number, radius: number, degrees: number) {
  const radians = ((degrees - 90) * Math.PI) / 180;
  return {
    x: cx + radius * Math.cos(radians),
    y: cy + radius * Math.sin(radians),
  };
}

function arcPath(cx: number, cy: number, radius: number, fromDeg: number, toDeg: number): string {
  const start = pointOnArc(cx, cy, radius, fromDeg);
  const end = pointOnArc(cx, cy, radius, toDeg);
  const largeArc = Math.abs(toDeg - fromDeg) > 180 ? 1 : 0;
  return `M ${start.x} ${start.y} A ${radius} ${radius} 0 ${largeArc} 1 ${end.x} ${end.y}`;
}

/**
 * A bounded numeric reading as a radial dial.
 *
 * shadcn has no gauge, so this is a step-(c) component built the way shadcn
 * would: no Radix primitive is involved because there is nothing interactive to
 * make accessible — it is one `img`-role SVG with a text alternative — CVA
 * carries the size variants, and both arcs paint from `currentColor` and the
 * token palette rather than from literals, so accent and dark mode reach it.
 *
 * The sweep is drawn as two stroked arcs on one radius (track, then value)
 * rather than an SVG `path` per segment: one geometry, two `stroke-dasharray`
 * lengths, so the value arc cannot drift out of alignment with its track.
 */
export function GaugeWidget({
  label,
  unit,
  value,
  config,
  className,
  size,
}: DisplayWidgetProps & VariantProps<typeof gaugeRoot>) {
  const reading = readNumber(value);
  const min = configNumber(config, "min", 0);
  const max = configNumber(config, "max", 100);
  const span = max - min;

  const fraction =
    reading === null || span <= 0
      ? 0
      : Math.min(1, Math.max(0, (reading - min) / span));

  const radius = 42;
  const circumference = 2 * Math.PI * radius;
  const trackLength = (circumference * SWEEP_DEGREES) / 360;
  const text = reading === null ? "—" : Number.isInteger(reading) ? String(reading) : reading.toFixed(1);

  return (
    <div className={cn(gaugeRoot({ size }), className)}>
      <div className="relative">
      <svg
        viewBox="0 0 100 100"
        className="h-[var(--gauge-size)] w-[var(--gauge-size)] max-w-full"
        role="img"
        aria-label={`${label}: ${reading === null ? "no reading" : `${text}${unit ?? ""}`}`}
      >
        <path
          d={arcPath(50, 50, radius, START_DEGREES, START_DEGREES + SWEEP_DEGREES)}
          fill="none"
          className="stroke-muted"
          strokeWidth={8}
          strokeLinecap="round"
        />
        <path
          d={arcPath(50, 50, radius, START_DEGREES, START_DEGREES + SWEEP_DEGREES)}
          fill="none"
          className="stroke-primary transition-[stroke-dasharray]"
          strokeWidth={8}
          strokeLinecap="round"
          strokeDasharray={`${trackLength * fraction} ${circumference}`}
        />
      </svg>
        <div className="pointer-events-none absolute inset-0 grid place-items-center">
          <div className="flex flex-col items-center">
            <span className="text-xl font-semibold tabular-nums">{text}</span>
            {unit && reading !== null ? (
              <span className="text-xs text-muted-foreground">{unit}</span>
            ) : null}
          </div>
        </div>
      </div>
      <span className="mt-1 max-w-full truncate text-xs text-muted-foreground" title={label}>
        {label}
      </span>
    </div>
  );
}
