import { Toaster as Sonner, type ToasterProps } from "sonner";

import { useAppearance } from "@/components/AccentThemeProvider";

/**
 * Toast host, mounted once at the root.
 *
 * Sonner is shadcn/ui's current toast component — it replaced the older
 * `Toast`/`useToast` pair upstream, so this is step (a) of the sourcing rule in
 * docs/UI_GUIDELINES.md, not a bespoke choice.
 *
 * The toast surface is styled from the same tokens as every other elevated
 * surface rather than Sonner's own palette, so the accent colour and the
 * reduced-transparency fallback reach it like anything else. Sonner respects
 * `prefers-reduced-motion` itself, and the global rule in index.css covers the
 * rest.
 *
 * The resolved palette is still handed to Sonner explicitly, matching what
 * shadcn's own wrapper does with `next-themes`. Our classes set the toast's
 * background, but Sonner also ships defaults keyed off its `theme` prop at the
 * same specificity — leaving it on `light` risks a white toast carrying
 * light-on-white text the moment the two rules are ordered differently.
 */
export function Toaster(props: ToasterProps) {
  const { resolvedTheme } = useAppearance();

  return (
    <Sonner
      theme={resolvedTheme}
      className="toaster group"
      toastOptions={{
        classNames: {
          toast:
            "surface-elevated group toast border border-border text-foreground rounded-lg shadow-lg",
          description: "text-muted-foreground",
          actionButton: "bg-primary text-primary-foreground",
          cancelButton: "bg-muted text-muted-foreground",
        },
      }}
      {...props}
    />
  );
}
