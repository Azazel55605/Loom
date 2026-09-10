import { GENERIC_ICONS } from "@loom/ui-kit/lib/generic-icons";

/**
 * Every icon a reference string can name, across all four sources, as plain
 * data — no components, no markup.
 *
 * ## Why metadata is separate from what draws it
 *
 * The picker has to search and list every icon before any of them is drawn,
 * and the resolver has to decide *whether* a reference names something before
 * it starts loading it. Both are questions about names, so they are answered
 * here, synchronously, without pulling a single icon into the bundle. What
 * actually draws an icon — the Tabler components, the vendored SVG files — is
 * loaded on demand through `icon-loaders.ts`, and `icon-catalog.test.ts` holds
 * the two halves to the same set of names.
 *
 * ## The four prefixes
 *
 * | Prefix | Source | Rendering |
 * | --- | --- | --- |
 * | `lucide:` | The curated `GENERIC_ICONS` set (lucide-react, ISC) | Line icon, `currentColor` |
 * | `brand:` | Vendored `homarr-labs/dashboard-icons` SVGs (Apache-2.0) | Full-colour logo on a neutral plate |
 * | `tabler:` | A curated subset of `@tabler/icons-react` (MIT) | Line icon, `currentColor` |
 * | `simple-icons:` | Vendored Simple Icons SVGs (CC0-1.0) | Monochrome logo in the foreground colour on a neutral plate |
 *
 * Names are kebab-case for `lucide:` and `tabler:` (each library's own catalog
 * name) and the vendored filename stem for the two SVG sources. See
 * `docs/THIRD_PARTY_ICONS.md` for provenance and licensing.
 */

export type IconSource = "lucide" | "brand" | "tabler" | "simple-icons";

export type IconCatalogEntry = {
  /** The full wire reference, e.g. `"tabler:server-2"`. */
  reference: string;
  source: IconSource;
  /** The part after the prefix. */
  name: string;
  /** Human-facing label, also what a screen reader announces in the picker. */
  label: string;
  /** Extra search terms: categories, aliases, what the thing is. */
  keywords: readonly string[];
};

export type IconCatalogSection = {
  source: IconSource;
  title: string;
  entries: readonly IconCatalogEntry[];
};

/**
 * The curated Tabler subset, by category.
 *
 * Larger than `GENERIC_ICONS` on purpose: that set was sized for a fixed grid
 * with no search, and doubles as the contract a connector author writes
 * against. This one sits behind a search box, so breadth is useful rather than
 * noise — but it is still a list, not the ~6000-icon catalog, so each entry is
 * a deliberate choice and a reference outside it falls back rather than
 * resolving to whatever Tabler happens to export.
 *
 * Names are Tabler's own kebab-case catalog names. The component each maps to
 * is in `tabler-icon-components.ts`, typed against this list so a missing or
 * extra entry is a compile error.
 */
const TABLER_CATEGORIES = {
  network: [
    ["network", "Network"],
    ["router", "Router"],
    ["wifi", "Wi-Fi"],
    ["access-point", "Access point"],
    ["antenna", "Antenna"],
    ["world", "Internet"],
    ["world-www", "Website"],
    ["sitemap", "Topology"],
    ["topology-star-3", "Mesh"],
    ["plug-connected", "Connection"],
    ["bluetooth", "Bluetooth"],
    ["rss", "Feed"],
  ],
  storage: [
    ["database", "Database"],
    ["database-cog", "Database admin"],
    ["server", "Server"],
    ["server-2", "Server rack"],
    ["server-cog", "Server admin"],
    ["server-bolt", "Server power"],
    ["device-floppy", "Save"],
    ["disc", "Disc"],
    ["folder", "Folder"],
    ["folder-open", "Open folder"],
    ["archive", "Archive"],
    ["files", "Files"],
    ["cloud", "Cloud"],
    ["cloud-computing", "Cloud compute"],
    ["download", "Download"],
    ["upload", "Upload"],
    ["usb", "USB"],
  ],
  hardware: [
    ["cpu", "CPU"],
    ["cpu-2", "Processor"],
    ["device-desktop", "Desktop"],
    ["device-laptop", "Laptop"],
    ["device-mobile", "Phone"],
    ["device-tv", "TV"],
    ["device-gamepad-2", "Gaming"],
    ["printer", "Printer"],
    ["container", "Container"],
    ["box", "Box"],
    ["package", "Package"],
  ],
  security: [
    ["shield", "Shield"],
    ["shield-lock", "Protected"],
    ["lock", "Lock"],
    ["key", "Key"],
    ["fingerprint", "Identity"],
    ["wall", "Firewall"],
  ],
  media: [
    ["movie", "Movies"],
    ["video", "Video"],
    ["music", "Music"],
    ["photo", "Photos"],
    ["headphones", "Audio"],
    ["microphone", "Microphone"],
    ["camera", "Camera"],
    ["broadcast", "Broadcast"],
  ],
  home: [
    ["home", "Home"],
    ["smart-home", "Smart home"],
    ["bulb", "Light"],
    ["thermometer", "Thermostat"],
    ["droplet", "Water"],
    ["sun", "Solar"],
    ["plant", "Garden"],
    ["robot", "Automation"],
    ["bolt", "Energy"],
    ["battery-charging", "Battery"],
    ["power", "Power"],
  ],
  monitoring: [
    ["activity", "Activity"],
    ["activity-heartbeat", "Uptime"],
    ["chart-bar", "Bar chart"],
    ["chart-line", "Line chart"],
    ["gauge", "Gauge"],
    ["dashboard", "Dashboard"],
    ["heartbeat", "Health"],
    ["radar", "Scanner"],
    ["alert-triangle", "Alert"],
    ["bell", "Notifications"],
    ["clock", "Schedule"],
  ],
  development: [
    ["terminal-2", "Console"],
    ["code", "Code"],
    ["api", "API"],
    ["webhook", "Webhook"],
    ["git-branch", "Git"],
    ["bug", "Bug"],
    ["settings", "Settings"],
    ["adjustments", "Adjustments"],
    ["tool", "Tool"],
  ],
  communication: [
    ["mail", "Mail"],
    ["message", "Chat"],
    ["users", "Users"],
    ["link", "Link"],
    ["calendar", "Calendar"],
  ],
} as const satisfies Record<string, readonly (readonly [string, string])[]>;

type TablerCategories = typeof TABLER_CATEGORIES;
/** Every curated Tabler name, as a type, so the component map can be checked
 *  against it exhaustively. */
export type TablerIconName = TablerCategories[keyof TablerCategories][number][0];

const TABLER_ENTRIES: readonly IconCatalogEntry[] = Object.entries(TABLER_CATEGORIES).flatMap(
  ([category, icons]) =>
    icons.map(([name, label]) => ({
      reference: `tabler:${name}`,
      source: "tabler" as const,
      name,
      label,
      keywords: [category, ...name.split("-")],
    })),
);

/**
 * The vendored Simple Icons, keyed by filename stem (Simple Icons' own slug).
 *
 * Chosen for what people self-host and have no dashboard-icons file for here.
 * Brands already vendored under `brand:` (Docker, Pi-hole, TrueNAS, UniFi) are
 * deliberately absent so one product never has two competing marks, and every
 * entry was checked to carry no per-icon `license` field in the upstream
 * data file — Simple Icons is CC0 as a collection, but some individual icons
 * are not, and those were excluded. See `docs/THIRD_PARTY_ICONS.md`.
 */
const SIMPLE_ICONS: readonly (readonly [slug: string, title: string, keywords: string])[] = [
  ["jellyfin", "Jellyfin", "media server streaming"],
  ["plex", "Plex", "media server streaming"],
  ["emby", "Emby", "media server streaming"],
  ["kodi", "Kodi", "media player"],
  ["sonarr", "Sonarr", "media tv arr"],
  ["radarr", "Radarr", "media movies arr"],
  ["audiobookshelf", "Audiobookshelf", "media books audio"],
  ["qbittorrent", "qBittorrent", "download torrent"],
  ["transmission", "Transmission", "download torrent"],
  ["nextcloud", "Nextcloud", "files cloud storage"],
  ["syncthing", "Syncthing", "files sync storage"],
  ["immich", "Immich", "photos backup"],
  ["paperlessngx", "Paperless-ngx", "documents"],
  ["minio", "MinIO", "storage object s3"],
  ["synology", "Synology", "nas storage hardware"],
  ["unraid", "Unraid", "nas storage os"],
  ["openmediavault", "openmediavault", "nas storage os"],
  ["homeassistant", "Home Assistant", "home automation smart"],
  ["nodered", "Node-RED", "automation flows"],
  ["mqtt", "MQTT", "automation messaging broker"],
  ["zigbee2mqtt", "Zigbee2MQTT", "home automation zigbee"],
  ["esphome", "ESPHome", "home automation firmware"],
  ["frigate", "Frigate", "camera nvr surveillance"],
  ["octoprint", "OctoPrint", "3d printer"],
  ["grafana", "Grafana", "monitoring dashboards metrics"],
  ["prometheus", "Prometheus", "monitoring metrics"],
  ["influxdb", "InfluxDB", "monitoring database metrics"],
  ["uptimekuma", "Uptime Kuma", "monitoring status uptime"],
  ["netdata", "Netdata", "monitoring metrics"],
  ["portainer", "Portainer", "containers management"],
  ["proxmox", "Proxmox", "virtualization hypervisor"],
  ["kubernetes", "Kubernetes", "containers orchestration k8s"],
  ["podman", "Podman", "containers"],
  ["traefikproxy", "Traefik Proxy", "reverse proxy ingress"],
  ["nginx", "NGINX", "web server reverse proxy"],
  ["nginxproxymanager", "Nginx Proxy Manager", "reverse proxy"],
  ["caddy", "Caddy", "web server reverse proxy"],
  ["cloudflare", "Cloudflare", "dns cdn tunnel network"],
  ["tailscale", "Tailscale", "vpn network mesh"],
  ["wireguard", "WireGuard", "vpn network"],
  ["openvpn", "OpenVPN", "vpn network"],
  ["zerotier", "ZeroTier", "vpn network"],
  ["adguard", "AdGuard", "dns ad blocking network"],
  ["pfsense", "pfSense", "firewall router network"],
  ["opnsense", "OPNsense", "firewall router network"],
  ["openwrt", "OpenWrt", "router firmware network"],
  ["mikrotik", "MikroTik", "router network hardware"],
  ["vaultwarden", "Vaultwarden", "passwords security"],
  ["bitwarden", "Bitwarden", "passwords security"],
  ["gitea", "Gitea", "git code development"],
  ["gitlab", "GitLab", "git code development ci"],
  ["postgresql", "PostgreSQL", "database sql"],
  ["mariadb", "MariaDB", "database sql mysql"],
  ["redis", "Redis", "database cache"],
  ["raspberrypi", "Raspberry Pi", "hardware sbc"],
];

/** The vendored dashboard-icons marks. Keys are the filename stems under
 *  `assets/icons/brand/`; see `BRAND_ICON_LOADERS`. */
const BRAND_ICONS: readonly (readonly [key: string, label: string])[] = [
  ["docker", "Docker"],
  ["pihole", "Pi-hole"],
  ["truenas", "TrueNAS"],
  ["unifi", "UniFi"],
];

export const ICON_SECTIONS: readonly IconCatalogSection[] = [
  {
    source: "lucide",
    title: "Default",
    entries: GENERIC_ICONS.map((icon) => ({
      reference: `lucide:${icon.name}`,
      source: "lucide" as const,
      name: icon.name,
      label: icon.label,
      keywords: icon.name.split("-"),
    })),
  },
  {
    source: "brand",
    title: "Connectors",
    entries: BRAND_ICONS.map(([name, label]) => ({
      reference: `brand:${name}`,
      source: "brand" as const,
      name,
      label,
      keywords: ["brand", "logo", "connector"],
    })),
  },
  { source: "tabler", title: "Tabler", entries: TABLER_ENTRIES },
  {
    source: "simple-icons",
    title: "Simple Icons",
    entries: SIMPLE_ICONS.map(([name, label, keywords]) => ({
      reference: `simple-icons:${name}`,
      source: "simple-icons" as const,
      name,
      label,
      keywords: ["brand", "logo", ...keywords.split(" ")],
    })),
  },
];

const BY_REFERENCE = new Map(
  ICON_SECTIONS.flatMap((section) => section.entries).map((entry) => [entry.reference, entry]),
);

/** Looks a full reference up in the catalog. `undefined` for anything this
 *  build does not have, including unprefixed legacy strings. */
export function iconCatalogEntry(reference: string | null | undefined): IconCatalogEntry | undefined {
  return reference == null ? undefined : BY_REFERENCE.get(reference);
}

/** Used when nothing else resolves. Must be a member of `GENERIC_ICONS`. */
export const FALLBACK_ICON_REFERENCE = "lucide:server";

/**
 * The first candidate that names something this build has, else the hard
 * fallback. Never returns nothing: an icon is not the place a missing asset
 * gets reported.
 */
export function resolveIconReference(
  candidates: readonly (string | null | undefined)[],
): IconCatalogEntry {
  for (const candidate of candidates) {
    const entry = iconCatalogEntry(candidate);
    if (entry !== undefined) return entry;
  }
  return BY_REFERENCE.get(FALLBACK_ICON_REFERENCE)!;
}

/**
 * Filters every section at once. Each whitespace-separated term must appear
 * somewhere in an entry's label, name, or keywords, so `"media server"` finds
 * Jellyfin and `"lock"` finds both Tabler's lock and its shield-lock. Sections
 * left with no matches are dropped rather than returned empty — the picker
 * hides a category with nothing to show.
 */
export function searchIconSections(query: string): IconCatalogSection[] {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (terms.length === 0) return [...ICON_SECTIONS];

  return ICON_SECTIONS.map((section) => ({
    ...section,
    entries: section.entries.filter((entry) => {
      const haystack = [entry.label, entry.name, ...entry.keywords].join(" ").toLowerCase();
      return terms.every((term) => haystack.includes(term));
    }),
  })).filter((section) => section.entries.length > 0);
}
