# Third-party icons

Loom vendors a small number of brand icons so a connector can be recognised at a
glance. This document records where they come from, what their license requires,
and exactly which files are covered.

## Source

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

A collection license does not waive third-party trademark, patent, or
brand-guideline restrictions. That is one more reason a brand icon is attached
to a connector *type* — the thing that genuinely integrates with that product —
and is not offered in the per-instance icon picker, where anyone could label
anything with anyone's mark. See `ConnectorIconPicker`.

## Vendored icons

Only icons an implemented connector type actually references are vendored.
**This is not a bulk import of the upstream set** — it is currently over two
thousand files, almost all of which would be dead weight in a repository and in
a bundle. Adding a connector type adds its icon here; nothing else does.

| Icon | File | Referenced by | Upstream path |
| --- | --- | --- | --- |
| Docker | `docker.svg` | The `docker` connector type (`crates/connector-docker`), as `brand:docker` | [`svg/docker.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/docker.svg) |
| TrueNAS | `truenas.svg` | The `truenas` connector type (`crates/connector-truenas`), as `brand:truenas` | [`svg/truenas.svg`](https://github.com/homarr-labs/dashboard-icons/blob/main/svg/truenas.svg) |

The debug fixture deliberately uses the `lucide:` path instead — it is not a
product and has no logo to claim.

## Adding an icon

1. Fetch the SVG from `https://raw.githubusercontent.com/homarr-labs/dashboard-icons/main/svg/<name>.svg`.
   Verify it exists rather than assuming the path; upstream has reorganised
   before. Save it unmodified as `packages/ui-kit/src/assets/icons/brand/<key>.svg`.
2. Add a line to `BRAND_ICONS` in `packages/ui-kit/src/components/ConnectorIcon.tsx`,
   keyed by that filename stem. The map is written out by hand so the set is
   greppable and so each icon lands in its own lazily-loaded chunk.
3. Add a row to the table above.
4. Reference it from the connector's `ConnectorMetadata::icon` as `"brand:<key>"`.

## The generic set

The other half of the icon system is `GENERIC_ICONS` in
`packages/ui-kit/src/lib/generic-icons.ts` — a curated sixteen from
[lucide-react](https://lucide.dev), which is ISC-licensed and already a direct
dependency of the ui-kit. Nothing is vendored for those; they are ordinary
component imports. They are what a `"lucide:<name>"` reference resolves against
and what the per-instance icon picker offers.
