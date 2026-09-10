# Third-party icons

Loom's icon system has four sources, each named by a reference prefix. This
document records where each comes from, what its license requires, and exactly
which files or icons are covered.

| Prefix | Source | License | How it reaches the build |
| --- | --- | --- | --- |
| `lucide:` | [lucide-react](https://lucide.dev) | ISC | npm dependency, curated list — [below](#the-generic-set) |
| `brand:` | [homarr-labs/dashboard-icons](https://github.com/homarr-labs/dashboard-icons) | Apache-2.0 | Vendored SVGs — [below](#source) |
| `tabler:` | [Tabler Icons](https://tabler.io/icons) (`@tabler/icons-react`) | MIT | npm dependency, curated list — [below](#tabler-icons) |
| `simple-icons:` | [Simple Icons](https://simpleicons.org) | CC0-1.0 (collection; see caveat) | Vendored SVGs — [below](#simple-icons) |

The names every source offers are listed in
`packages/ui-kit/src/lib/icon-catalog.ts`; `icon-catalog.test.ts` fails if a
vendored file, a loader, or a Tabler component drifts from that list.

## Source

The `brand:` set.

| | |
| --- | --- |
| Project | [homarr-labs/dashboard-icons](https://github.com/homarr-labs/dashboard-icons) (formerly `walkxcode/dashboard-icons`) |
| License | **Apache License 2.0** — [`LICENSE`](https://github.com/homarr-labs/dashboard-icons/blob/main/LICENSE) |
| Copyright | Copyright (c) 2024 Bjorn Lammers, Meier Lukas, Thomas Camlong and Homarr Labs |
| Vendored into | `packages/ui-kit/src/assets/icons/brand/` |
| License copy | `packages/ui-kit/src/assets/icons/brand/LICENSE`, verbatim from upstream |

> **The license is Apache-2.0, not CC0-1.0.** The CC0-1.0 reference that
> circulates about this project belongs to *Simple Icons*, a different
> collection `dashboard-icons` links to from its README for monochrome logos.
> The repository itself — the one these SVGs come from — is Apache-2.0, and its
> GitHub license metadata says so. This matters: Apache-2.0 carries obligations
> that CC0 does not, which is why the two files below exist rather than a bare
> SVG.

Loom itself is MIT-licensed. Including Apache-2.0 material is compatible with
that; the Apache terms continue to govern these specific files, which is what
the vendored `LICENSE` copy is for.

## What Apache-2.0 asks of us, and where it is satisfied

| Obligation (§4) | Where |
| --- | --- |
| Give recipients a copy of the license | `packages/ui-kit/src/assets/icons/brand/LICENSE` |
| Retain copyright, patent, trademark and attribution notices | The copyright line above and in that file. The SVGs carry no embedded notices of their own. |
| State significant changes to modified files | Nothing is modified. Every file below is byte-identical to upstream. |

**Do not reformat, minify, or re-optimise a vendored SVG.** Keeping them
byte-identical is what makes "nothing is modified" true, and it makes a refresh
from upstream a diff you can read rather than one you have to trust.

## Disclaimer

Reproduced from the upstream project's own README, and true of Loom's use as
well:

> All product names, trademarks, and registered trademarks are the property of
> their respective owners. Icons are used for identification purposes only and
> do not imply endorsement.

The same statement applies to every brand mark Loom ships, from both the
`brand:` and `simple-icons:` sources.

A collection license does not waive third-party trademark, patent, or
brand-guideline restrictions. Brand marks are offered in `IconPicker` so a
user can label their own tile, connector, group, or folder with the product it
represents — identification, which is what the disclaimer covers. Loom never
uses a mark to suggest a product endorses, is affiliated with, or is part of
Loom.

## Vendored icons

Only icons an implemented connector type actually references are vendored.
**This is not a bulk import of the upstream set** — it is currently over two
thousand files, almost all of which would be dead weight in a repository and in
a bundle. Adding a connector type adds its icon here; nothing else does.

| Icon | File | Referenced by | Upstream path |
| --- | --- | --- | --- |
| Docker | `docker.svg` | The `docker` connector type (`crates/connector-docker`), as `brand:docker` | [`svg/docker.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/docker.svg) |
| Pi-hole | `pihole.svg` | The `pihole` connector type (`crates/connector-pihole`), as `brand:pihole` | [`svg/pi-hole.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/pi-hole.svg) |
| TrueNAS | `truenas.svg` | The `truenas` connector type (`crates/connector-truenas`), as `brand:truenas` | [`svg/truenas.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/truenas.svg) |
| UniFi | `unifi.svg` | The `unifi-network` connector type (`crates/connector-unifi-network`), as `brand:unifi` | [`svg/unifi.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/unifi.svg) |

The debug fixture deliberately uses the `lucide:` path instead — it is not a
product and has no logo to claim.

## Adding an icon

1. Fetch the SVG from `https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/svg/<name>.svg`.
   Verify it exists rather than assuming the path; upstream has reorganised
   before. Save it unmodified as `packages/ui-kit/src/assets/icons/brand/<key>.svg`.
2. Add a line to `BRAND_ICON_LOADERS` in `packages/ui-kit/src/lib/icon-loaders.ts`
   and to `BRAND_ICONS` in `packages/ui-kit/src/lib/icon-catalog.ts`, keyed by
   that filename stem. The maps are written out by hand so the set is
   greppable and so each icon lands in its own lazily-loaded chunk.
3. Add a row to the table above.
4. Reference it from the connector's `ConnectorMetadata::icon` as `"brand:<key>"`.

## Tabler Icons

| | |
| --- | --- |
| Project | [tabler/tabler-icons](https://github.com/tabler/tabler-icons), npm package [`@tabler/icons-react`](https://www.npmjs.com/package/@tabler/icons-react) |
| License | **MIT** — [`LICENSE`](https://github.com/tabler/tabler-icons/blob/main/LICENSE) |
| Copyright | Copyright (c) 2020-2026 Paweł Kuna |
| Version | `3.46.0`, pinned exactly in `packages/ui-kit/package.json` |
| Used from | `packages/ui-kit/src/lib/tabler-icon-components.ts` |

Nothing is vendored: the icons are ordinary named imports from the npm
package, which carries its own `LICENSE` and an `@license` header in every
module, satisfying MIT's notice requirement. The package is `sideEffects: false`
with one ES module per icon, so the bundle contains only the ~90 icons named in
`tabler-icon-components.ts` (in one lazily-loaded chunk), not the ~6000-icon
library. The curated names, grouped by category (network, storage, hardware,
security, media, home, monitoring, development, communication), are
`TABLER_CATEGORIES` in `icon-catalog.ts`.

To add one: find its kebab-case name on [tabler.io/icons](https://tabler.io/icons),
add it to a category in `TABLER_CATEGORIES`, and add the matching `Icon…`
import to `TABLER_COMPONENTS` — the map is typed against the catalog, so
forgetting either half is a compile error.

## Simple Icons

| | |
| --- | --- |
| Project | [simple-icons/simple-icons](https://github.com/simple-icons/simple-icons) |
| License | **CC0-1.0** for the collection — [`LICENSE.md`](https://github.com/simple-icons/simple-icons/blob/develop/LICENSE.md) |
| Vendored from | The `simple-icons@16.30.0` npm package, `icons/<slug>.svg` (identical to the upstream repository's `icons/` directory at that release) |
| Vendored into | `packages/ui-kit/src/assets/icons/simple-icons/` |
| License copy | `packages/ui-kit/src/assets/icons/simple-icons/LICENSE.md`, verbatim from upstream |

The files are **not** an npm dependency — only the SVGs are copied, the same
pattern as the `brand:` set. CC0 imposes no attribution or notice obligation;
the license copy and this record are kept anyway so provenance is auditable.
The same rule applies: **do not reformat, minify, or re-optimise** a vendored
SVG. Upstream ships each mark as a single uncoloured path; Loom colours it at
render time with CSS (`fill: currentColor`), not by editing the file.

> **Not every Simple Icon is CC0.** Upstream's
> [`DISCLAIMER.md`](https://github.com/simple-icons/simple-icons/blob/develop/DISCLAIMER.md)
> is explicit that the CC0 collection license does not imply every icon is
> CC0: individual icons may carry their own license, recorded in the `license`
> field of the package's `data/simple-icons.json`. **Every icon below was
> checked against that file at `16.30.0` and has no `license` entry.** Icons
> that did — Deluge (GPL-3.0-only), Authelia (Apache-2.0), Keycloak (custom
> trademark terms), Forgejo (CC-BY-SA-4.0), Jenkins (CC-BY-SA-3.0), Debian
> (CC-BY-SA-3.0) — were deliberately left out. Re-check the field for any icon
> you add or refresh; upstream notes the data can change.

Upstream also asks users to respect each brand's own guidelines; where
Simple Icons records a guidelines link it is marked **G** below. Loom renders
these marks unaltered in shape, monochrome, at icon size, for identification
only — see the [disclaimer](#disclaimer), which applies to these marks too.

Brands already vendored under `brand:` (Docker, Pi-hole, TrueNAS, and UniFi —
Simple Icons' `ubiquiti`) are deliberately **not** duplicated here, so one
product never has two competing marks.

| Category | Icons (filename stem = slug) |
| --- | --- |
| Media | `jellyfin` (G), `plex` (G), `emby`, `kodi`, `sonarr`, `radarr`, `audiobookshelf` |
| Downloads | `qbittorrent`, `transmission` |
| Files & storage | `nextcloud` (G), `syncthing`, `immich`, `paperlessngx`, `minio` (G), `synology` (G), `unraid`, `openmediavault` |
| Home automation | `homeassistant` (G), `nodered`, `mqtt`, `zigbee2mqtt`, `esphome`, `frigate`, `octoprint` |
| Monitoring | `grafana`, `prometheus`, `influxdb` (G), `uptimekuma`, `netdata` |
| Containers & virtualisation | `portainer`, `proxmox` (G), `kubernetes`, `podman` |
| Proxy & web | `traefikproxy`, `nginx` (G), `nginxproxymanager`, `caddy`, `cloudflare` (G) |
| Network & VPN | `tailscale`, `wireguard` (G), `openvpn` (G), `zerotier` (G), `adguard`, `pfsense`, `opnsense`, `openwrt` (G), `mikrotik` |
| Security | `vaultwarden`, `bitwarden` (G) |
| Development | `gitea`, `gitlab` (G) |
| Databases | `postgresql` (G), `mariadb` (G), `redis` (G) |
| Hardware | `raspberrypi` (G) |

To add one: confirm the slug exists and has **no** `license` field in
`data/simple-icons.json` for the version you copy from, copy
`icons/<slug>.svg` unmodified into the directory above, add it to
`SIMPLE_ICON_LOADERS` (`icon-loaders.ts`) and `SIMPLE_ICONS`
(`icon-catalog.ts`), and add it to the table above.

## The generic set

The `lucide:` source is `GENERIC_ICONS` in
`packages/ui-kit/src/lib/generic-icons.ts` — a curated twenty from
[lucide-react](https://lucide.dev), which is ISC-licensed and already a direct
dependency of the ui-kit. Nothing is vendored for those; they are ordinary
component imports. They are what a `"lucide:<name>"` reference resolves against,
the set connector authors write against, and the "Default" section of
`IconPicker`.
