import type { TablerIcon } from "@tabler/icons-react";

type SvgLoader = () => Promise<{ default: string }>;

/**
 * How each icon source is actually fetched — the half of the icon system that
 * touches the bundler. Names and search live in `icon-catalog.ts`;
 * `icon-catalog.test.ts` checks the two agree.
 *
 * Every entry is a dynamic `import()`, so each vendored SVG lands in its own
 * lazily-loaded chunk and the Tabler subset in one: a deployment ships none of
 * the icons it is not drawing. The maps are written out rather than produced by
 * `import.meta.glob` so this file stays bundler-agnostic and the vendored set is
 * greppable — `docs/THIRD_PARTY_ICONS.md` lists exactly these files.
 *
 * `lucide:` has no loader: the curated generic set is a handful of ordinary
 * static imports, and it holds the hard fallback, which must never wait.
 */

/** Vendored `homarr-labs/dashboard-icons` SVGs, keyed by filename stem. */
export const BRAND_ICON_LOADERS: Record<string, SvgLoader> = {
  docker: () => import("../assets/icons/brand/docker.svg?raw"),
  pihole: () => import("../assets/icons/brand/pihole.svg?raw"),
  truenas: () => import("../assets/icons/brand/truenas.svg?raw"),
  unifi: () => import("../assets/icons/brand/unifi.svg?raw"),
};

/** Vendored Simple Icons SVGs, keyed by filename stem (Simple Icons' slug). */
export const SIMPLE_ICON_LOADERS: Record<string, SvgLoader> = {
  adguard: () => import("../assets/icons/simple-icons/adguard.svg?raw"),
  audiobookshelf: () => import("../assets/icons/simple-icons/audiobookshelf.svg?raw"),
  bitwarden: () => import("../assets/icons/simple-icons/bitwarden.svg?raw"),
  caddy: () => import("../assets/icons/simple-icons/caddy.svg?raw"),
  cloudflare: () => import("../assets/icons/simple-icons/cloudflare.svg?raw"),
  emby: () => import("../assets/icons/simple-icons/emby.svg?raw"),
  esphome: () => import("../assets/icons/simple-icons/esphome.svg?raw"),
  frigate: () => import("../assets/icons/simple-icons/frigate.svg?raw"),
  gitea: () => import("../assets/icons/simple-icons/gitea.svg?raw"),
  gitlab: () => import("../assets/icons/simple-icons/gitlab.svg?raw"),
  grafana: () => import("../assets/icons/simple-icons/grafana.svg?raw"),
  homeassistant: () => import("../assets/icons/simple-icons/homeassistant.svg?raw"),
  immich: () => import("../assets/icons/simple-icons/immich.svg?raw"),
  influxdb: () => import("../assets/icons/simple-icons/influxdb.svg?raw"),
  jellyfin: () => import("../assets/icons/simple-icons/jellyfin.svg?raw"),
  kodi: () => import("../assets/icons/simple-icons/kodi.svg?raw"),
  kubernetes: () => import("../assets/icons/simple-icons/kubernetes.svg?raw"),
  mariadb: () => import("../assets/icons/simple-icons/mariadb.svg?raw"),
  mikrotik: () => import("../assets/icons/simple-icons/mikrotik.svg?raw"),
  minio: () => import("../assets/icons/simple-icons/minio.svg?raw"),
  mqtt: () => import("../assets/icons/simple-icons/mqtt.svg?raw"),
  netdata: () => import("../assets/icons/simple-icons/netdata.svg?raw"),
  nextcloud: () => import("../assets/icons/simple-icons/nextcloud.svg?raw"),
  nginxproxymanager: () => import("../assets/icons/simple-icons/nginxproxymanager.svg?raw"),
  nginx: () => import("../assets/icons/simple-icons/nginx.svg?raw"),
  nodered: () => import("../assets/icons/simple-icons/nodered.svg?raw"),
  octoprint: () => import("../assets/icons/simple-icons/octoprint.svg?raw"),
  openmediavault: () => import("../assets/icons/simple-icons/openmediavault.svg?raw"),
  openvpn: () => import("../assets/icons/simple-icons/openvpn.svg?raw"),
  openwrt: () => import("../assets/icons/simple-icons/openwrt.svg?raw"),
  opnsense: () => import("../assets/icons/simple-icons/opnsense.svg?raw"),
  paperlessngx: () => import("../assets/icons/simple-icons/paperlessngx.svg?raw"),
  pfsense: () => import("../assets/icons/simple-icons/pfsense.svg?raw"),
  plex: () => import("../assets/icons/simple-icons/plex.svg?raw"),
  podman: () => import("../assets/icons/simple-icons/podman.svg?raw"),
  portainer: () => import("../assets/icons/simple-icons/portainer.svg?raw"),
  postgresql: () => import("../assets/icons/simple-icons/postgresql.svg?raw"),
  prometheus: () => import("../assets/icons/simple-icons/prometheus.svg?raw"),
  proxmox: () => import("../assets/icons/simple-icons/proxmox.svg?raw"),
  qbittorrent: () => import("../assets/icons/simple-icons/qbittorrent.svg?raw"),
  radarr: () => import("../assets/icons/simple-icons/radarr.svg?raw"),
  raspberrypi: () => import("../assets/icons/simple-icons/raspberrypi.svg?raw"),
  redis: () => import("../assets/icons/simple-icons/redis.svg?raw"),
  sonarr: () => import("../assets/icons/simple-icons/sonarr.svg?raw"),
  syncthing: () => import("../assets/icons/simple-icons/syncthing.svg?raw"),
  synology: () => import("../assets/icons/simple-icons/synology.svg?raw"),
  tailscale: () => import("../assets/icons/simple-icons/tailscale.svg?raw"),
  traefikproxy: () => import("../assets/icons/simple-icons/traefikproxy.svg?raw"),
  transmission: () => import("../assets/icons/simple-icons/transmission.svg?raw"),
  unraid: () => import("../assets/icons/simple-icons/unraid.svg?raw"),
  uptimekuma: () => import("../assets/icons/simple-icons/uptimekuma.svg?raw"),
  vaultwarden: () => import("../assets/icons/simple-icons/vaultwarden.svg?raw"),
  wireguard: () => import("../assets/icons/simple-icons/wireguard.svg?raw"),
  zerotier: () => import("../assets/icons/simple-icons/zerotier.svg?raw"),
  zigbee2mqtt: () => import("../assets/icons/simple-icons/zigbee2mqtt.svg?raw"),
};

/** The curated Tabler subset as one chunk. */
export const loadTablerComponents = (): Promise<{
  TABLER_COMPONENTS: Record<string, TablerIcon>;
}> => import("./tabler-icon-components");
