import { AppIcon } from "@loom/ui-kit/components/AppIcon";

/**
 * Draws the icon for a connector: a thin wrapper over `AppIcon` that names the
 * two candidates the way connector call sites think about them.
 *
 * Resolution order is `iconOverride` (the user's per-instance choice, so two
 * Docker hosts on one dashboard can be told apart), then `typeIcon` (what the
 * connector type declares), then `AppIcon`'s hard `lucide:server` fallback. An
 * override naming something this build lacks falls back to the type's own
 * icon rather than straight to the default. Everything else — the four
 * prefixes, lazy loading, brand plates — is `AppIcon`'s.
 */
export function ConnectorIcon({
  typeIcon,
  iconOverride,
  size = 20,
  className,
}: {
  /** `metadata.icon` — what the connector *type* declares. */
  typeIcon: string | null;
  /** `iconOverride` — the user's choice for this instance, if any. */
  iconOverride: string | null;
  /** Edge length in pixels. The icon is square. */
  size?: number;
  className?: string;
}) {
  return <AppIcon icon={iconOverride} fallback={typeIcon} size={size} className={className} />;
}
