import { Toaster as Sonner, type ToasterProps } from "sonner";

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
 */
export function Toaster(props: ToasterProps) {
  return (
    <Sonner
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
