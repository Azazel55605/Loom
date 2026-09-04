# 0033 — Pi-hole v6 resource API mapping

- Status: accepted
- Date: 2026-09-03

## Context

The Pi-hole connector needs to browse and manage local allow/deny entries and
show top clients through Loom's generic resource browser. These operations must
follow Pi-hole v6's current REST contract rather than infer paths from the web
interface. Pi-hole serves API documentation matching the installed version at
`/api/docs`; the upstream `pi-hole/FTL` OpenAPI files are the maintenance-time
reference used for this decision.

## Verified API facts

- `GET /api/domains` is one combined collection, not separate allow-list and
  deny-list endpoints. Each row includes a numeric database `id`, `domain`,
  `type` (`allow` or `deny`), `kind` (`exact` or `regex`), nullable `comment`,
  `groups`, and `enabled`.
- An exact entry is created with `POST /api/domains/{type}/exact`; the JSON body
  supplies `domain` and may supply `comment`, `groups`, and `enabled` (default
  `true`).
- An item is replaced with `PUT
  /api/domains/{type}/{kind}/{domain}`. Pi-hole documents replacement semantics,
  so Loom resends the current `type`, `kind`, `comment`, `groups`, and the new
  `enabled` value when toggling instead of treating the operation as a partial
  patch.
- An item is removed with `DELETE
  /api/domains/{type}/{kind}/{domain}`, returning 204 on success.
- Current top clients come from the dedicated `GET /api/stats/top_clients`
  endpoint, not from summary or history. Its response is `clients: [{ ip, name,
  count }]` plus aggregate query counts. The optional `blocked` query parameter
  selects permitted-query (`false`, also the default) or blocked-query (`true`)
  rankings. Loom displays a non-empty resolved `name`, otherwise `ip`.
- Current top domains come from `GET /api/stats/top_domains`, returning
  `domains: [{ domain, count }]` plus aggregate query counts. It uses the same
  `blocked` selector; Loom requests `blocked=true` for its blocked-domain
  ranking.
- `GET /api/history` already returns `timestamp`, `total`, `cached`, `blocked`,
  and `forwarded` in every bucket. Query-volume and blocked-query charts are
  therefore two projections of one response, not separate API calls.
- The current web interface exposes application-password generation at
  **Settings > Web interface / API > Configure app password**. Pi-hole describes
  this separately revocable password as suitable for applications that cannot
  provide the TOTP required by 2FA.

## Decision

Publish four host-only resource kinds from `PiHoleConnector`:

- `domains`, containing exact `allow` and `deny` entries with add, toggle, and
  remove actions;
- `topClients`, a read-only permitted-query top-client table;
- `topBlockedDomains`, a read-only blocked-domain ranking; and
- `topBlockedClients`, a read-only blocked-query client ranking.

The four requested domain columns contain no `kind`, so regex rules are excluded
rather than rendered indistinguishably from exact entries. A future regex editor
must expose regex semantics explicitly and will supersede this boundary.

Domain resource ids use Pi-hole's numeric database id. Before a toggle or
remove, Loom refreshes the combined collection and resolves the id to the
current domain/type/kind tuple. Dynamic path values are appended as URL path
segments so reserved characters are percent-encoded rather than interpolated
raw.

The setup connection test is built without network I/O, authenticates inside
`test_connection()`, and returns authentication/transport failures as
`reachable: false`. After authentication it genuinely reads summary, domains,
and top clients. Pi-hole exposes no scoped application-password permission
model for these connector operations, so write capabilities are reported
available once authentication succeeds; the test never performs a write.

## Consequences

- The existing generic resource browser renders both tabs and dispatches their
  actions without Pi-hole-specific client code.
- Listing and testing are safe read-only operations. Live add/toggle/remove
  coverage remains ignored and explicitly environment-gated because it mutates
  a real Pi-hole domain list, even though it uses a unique reserved `.invalid`
  name and performs best-effort cleanup.
- An empty setup-guide template is meaningful for instruction-only setup paths.
  Shared UI omits the otherwise blank template/copy surface while retaining the
  description and live connection test.
