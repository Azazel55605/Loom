import {
  Activity,
  Bug,
  Camera,
  Cloud,
  Container,
  Cpu,
  Database,
  Gauge,
  HardDrive,
  Home,
  Network,
  Router,
  Server,
  Shield,
  Terminal,
  Wifi,
  type LucideIcon,
} from "lucide-react";

/**
 * The generic icons a connector may reference and a user may choose from.
 *
 * ## Why a curated list and not all of lucide
 *
 * lucide-react exports over a thousand icons. Exposing the catalog would turn
 * the icon picker into a search problem, and — more importantly — it would make
 * every `lucide:<name>` reference a connector could write valid, so there would
 * be nothing to check and no honest fallback path. A fixed list is a contract:
 * a connector author can read it, and a reference outside it resolves to the
 * default rather than to whatever happened to be exported.
 *
 * Chosen for what a homelab actually runs: machines, storage, networking,
 * containers, surveillance, and the two the system itself needs — `server`,
 * which is the hard fallback, and `bug`, which the debug fixture declares.
 *
 * ## Keys are kebab-case
 *
 * The key is the wire name: `"lucide:hard-drive"`, never `"lucide:HardDrive"`.
 * PascalCase is a detail of this one binding's component exports, and the wire
 * format does not get to depend on it — lucide's own catalog and its
 * `dynamicIconImports` map are both kebab-case. The mapping is written out by
 * hand rather than derived, so a rename upstream is a compile error here
 * instead of an icon that silently stops resolving.
 *
 * Both the resolver (`components/ConnectorIcon.tsx`) and the picker
 * (`components/ConnectorInstanceDialog.tsx`) read this array, so adding an icon
 * is one line in one place.
 */
export type GenericIcon = {
  /** Kebab-case wire name, the part after `lucide:`. */
  name: string;
  /** Short human-facing label for the picker. */
  label: string;
  Component: LucideIcon;
};

export const GENERIC_ICONS: readonly GenericIcon[] = [
  { name: "server", label: "Server", Component: Server },
  { name: "container", label: "Container", Component: Container },
  { name: "database", label: "Database", Component: Database },
  { name: "hard-drive", label: "Storage", Component: HardDrive },
  { name: "cpu", label: "Compute", Component: Cpu },
  { name: "network", label: "Network", Component: Network },
  { name: "router", label: "Router", Component: Router },
  { name: "wifi", label: "Wireless", Component: Wifi },
  { name: "cloud", label: "Cloud", Component: Cloud },
  { name: "shield", label: "Security", Component: Shield },
  { name: "terminal", label: "Terminal", Component: Terminal },
  { name: "activity", label: "Monitoring", Component: Activity },
  { name: "gauge", label: "Metrics", Component: Gauge },
  { name: "camera", label: "Camera", Component: Camera },
  { name: "home", label: "Home", Component: Home },
  { name: "bug", label: "Debug", Component: Bug },
];

/** Lookup by wire name. `undefined` for anything outside the curated set — the
 *  caller is expected to fall back rather than to render nothing. */
export function genericIcon(name: string): GenericIcon | undefined {
  return GENERIC_ICONS.find((icon) => icon.name === name);
}
