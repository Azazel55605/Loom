import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

const badgeVariants = cva(
  "inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
  {
    variants: {
      variant: {
        default: "border-transparent bg-primary text-primary-foreground shadow",
        secondary: "border-transparent bg-muted text-muted-foreground",
        destructive:
          "border-transparent bg-destructive text-destructive-foreground shadow",
        outline: "text-foreground",
        // Connector health. Added as CVA variants rather than a new component
        // or ad-hoc classes at the call site, per step (b) of the sourcing rule
        // in docs/UI_GUIDELINES.md. The colours resolve from the --status-*
        // tokens in index.css; nothing here is a literal.
        healthy: "border-transparent bg-status-healthy text-status-foreground shadow",
        degraded:
          "border-transparent bg-status-degraded text-status-foreground shadow",
        down: "border-transparent bg-status-down text-status-foreground shadow",
        unknown: "border-transparent bg-status-unknown text-status-foreground",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

export interface BadgeProps
  extends React.HTMLAttributes<HTMLDivElement>,
    VariantProps<typeof badgeVariants> {}

function Badge({ className, variant, ...props }: BadgeProps) {
  return <div className={cn(badgeVariants({ variant }), className)} {...props} />;
}

export { Badge, badgeVariants };
