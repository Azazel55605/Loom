/**
 * Typed client for the web-backend API.
 *
 * Every shape here mirrors `docs/API_CONTRACT.md`, which is itself derived from
 * the serde output of the Rust structs in `crates/core/src/connector/`. Field
 * names are `camelCase` throughout because every wire type carries
 * `#[serde(rename_all = "camelCase")]` — the single exception is `/health`,
 * which predates that convention and still emits `core_version`. That
 * inconsistency is documented in the contract; it is mirrored faithfully here
 * rather than papered over, so this file keeps telling the truth about what the
 * backend actually sends.
 *
 * These types are **hand-mirrored** from the Rust structs. That is fine while
 * the surface is this small, and it is checked by review against the contract
 * doc. If it starts drifting in practice — a renamed field that typechecks
 * cleanly here and fails at runtime — the answer is to generate them from Core
 * instead, via `ts-rs` or `specta`, rather than to mirror harder. Not
 * implemented now; flagged so the decision is made deliberately when the pain
 * shows up.
 *
 * Authentication is real: see `docs/adr/0008-auth-model.md`. Every
 * authenticated call goes through one wrapper, which attaches the access token
 * and transparently refreshes it once on a 401 — see `authorizedRequest`.
 */

import { TokenStore, type StoredTokens, type TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";

/** Platform-owned resolution of the backend URL or proxy prefix. */
export interface BaseUrlProvider {
  getBaseUrl(): Promise<string>;
}

/** Platform-owned HTTP transport. Browsers use `fetch`; Tauri can use its
 * native HTTP client when the webview cannot express a required TLS policy. */
export interface HttpTransport {
  fetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response>;
}

type ApiRuntime = {
  readonly baseUrlProvider: BaseUrlProvider;
  readonly httpTransport: HttpTransport;
  readonly tokenStore: TokenStore;
  baseUrl: string | null;
  initialization: Promise<void> | null;
  inFlightRefresh: Promise<StoredTokens> | null;
};

/* -------------------------------------------------------------------------- */
/* Wire types                                                                  */
/* -------------------------------------------------------------------------- */

/** Coarse verdict on a service, as `ConnectorStatus.health`. */
export type HealthState = "healthy" | "degraded" | "down" | "unknown";

/** How a service is doing, as of `lastChecked`. */
export type ConnectorStatus = {
  health: HealthState;
  /**
   * Readings nested by target key, then `DataPointDescriptor.id`.
   *
   * The empty-string target key is the host/aggregate view; every other key is
   * a sub-target id. Values follow each data point's declared `valueType` — `number` → number,
   * `string` → string, `bool` → boolean, `timeSeries` → an array of
   * `{ timestamp, value }` objects, oldest first. A connector may add keys that
   * are not data points (a version string, a queue depth) and a client that
   * does not recognise one ignores it. This is what a `WidgetBinding.display`
   * resolves against on every poll, which is why there is no separate
   * values endpoint.
   */
  details: unknown;
  /** RFC 3339, UTC, `Z`-suffixed. Part of the value so a polled reading stays
   *  honest about its own age. */
  lastChecked: string;
};

/** One operation a connector is willing to perform. */
export type ConnectorAction = {
  id: string;
  /** Addressed sub-target, or `null` for the host/aggregate view. */
  targetId: string | null;
  label: string;
  description: string | null;
  /** JSON Schema for this action's params; `{}` when it takes none. */
  paramsSchema: unknown;
  /**
   * Whether running this makes the service stop answering for a while.
   *
   * Not "is this dangerous" — the test is whether a user would be *surprised*
   * by the gap. `stop` takes the service down too, but the person who pressed
   * Stop expects that; `restart` is the case where a service vanishes and
   * comes back on its own. While one of these runs, the backend reports a
   * `pendingOperation` instead of letting the outage read as a fault.
   */
  isDisruptive: boolean;
};

/**
 * The outcome of an action the backend actually managed to run.
 *
 * `success: false` on a 200 means the service was reached and declined or
 * failed the request. An HTTP error means Loom never got a verdict at all —
 * these are different things and the UI should not collapse them.
 */
export type ActionResult = {
  success: boolean;
  message: string;
  payload: unknown | null;
};

/**
 * A disruptive action the platform is currently running against an instance.
 *
 * `actionLabel` is the connector's own word for it, ready to render — the same
 * word that is on the button, so "Performing: Restart" and the Restart button
 * cannot disagree.
 */
export type PendingOperation = {
  actionLabel: string;
  /** RFC 3339. */
  startedAt: string;
};

/** Identifying information for a connector. */
export type ConnectorMetadata = {
  /**
   * The connector **type**'s identifier (`"debug"`), not the instance's.
   * The instance's own id is the sibling `id` on the response envelope — do
   * not use this one in a URL or a resource-scoped grant.
   */
  id: string;
  name: string;
  /**
   * Icon *reference*, not a URL or image data. `null` when the connector
   * declares none.
   *
   * One of two prefixed forms: `"brand:<key>"`, resolving to an SVG vendored
   * under `packages/ui-kit/src/assets/icons/brand` (see
   * `docs/THIRD_PARTY_ICONS.md`), or `"lucide:<name>"`, resolving to a
   * kebab-case member of `GENERIC_ICONS`. Resolution and fallback are entirely
   * client-side and live in `ConnectorIcon` — the backend never validates this
   * string, because only a client knows which icons it has.
   */
  icon: string | null;
  version: string;
  /** `[width, height]` in dashboard grid units: the smallest footprint at which
   *  this connector is still readable. Used as a new placement's default size
   *  and as its `minW`/`minH` on the dashboard grid, and enforced again by the
   *  backend on create and update. */
  minSize: [number, number];
};

/** One resolved value a connector has agreed may be shown on the shell.
 *
 *  Never derived from `configSchema` — the connector author writes these out by
 *  hand, precisely so stored credentials cannot reach a dashboard. */
export type DisplayField = {
  label: string;
  value: string;
};

/** The shape of a data point's values, constraining which widgets can draw it. */
export type DataPointValueType = "number" | "string" | "bool" | "timeSeries";

/**
 * One piece of data an instance can bind to a widget.
 *
 * A descriptor, not a reading: the current value lives in `status.details` under
 * this `id`.
 */
export type DataPointDescriptor = {
  id: string;
  /** Addressed sub-target, or `null` for the host/aggregate view. */
  targetId: string | null;
  label: string;
  valueType: DataPointValueType;
  /** Display suffix (`"%"`, `"MiB"`). `null` for a dimensionless value; the
   *  value itself is never scaled. */
  unit: string | null;
};

/** Which plot a `metricChart` widget draws. */
export type ChartType = "pie" | "bar" | "line";

/**
 * How a data point is drawn. Read-only: nothing here invokes anything.
 *
 * **Not always a string.** Unit variants serialize bare; the one variant
 * carrying data is a single-key object, the same externally-tagged shape as
 * `ConnectorError`. A consumer that assumes a string throws on the chart case.
 */
export type DisplayWidgetType =
  | "statTile"
  | "progressBar"
  | { metricChart: { chartType: ChartType } }
  | "gauge"
  | "statusDot"
  | "logStream";

/**
 * How a resource table's cell should be *formatted*.
 *
 * Not `DataPointValueType`, though they overlap. A data point's type decides
 * which widget may bind to it, so it needs `timeSeries` and has no use for a
 * byte count; a table cell is the reverse. The wire value is always the raw one
 * — `bytes` is an exact byte count, `timestamp` an ISO 8601 string — and
 * scaling and localizing are the client's business, so two clients never
 * disagree about what the number meant.
 */
export type ColumnValueType =
  | "text"
  | "number"
  | "bool"
  | "timestamp"
  | "bytes"
  | "status";

/**
 * How a `status` cell should read at a glance.
 *
 * About sentiment, not colour: the client picks colours that match its theme.
 * The **connector** supplies the tone rather than the client inferring one from
 * the label, because "unused" is reclaimable disk for an image and a failure
 * for a backup job, and that vocabulary belongs to the connector.
 */
export type StatusTone = "neutral" | "positive" | "caution" | "negative";

/** The value of a `status` cell: a label and how it should read. */
export type StatusValue = { label: string; tone: StatusTone };

/** One column of a browsable resource table. */
export type ColumnDescriptor = {
  /** Machine key this column's value appears under in `ResourceItem.fields`. */
  key: string;
  label: string;
  valueType: ColumnValueType;
};

/** One row of a browsable resource table. */
export type ResourceItem = {
  /** Stable row id, passed back as `resourceId` when a row action is invoked. */
  id: string;
  /** Cell values keyed by `ColumnDescriptor.key`. A missing key is an empty
   *  cell; an unknown key is ignored. */
  fields: Record<string, unknown>;
};

/**
 * One browsable collection a connector holds — images, volumes, updates.
 *
 * Entirely descriptor-driven: a client renders the table from `columns` and
 * offers `rowActions`/`kindActions` beside it without knowing what the
 * connector is. A row action names its row through `resourceId` in the
 * submitted params, which is the caller's job to add — see
 * `ResourceKindBrowser`, which does it so no call site has to remember.
 */
export type ResourceKindDescriptor = {
  kind: string;
  label: string;
  columns: ColumnDescriptor[];
  /** Actions on one row. The caller adds `resourceId`. */
  rowActions: ConnectorAction[];
  /** Actions on the collection as a whole. No `resourceId`. */
  kindActions: ConnectorAction[];
  /**
   * A `ColumnDescriptor.key` whose value rows should be gathered under.
   *
   * A hint: the rows are the same rows either way, and rendering a flat table
   * is a correct reading of a grouped kind. Docker's images use it — twenty
   * rows of which four are `nginx` is a list you have to read, the same twenty
   * under seven repository headings is one you can scan.
   */
  groupByKey: string | null;
  /** Whether this kind means anything at the host, at one sub-target, or
   *  both. */
  applicableTarget: ApplicableTarget;
  /**
   * Values describing each **group** as a whole, shown on the group heading and
   * never as a row cell. Empty unless `groupByKey` is set.
   *
   * Each descriptor's `key` names a field every row of a group carries with the
   * same value, so a client reads it off any row rather than aggregating.
   * Deliberately not client-side aggregation: Docker lists one row per *tag*,
   * so summing a size column across three tags of one image would report three
   * times the disk that exists. Only the connector knows which rows share a
   * thing.
   */
  groupSummary: ColumnDescriptor[];
};

/**
 * Where a browsable kind is worth showing.
 *
 * Declared by the connector rather than inferred from an empty listing, which
 * cannot tell "this does not apply here" from "there are none right now" — the
 * difference between a tab that is empty today and one that will never fill.
 */
export type ApplicableTarget = "hostOnly" | "targetOnly" | "any";

/**
 * Whether a kind belongs in a view of `targetId`.
 *
 * `null` is the host view. An unrecognised value is shown rather than hidden: a
 * newer backend inventing a fourth case must not make a table disappear from an
 * older client with no explanation.
 */
export function appliesToTarget(
  descriptor: ResourceKindDescriptor,
  targetId: string | null | undefined,
): boolean {
  switch (descriptor.applicableTarget) {
    case "hostOnly":
      return targetId === null || targetId === undefined;
    case "targetOnly":
      return targetId !== null && targetId !== undefined;
    default:
      return true;
  }
}

/** What the update scheduler last found for one target. */
export type UpdateStatus = {
  available: boolean;
  /** What the newer thing is called in the managed system's own terms — a
   *  digest, a tag, a version. Opaque here. */
  latestRef: string | null;
  /** When this was established. Hours old by design, which is why it is shown
   *  rather than implied. */
  lastChecked: string;
};

/** How an action is offered. Every variant invokes `executeConnectorAction`. */
export type ActionWidgetType =
  | "button"
  | "toggle"
  | "slider"
  | "textField"
  | "selector";

/**
 * One widget and the thing it is wired to.
 *
 * Externally tagged — a single-key object, `{ display: … }` or `{ action: … }`
 * — because the two kinds bind to different identifier spaces: a display widget
 * reads a `DataPointDescriptor.id` out of `status.details`, a control widget
 * invokes a `ConnectorAction.id`. Narrow on the key, not on the widget type.
 */
export type WidgetBinding =
  | {
      display: {
        /** A `DataPointDescriptor.id`; its current value is `status.details`
         *  under this same key. */
        dataPointId: string;
        widgetType: DisplayWidgetType;
        /** Widget-specific extras (`min`/`max`). Always an object. */
        config: unknown;
      };
    }
  | {
      action: {
        /** A `ConnectorAction.id`, as passed to `executeConnectorAction`. */
        actionId: string;
        widgetType: ActionWidgetType;
        /** Widget-specific extras (`options`, `min`/`max`/`step`). Always an
         *  object. */
        config: unknown;
      };
    };

/**
 * The widget arrangement a connector ships with.
 *
 * A starting point for a placement's bindings, not a constraint: it is what a
 * user gets when they place the connector without configuring anything, and it
 * is theirs to edit afterwards.
 */
export type WidgetLayout = {
  bindings: WidgetBinding[];
};

/**
 * Externally tagged `ConnectorError`: exactly one key, naming the variant.
 *
 * `internal` is a newtype variant, so its value is a bare string rather than an
 * object — the one asymmetry in the enum.
 */
export type ConnectorError =
  | { unreachable: { reason: string } }
  | { authFailed: { reason: string } }
  | { invalidAction: { actionId: string } }
  | { invalidParams: { actionId: string; reason: string } }
  | { invalidConfig: { reason: string } }
  | { internal: string };

/**
 * One element of `GET /connector-types`: a kind of connector this build can
 * create instances of.
 *
 * The catalog is code-defined and identical on every deployment of a version.
 * `configSchema` is what the add-connector form is generated from, which is why
 * no client hardcodes a per-type form.
 */
export type ConnectorTypeSummary = {
  typeId: string;
  displayName: string;
  /** The type's icon reference, so a type picker can draw one before any
   *  instance exists. Same convention as `ConnectorMetadata.icon`. */
  icon: string | null;
  /**
   * JSON Schema for this type's configuration.
   *
   * **Advisory for the client, not the server's validator.** The backend hands
   * the submitted value to the connector's own factory, which can refuse a
   * configuration this schema would accept — an out-of-range number, say. So a
   * form built from this must still surface the 400 that comes back.
   */
  configSchema: unknown;
  /** Optional type-level setup help rendered by clients. */
  setupGuide: SetupGuide | null;
  /** Connector type discoverable through a configured instance, if any. */
  discoverableType: string | null;
  /** Candidate configuration field type-scoped discovery can fill. */
  discoveryTargetField: string | null;
};

export type SetupGuide = {
  variants: SetupGuideVariant[];
};

export type SetupGuideVariant = {
  id: string;
  label: string;
  description: string;
  /** Text with literal config-field or toggle placeholders. */
  template: string;
  toggles: SetupGuideToggle[];
  capabilityRequirements: CapabilityRequirement[];
};

export type SetupGuideToggle = {
  key: string;
  envVar: string;
  label: string;
  description: string;
  default: boolean;
  recommended: boolean;
};

export type CapabilityRequirement = {
  capabilityKey: string;
  label: string;
  /** Every listed toggle key must be enabled (AND-only in v1). */
  requiredToggleKeys: string[];
};

export type CapabilityStatus = {
  key: string;
  label: string;
  available: boolean;
  note: string | null;
};

export type ConnectionTestResult = {
  reachable: boolean;
  capabilities: CapabilityStatus[];
  message: string | null;
};

export type DiscoveredResource = {
  suggestedName: string;
  targetConnectorType: string;
  config: unknown;
  targetFieldValue: unknown | null;
};

export type DiscoveryResponse = {
  discoveryTargetField: string | null;
  resources: DiscoveredResource[];
};

/** One element of `GET /connector-instances`: a connector this deployment has. */
export type ConnectorInstanceSummary = {
  /** The instance's UUID. This is what goes in a URL and in a resource-scoped
   *  `connectors.control` grant — not `metadata.id`. */
  id: string;
  name: string;
  connectorType: string;
  /** RFC 3339. */
  createdAt: string;
  /** Free-form administrator labels, sorted by the backend. */
  tags: string[];
  metadata: ConnectorMetadata;
  /** The user's per-instance icon choice, in the same reference convention as
   *  `metadata.icon`. `null` means "no override" — fall back to the connector
   *  type's own icon, then to the generic default. */
  iconOverride: string | null;
  /** `null` when the connector's own status check failed. */
  status: ConnectorStatus | null;
  /** Present only when `status` is null; absent otherwise. */
  statusError?: ConnectorError;
  /**
   * A disruptive action running against this instance right now.
   *
   * A **sibling** of `status`, not a field inside it, and the two say different
   * things: `status` is what the connector reported, this is what Loom is doing
   * to it. A service mid-restart genuinely is Down, and this is the context that
   * makes that reading useful rather than alarming — so it takes visual
   * precedence over health wherever both are shown.
   */
  pendingOperation: PendingOperation | null;
  /**
   * Why this instance is Down, established by probing the network beneath it —
   * DNS, then a TCP connect. `null` unless it is Down and its connector names
   * an endpoint worth probing. Cleared as soon as it recovers.
   */
  diagnosis: string | null;
  /** Values the connector agreed may be shown. May be empty — notably for an
   *  instance the backend could not construct at startup. */
  displayFields: DisplayField[];
};

/**
 * `GET /connector-instances/{id}`: everything the list carries, plus what only
 * a detail view needs.
 *
 * `actions` is **not** in the list response. It is per-instance and can vary
 * with configuration and remote state, so it costs a request rather than
 * bloating every list entry — see `ConnectorCard`, which renders the summary
 * immediately and fills the actions in when this resolves.
 */
export type ConnectorInstanceDetail = ConnectorInstanceSummary & {
  /** The stored configuration, as written. `null` when it is unreadable. */
  config: unknown;
  /** What this instance can be asked to do right now. May be empty. */
  actions: ConnectorAction[];
  /** What this instance can bind to a widget. Resolved against a placement's
   *  `display` bindings for their labels and units. */
  dataPoints: DataPointDescriptor[];
  /** The arrangement the connector ships with. Seeds a new placement's
   *  bindings, which the user then owns — nothing re-applies it afterwards. */
  defaultLayout: WidgetLayout;
  /** Whether this instance exposes addressable views below its host view. */
  supportsSubTargets: boolean;
  /** Type id this live instance can discover, or null when unsupported. */
  discoverableType: string | null;
  /** Whether this connector can be asked if what it manages is out of date. */
  supportsUpdateChecking: boolean;
  /**
   * What the update scheduler last found, keyed by target with `""` for the
   * instance itself — the same convention `status.details` uses.
   *
   * **Empty until a check has run**, and every entry carries its own
   * `lastChecked`. Beside `status` rather than inside it: a registry reading is
   * hours old by design and a status reading is seconds old, and one object
   * carrying both would invite treating them as equally fresh.
   */
  updateStatus: Record<string, UpdateStatus>;
};

/** The caller's effective role for one dashboard. */
export type DashboardRole = "owner" | "editor" | "viewer";

/** One dashboard visible in `GET /dashboards`. */
export type DashboardSummary = {
  id: string;
  name: string;
  role: DashboardRole;
  /** Whether this dashboard is pinned for the current user. */
  pinned: boolean;
};

/** The account that owns a dashboard. */
export type DashboardOwner = {
  id: string;
  username: string;
};

/** One connector placement returned as part of dashboard detail. */
export type DashboardPlacement = {
  id: string;
  connector: ConnectorInstanceSummary;
  /** Addressed connector sub-target, or `null` for its host/aggregate view. */
  targetId: string | null;
  /**
   * The placement's **standalone** geometry.
   *
   * When `groupId` is set these four are still returned and still writable, but
   * they are **not** where the placement sits on the grid — its group's box is.
   * They are preserved so that ungrouping puts the placement back exactly where
   * it was, which is what makes grouping something a user can undo. Do not use
   * them to position a member inside its group.
   */
  positionX: number;
  positionY: number;
  width: number;
  height: number;
  widgetBindings: WidgetBinding[];
  /** RFC 3339. */
  createdAt: string;
  /** The group this placement belongs to, or `null` when it stands alone. */
  groupId: string | null;
};

/**
 * Several placements combined into one wider tile.
 *
 * Any number of members from two upward, of any connector types — nothing here
 * is pairwise. A group with fewer than two members cannot exist: the backend
 * dissolves it, returning the survivor to standalone. See
 * `docs/adr/0015-dashboard-tile-grouping.md`.
 */
export type DashboardPlacementGroup = {
  id: string;
  /** User-facing group name. */
  name: string;
  /** Assigned generic icon, or null to use the group default. */
  icon: string | null;
  /** The tile's own grid box. **This** is what the grid lays out; a member's
   *  own position and size are ignored while it is grouped. */
  positionX: number;
  positionY: number;
  width: number;
  height: number;
  /** RFC 3339. */
  createdAt: string;
  /** Two or more, in the order they should be drawn. */
  members: DashboardPlacement[];
};

/** `GET /dashboards/{id}`. */
export type DashboardDetail = {
  id: string;
  name: string;
  owner: DashboardOwner;
  role: DashboardRole;
  /** RFC 3339. */
  createdAt: string;
  /**
   * **Standalone placements only.** A placement that is a member of a group is
   * in that group's `members` and is not repeated here, so rendering
   * `placements` plus `placementGroups` draws every tile exactly once and
   * neither list has to be filtered against the other.
   */
  placements: DashboardPlacement[];
  placementGroups: DashboardPlacementGroup[];
};

/** `POST /dashboards/{id}/placement-groups` body — Editor or Owner. */
export type CreateDashboardPlacementGroupRequest = {
  /** At least two, each once, each a standalone placement on this dashboard.
   *  This order becomes the initial member order. */
  placementIds: string[];
  /** Optional for compatibility; the backend generates `Group of N`. */
  name?: string;
  /** Omit or null to use the generic group icon. */
  icon?: string | null;
  positionX: number;
  positionY: number;
  width: number;
  height: number;
};

/** `PATCH /dashboards/{id}/placement-groups/{groupId}` body. Every field is
 *  optional; an omitted one is left alone. */
export type UpdateDashboardPlacementGroupRequest = {
  name?: string;
  /** `null` clears the assigned icon. */
  icon?: string | null;
  positionX?: number;
  positionY?: number;
  width?: number;
  height?: number;
  /** Must name **exactly** the current membership — same ids, each once.
   *  Reordering cannot add or remove members; those are their own requests. */
  memberOrder?: string[];
};

export type DashboardShareTargetType = "user" | "group";
export type DashboardShareRole = "view" | "edit";

/** One owner-managed dashboard share. */
export type DashboardShare = {
  id: string;
  targetType: DashboardShareTargetType;
  targetId: string;
  role: DashboardShareRole;
  resolvedName: string;
  /** RFC 3339. */
  createdAt: string;
};

/** `POST /dashboards/{id}/shares` body. */
export type CreateDashboardShareRequest = {
  targetType: DashboardShareTargetType;
  targetId: string;
  role: DashboardShareRole;
};

/**
 * `POST /dashboards/{id}/placements` body — Editor or Owner.
 *
 * `widgetBindings` may be omitted, in which case the backend stores the
 * connector's `default_layout_for(targetId)` bindings. Width and height must
 * each meet the connector's `metadata.minSize`, and every binding is validated
 * against the selected target's descriptors — a `display` against the
 * connector's `dataPoints`, an `action` against its `actions`.
 */
export type CreateDashboardPlacementRequest = {
  connectorInstanceId: string;
  /** Omit or send `null` for the connector's host/aggregate view. */
  targetId?: string | null;
  positionX: number;
  positionY: number;
  width: number;
  height: number;
  widgetBindings?: WidgetBinding[];
};

/**
 * `PATCH /dashboards/{id}/placements/{placementId}` body — Editor or Owner.
 *
 * Every field is optional and an absent one is left alone, which is what lets a
 * drag send only the position and a binding edit send only the bindings. The
 * connector instance itself is fixed; re-pointing a placement means deleting it
 * and adding another.
 */
export type UpdateDashboardPlacementRequest = {
  positionX?: number;
  positionY?: number;
  width?: number;
  height?: number;
  /** `null` selects the host view. Existing placement UI keeps this read-only. */
  targetId?: string | null;
  widgetBindings?: WidgetBinding[];
};

/** One cheap, addressable view inside a connector instance. */
export type SubTarget = {
  id: string;
  label: string;
  /**
   * What *sort* of thing this target is, in the connector's own vocabulary —
   * Docker uses `"container"` and `"stack"`; `"target"` when a connector does
   * not distinguish.
   *
   * Deliberately a free-form string, like connector type ids and action ids:
   * the vocabulary belongs to the connector. Group or icon by it if useful, and
   * **treat an unrecognised value as an ordinary target** — a connector
   * inventing a word must not make its targets disappear from an older client.
   */
  kind: string;
};

/** `POST /connector-instances` body. */
export type CreateConnectorInstanceRequest = {
  /** A `typeId` from `GET /connector-types`. */
  connectorType: string;
  name: string;
  /** Omitting it means "no configuration", which is what an unfilled form
   *  submits. */
  config?: unknown;
};

/**
 * `PATCH /connector-instances/{id}` body. Every field is optional; an absent
 * field is left alone.
 *
 * `config` **replaces** the whole configuration rather than merging into it — a
 * connector is rebuilt from its configuration wholesale, so a partial one has
 * no coherent meaning.
 */
export type UpdateConnectorInstanceRequest = {
  name?: string;
  config?: unknown;
  /** Complete replacement set. Omit to leave tags unchanged. */
  tags?: string[];
  /**
   * Three states, and the distinction matters: **omit** the key to leave the
   * override alone, send **`null`** to clear it back to the connector type's
   * own icon, and send a **string** to set it. Because `undefined` is dropped
   * when the body is serialized, "leave it alone" and "clear it" are exactly
   * the difference between not setting the property and setting it to `null`.
   */
  iconOverride?: string | null;
};

/** `GET /setup/status` and `POST /setup` response. */
export type SetupStatus = {
  /** `false` when the instance still needs first-run setup. */
  setupComplete: boolean;
};

/**
 * `POST /setup` request.
 *
 * The stub reads and discards every value. The shape is what the real
 * implementation needs, so the wizard is built against it now.
 */
export type SetupRequest = {
  instanceName: string;
  adminUsername: string;
  adminPassword: string;
};

/**
 * One permission granted to the signed-in user.
 *
 * Scope reads as: both null means every resource of every type; a
 * `resourceType` with a null `resourceId` means every resource of that type;
 * both set means exactly that one resource.
 *
 * Useful for hiding controls the user cannot operate. That is a convenience,
 * **never** a control — the server decides what is permitted, and a client that
 * ignores this array learns nothing it could not learn by trying.
 *
 * Note the deliberate asymmetry when reading it: the server treats a *scoped*
 * grant as not satisfying a *global* check, so holding `connectors.control`
 * over one connector is not authority over connectors in general.
 */
export type PermissionGrant = {
  key: string;
  resourceType: string | null;
  resourceId: string | null;
};

/**
 * Response shared by `POST /auth/login` and `POST /auth/refresh`.
 *
 * The refresh token **rotates**: every successful refresh returns a new one and
 * revokes the one presented. A caller must persist what it receives here; a
 * client that keeps reusing its original refresh token is signed out on its
 * second refresh.
 */
export type TokenResponse = {
  accessToken: string;
  refreshToken: string;
  /**
   * RFC 3339. Refers to the **access** token, which lives 15 minutes — the
   * value to schedule a refresh against. The refresh token's own 7-day expiry
   * is not sent, because a client cannot act on it except by discovering its
   * refresh failed.
   */
  expiresAt: string;
};

/** `GET /auth/session` response for an accepted access token. */
export type SessionResponse = {
  authenticated: boolean;
  userId: string;
  username: string;
  permissions: PermissionGrant[];
};

/**
 * The `/health` response.
 *
 * `core_version` is snake_case, unlike every other field in the API. It
 * predates the camelCase convention and three clients already read it, so
 * renaming it is a deliberate breaking change rather than a tidy-up. Mirrored
 * as-is; see the "Known wart" note in docs/API_CONTRACT.md.
 */
export type Health = {
  status: string;
  core_version: string;
};

/* -------------------------------------------------------------------------- */
/* Errors                                                                      */
/* -------------------------------------------------------------------------- */

/**
 * A non-2xx response from the API.
 *
 * Carries the HTTP status so callers can branch on it — a 401 means the token
 * is gone and the app should return to the login screen, which is different
 * from any other failure and is the one case the UI must special-case.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly connectorError?: ConnectorError;
  /**
   * Whether the response carried an error body of the backend's own shape.
   *
   * This is what separates "the handler ran and rejected you" from "there is no
   * handler". A 404 from `POST /connectors/{id}/actions/{actionId}` naming an
   * unknown action arrives with `{"error": …}`; a 404 because the whole route
   * is absent from the routing table arrives with nothing. The two need
   * different explanations, and the status code alone cannot tell them apart.
   */
  readonly hasErrorBody: boolean;

  constructor(
    status: number,
    message: string,
    options: { connectorError?: ConnectorError; hasErrorBody?: boolean } = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.connectorError = options.connectorError;
    this.hasErrorBody = options.hasErrorBody ?? false;
  }

  /** True when the backend rejected our token and the session is over. */
  get isUnauthorized(): boolean {
    return this.status === 401;
  }

  /**
   * True when the caller is authenticated and still not allowed.
   *
   * Distinct from `isUnauthorized` in the one way that matters to a client:
   * refreshing the token cannot fix it. The transport must never retry a 403
   * through a refresh, or a missing grant turns into a refresh loop.
   */
  get isForbidden(): boolean {
    return this.status === 403;
  }

  /**
   * True when the request was well-formed and refused because of the state it
   * would produce — a taken username, or one of the administration safeguards.
   *
   * The message is written for a person and should be shown as-is rather than
   * replaced with a generic failure: "that would leave no active administrator"
   * tells the user what to do next, and "something went wrong" does not.
   */
  get isConflict(): boolean {
    return this.status === 409;
  }

  /**
   * True when a setup attempt lost the race — the instance was already
   * configured.
   *
   * Not an error from the caller's point of view: the end state is the one it
   * was trying to reach, so the right response is to carry on to login.
   */
  get isAlreadyComplete(): boolean {
    return this.status === 409;
  }

  /**
   * True when the route itself does not exist — a 404 with no error body of
   * ours behind it.
   *
   * In practice this means the backend on the other end does not serve this
   * API — an older build, or something else on the port.
   */
  get isMissingRoute(): boolean {
    return this.status === 404 && !this.hasErrorBody;
  }
}

/**
 * What a 404 on an auth or connector route actually means.
 *
 * A 404 with no error body is not a missing record — the route is absent from
 * the routing table, so the backend on the other end is not one that serves
 * this API. In practice: an old build, or something else answering on the port.
 *
 * Reporting the raw 404 sends people looking for a typo in a URL that is
 * correct. This says the real thing instead.
 */
export const MISSING_ROUTE_MESSAGE =
  "This backend does not serve the endpoint the app asked for. It may be an " +
  "older build, or something else may be answering on that port — see " +
  "docs/BUILD.md.";

/** The shared error body: `{ "error": string }`, plus `connectorError` when a
 *  connector produced the failure. */
type ErrorBody = {
  error?: string;
  connectorError?: ConnectorError;
};

/* -------------------------------------------------------------------------- */
/* Transport                                                                   */
/* -------------------------------------------------------------------------- */

type RequestOptions = {
  method?: string;
  /** Bearer token, attached as `Authorization` when present. */
  token?: string | null;
  /**
   * The request body.
   *
   * A `FormData` is passed through untouched; anything else is serialized as
   * JSON. The distinction matters because a multipart body's `Content-Type`
   * carries a boundary that only the browser can generate — setting the header
   * ourselves would produce one without a boundary, and the server would fail
   * to parse a body that is perfectly well-formed.
   */
  body?: unknown;
  signal?: AbortSignal;
};

/**
 * One HTTP round trip. No token handling, no retry.
 *
 * Used directly only by calls that must not trigger a refresh: the unauth
 * endpoints, and the refresh call itself — which would otherwise recurse into
 * itself the moment a refresh token is rejected.
 */
async function initializeRuntime(runtime: ApiRuntime): Promise<void> {
  if (runtime.initialization !== null) return runtime.initialization;
  runtime.initialization = Promise.all([
    runtime.baseUrlProvider.getBaseUrl(),
    runtime.tokenStore.initialize(),
  ]).then(([baseUrl]) => {
    runtime.baseUrl = baseUrl.replace(/\/$/, "");
  });
  return runtime.initialization;
}

async function request<T>(
  runtime: ApiRuntime,
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  await initializeRuntime(runtime);
  const { method = "GET", token, body, signal } = options;

  const isFormData = body instanceof FormData;

  const headers: Record<string, string> = {};
  // Deliberately not set for FormData — see `RequestOptions.body`.
  if (body !== undefined && !isFormData) headers["Content-Type"] = "application/json";
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const response = await runtime.httpTransport.fetch(`${runtime.baseUrl ?? ""}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : isFormData ? body : JSON.stringify(body),
    signal,
  });

  if (!response.ok) throw await toApiError(response);

  // 204 No Content has no body to parse, and `logout` returns one.
  if (response.status === 204) return undefined as T;

  return (await response.json()) as T;
}

/* -------------------------------------------------------------------------- */
/* Session refresh                                                             */
/* -------------------------------------------------------------------------- */

/**
 * How close to expiry counts as "refresh now".
 *
 * Covers clock skew between browser and backend plus the request's flight time:
 * a token still valid when the check runs may not be by the time it arrives.
 * Refreshing slightly early costs one extra request; refreshing slightly late
 * costs a failed request and a retry.
 */
const REFRESH_BUFFER_MS = 60_000;

/**
 * The refresh in flight, if any.
 *
 * A dashboard fires several queries at once, so several can hit a 401 together.
 * Without this they would each start their own refresh, and because the backend
 * *rotates* refresh tokens the first to land invalidates the token the others
 * are still using — turning one expired access token into a forced sign-out.
 * Everyone awaits the same promise instead.
 */
/** Raised when the session is over and only signing in again will fix it. */
export class SessionExpiredError extends Error {
  constructor(message = "Your session has expired. Please sign in again.") {
    super(message);
    this.name = "SessionExpiredError";
  }
}

/**
 * Exchanges the stored refresh token for a fresh pair, once.
 *
 * Concurrent callers share one request. On failure the session is cleared —
 * a rejected refresh token cannot be retried into working, and leaving it in
 * storage would mean retrying it on every subsequent request.
 */
function refreshSession(runtime: ApiRuntime): Promise<StoredTokens> {
  if (runtime.inFlightRefresh !== null) return runtime.inFlightRefresh;

  const stored = runtime.tokenStore.getSnapshot();
  if (stored === null) return Promise.reject(new SessionExpiredError());

  runtime.inFlightRefresh = (async () => {
    try {
      const response = await request<TokenResponse>(runtime, "/auth/refresh", {
        method: "POST",
        body: { refreshToken: stored.refreshToken },
      });

      const session: StoredTokens = {
        accessToken: response.accessToken,
        // The rotated token. Persisting the old one here would sign the user
        // out on their next refresh.
        refreshToken: response.refreshToken,
        expiresAt: response.expiresAt,
      };
      await runtime.tokenStore.setTokens(session);
      return session;
    } catch (error) {
      // A 401 means the refresh token is spent, revoked, or expired: the
      // session is genuinely over. Any other failure — backend down, network
      // out — says nothing about the token, so the session is left alone and
      // the caller sees the real error.
      if (error instanceof ApiError && error.isUnauthorized) {
        await runtime.tokenStore.clear();
        throw new SessionExpiredError();
      }
      throw error;
    } finally {
      runtime.inFlightRefresh = null;
    }
  })();

  return runtime.inFlightRefresh;
}

/**
 * An authenticated request, with the token handling every call needs.
 *
 * Refreshes proactively when the access token is within [`REFRESH_BUFFER_MS`]
 * of expiry, and reactively **exactly once** on a 401. One retry, not a loop:
 * if a freshly minted token is also rejected, retrying cannot help and would
 * turn a broken session into a request storm.
 *
 * Every authenticated endpoint goes through here, so the refresh logic lives in
 * one place rather than at each call site.
 */
async function authorizedRequest<T>(
  runtime: ApiRuntime,
  path: string,
  options: Omit<RequestOptions, "token"> & { retryOnUnauthorized?: boolean } = {},
): Promise<T> {
  const { retryOnUnauthorized = true, ...requestOptions } = options;

  await initializeRuntime(runtime);
  if (runtime.tokenStore.getSnapshot() === null) throw new SessionExpiredError();

  if (runtime.tokenStore.expiresWithin(REFRESH_BUFFER_MS)) {
    // Let a proactive refresh failure fall through to the request below: if the
    // backend is merely unreachable the access token may still be good, and
    // failing here would report the wrong problem. A genuinely dead session
    // surfaces as the 401 handled next.
    await refreshSession(runtime).catch(() => undefined);
  }

  try {
    return await request<T>(runtime, path, {
      ...requestOptions,
      token: runtime.tokenStore.getAccessToken(),
    });
  } catch (error) {
    if (!(error instanceof ApiError) || !error.isUnauthorized) throw error;

    // Not every 401 is about the token. `POST /account/password` returns one
    // for a wrong *current password*, and treating that as an expired session
    // would be actively harmful: the client would burn a refresh, retry, get
    // the same 401, and surface it as "your session expired" — signing the user
    // out because they mistyped a password. Callers whose 401 means something
    // else opt out here.
    if (!retryOnUnauthorized) throw error;

    // The token was rejected despite looking current — expired early, or signed
    // by a backend that has since been rebuilt with a new secret.
    const session = await refreshSession(runtime);
    return await request<T>(runtime, path, {
      ...requestOptions,
      token: session.accessToken,
    });
  }
}

/**
 * Turns a failed response into an `ApiError`.
 *
 * Not every error body is Loom's own shape: axum's extractors reject a bad
 * content type (415) or an unparseable body (422) before any handler runs, and
 * those come back as `text/plain`. Parsing is therefore best-effort, falling
 * back to the status line rather than throwing a second error while handling
 * the first.
 */
async function toApiError(response: Response): Promise<ApiError> {
  const fallback = `${response.status} ${response.statusText}`;
  try {
    const text = await response.text();
    if (!text) return new ApiError(response.status, fallback);

    try {
      const parsed = JSON.parse(text) as ErrorBody;
      return new ApiError(response.status, parsed.error ?? fallback, {
        connectorError: parsed.connectorError,
        // Only a body carrying our own `error` field proves a handler produced
        // this. Valid JSON alone does not.
        hasErrorBody: typeof parsed.error === "string",
      });
    } catch {
      // A plain-text rejection from the extractor layer — axum's 415 and 422
      // reject before any handler runs, but they are still deliberate answers
      // about this request rather than a missing route.
      return new ApiError(response.status, text.trim() || fallback, {
        hasErrorBody: true,
      });
    }
  } catch {
    return new ApiError(response.status, fallback);
  }
}

/* -------------------------------------------------------------------------- */
/* Endpoints                                                                   */
/* -------------------------------------------------------------------------- */

/** `GET /health` — unauthenticated, and the one route that predates auth. */
function getHealth(runtime: ApiRuntime, signal?: AbortSignal): Promise<Health> {
  return request<Health>(runtime, "/health", { signal });
}

/**
 * `GET /setup/status` — whether this instance still needs first-run setup.
 *
 * Unauthenticated, necessarily: it is asked before anyone can hold a token.
 */
function getSetupStatus(runtime: ApiRuntime, signal?: AbortSignal): Promise<SetupStatus> {
  return request<SetupStatus>(runtime, "/setup/status", { signal });
}

/**
 * `POST /setup` — completes first-run setup.
 *
 * Throws an `ApiError` with status 409 when setup was already completed, which
 * a caller should treat as success: the instance is configured, which is the
 * outcome it wanted. See `ApiError.isAlreadyComplete`.
 */
function completeSetup(runtime: ApiRuntime, 
  data: SetupRequest,
  signal?: AbortSignal,
): Promise<SetupStatus> {
  return request<SetupStatus>(runtime, "/setup", { method: "POST", body: data, signal });
}

/**
 * `POST /auth/login`.
 *
 * Unauthenticated by definition, so it does not go through
 * `authorizedRequest` — there is no session to refresh yet.
 *
 * A 401 means the credentials were rejected. The backend deliberately returns
 * one identical response for a wrong password, an unknown username, and a
 * deactivated account, so there is nothing here to distinguish between them.
 */
function login(runtime: ApiRuntime, 
  username: string,
  password: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>(runtime, "/auth/login", {
    method: "POST",
    body: { username, password },
    signal,
  });
}

/**
 * `POST /auth/refresh` — exchanges a refresh token for a rotated pair.
 *
 * Low-level and rarely what you want: it neither reads nor writes the stored
 * session. Prefer [`refreshSession`], which does both and deduplicates
 * concurrent callers. This exists for a caller holding a token from somewhere
 * other than the store.
 */
function refreshTokens(runtime: ApiRuntime, 
  refreshToken: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>(runtime, "/auth/refresh", {
    method: "POST",
    body: { refreshToken },
    signal,
  });
}

/**
 * `POST /auth/logout` — revokes one refresh token server-side.
 *
 * Returns 204 whether or not the token was live, so this resolves rather than
 * throwing for an already-revoked token. Only the presented token is revoked:
 * other devices stay signed in.
 *
 * An access token already issued stays valid until it expires — the backend
 * cannot recall it — so a caller must also discard its local session rather
 * than assume the server stopped honouring what it already handed out.
 */
function logout(runtime: ApiRuntime, refreshToken: string, signal?: AbortSignal): Promise<void> {
  return request<void>(runtime, "/auth/logout", {
    method: "POST",
    body: { refreshToken },
    signal,
  });
}

/**
 * `GET /auth/session` — who the current access token belongs to.
 *
 * Answered from the token's claims, so the permission list can lag a change by
 * up to the access token's 15-minute life.
 */
function getSession(runtime: ApiRuntime, signal?: AbortSignal): Promise<SessionResponse> {
  return authorizedRequest<SessionResponse>(runtime, "/auth/session", { signal });
}

/**
 * `GET /connector-types` — the kinds of connector this build can create.
 *
 * Requires `connectors.manage`, not `connectors.view`: this is the catalog
 * behind the add-connector form, and a caller who cannot add one has no use
 * for it.
 */
function getConnectorTypes(
  runtime: ApiRuntime,
  signal?: AbortSignal,
): Promise<ConnectorTypeSummary[]> {
  return authorizedRequest<ConnectorTypeSummary[]>(runtime, "/connector-types", { signal });
}

/** `GET /connector-instances` — every configured instance with a live status. */
function getConnectorInstances(
  runtime: ApiRuntime,
  signal?: AbortSignal,
): Promise<ConnectorInstanceSummary[]> {
  return authorizedRequest<ConnectorInstanceSummary[]>(runtime, "/connector-instances", {
    signal,
  });
}

/** `GET /connector-instances/tags` — distinct tags currently in use. */
function getConnectorTags(runtime: ApiRuntime, signal?: AbortSignal): Promise<string[]> {
  return authorizedRequest<string[]>(runtime, "/connector-instances/tags", { signal });
}

/** `GET /connector-instances/{id}` — the same entry plus actions, data points,
 *  the shipped layout, and the stored configuration. */
function getConnectorInstance(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<ConnectorInstanceDetail> {
  return authorizedRequest<ConnectorInstanceDetail>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}`,
    { signal },
  );
}

/** `GET /connector-instances/{id}/sub-targets` — live names/labels only. */
function getSubTargets(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<SubTarget[]> {
  return authorizedRequest<SubTarget[]>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}/sub-targets`,
    { signal },
  );
}

/** `GET /connector-instances/{id}/resource-kinds` — the browsable tables this
 *  instance publishes, live from its connector. */
function getResourceKinds(
  runtime: ApiRuntime,
  id: string,
  targetId?: string | null,
  signal?: AbortSignal,
): Promise<ResourceKindDescriptor[]> {
  // Which view is being looked at. Not the same question as `?targetId=` on a
  // *listing*, which scopes rows: this decides whether a kind is published at
  // all, so a kind only one sort of target has is absent elsewhere rather than
  // an empty tab that will never fill.
  const query =
    targetId === undefined || targetId === null
      ? ""
      : `?targetId=${encodeURIComponent(targetId)}`;
  return authorizedRequest<ResourceKindDescriptor[]>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}/resource-kinds${query}`,
    { signal },
  );
}

/**
 * `GET /connector-instances/{id}/resources/{kind}` — the current rows of one
 * table, optionally scoped to a sub-target.
 *
 * A `kind` this instance does not declare is a 400, not an empty list: the
 * backend validates it against the live descriptors so an empty table and a
 * typo are distinguishable.
 */
function getResourceItems(
  runtime: ApiRuntime,
  id: string,
  kind: string,
  targetId?: string | null,
  signal?: AbortSignal,
): Promise<ResourceItem[]> {
  const query =
    targetId === undefined || targetId === null
      ? ""
      : `?targetId=${encodeURIComponent(targetId)}`;
  return authorizedRequest<ResourceItem[]>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}/resources/${encodeURIComponent(kind)}${query}`,
    { signal },
  );
}

/** Run discovery through one configured connector instance. */
function discoverConnectorResources(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<DiscoveryResponse> {
  return authorizedRequest<DiscoveryResponse>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}/discover`,
    { method: "POST", signal },
  );
}

/** Run discovery through an ephemeral candidate configuration. */
function discoverForType(
  runtime: ApiRuntime,
  typeId: string,
  config: unknown,
  signal?: AbortSignal,
): Promise<DiscoveryResponse> {
  return authorizedRequest<DiscoveryResponse>(
    runtime,
    `/connector-types/${encodeURIComponent(typeId)}/discover`,
    { method: "POST", body: config, signal },
  );
}

/** Test an ephemeral candidate configuration without persisting an instance. */
function testConnectionForType(
  runtime: ApiRuntime,
  typeId: string,
  config: unknown,
  signal?: AbortSignal,
): Promise<ConnectionTestResult> {
  return authorizedRequest<ConnectionTestResult>(
    runtime,
    `/connector-types/${encodeURIComponent(typeId)}/test-connection`,
    { method: "POST", body: config, signal },
  );
}

/**
 * `POST /connector-instances` — add one.
 *
 * A 400 here is usually the *connector* refusing the configuration, and it
 * carries `connectorError` (typically `invalidConfig`) alongside a rendered
 * message. Show that message on the form: it names the field the user has to
 * fix, which a generic failure toast throws away.
 */
function createConnectorInstance(
  runtime: ApiRuntime,
  data: CreateConnectorInstanceRequest,
  signal?: AbortSignal,
): Promise<ConnectorInstanceDetail> {
  return authorizedRequest<ConnectorInstanceDetail>(runtime, "/connector-instances", {
    method: "POST",
    body: data,
    signal,
  });
}

/** `PATCH /connector-instances/{id}` — rename and/or reconfigure. Rejected
 *  configurations change nothing. */
function updateConnectorInstance(
  runtime: ApiRuntime,
  id: string,
  data: UpdateConnectorInstanceRequest,
  signal?: AbortSignal,
): Promise<ConnectorInstanceDetail> {
  return authorizedRequest<ConnectorInstanceDetail>(
    runtime,
    `/connector-instances/${encodeURIComponent(id)}`,
    { method: "PATCH", body: data, signal },
  );
}

/** `DELETE /connector-instances/{id}` — 204, no body. */
function deleteConnectorInstance(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(runtime, `/connector-instances/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/**
 * `POST /connector-instances/{id}/actions/{actionId}`.
 *
 * When `targetId` is supplied, the target-aware `{ targetId, params }`
 * envelope is sent. Omitting it preserves the direct-params form used by
 * instance-level callers.
 */
function executeConnectorAction(
  runtime: ApiRuntime,
  instanceId: string,
  actionId: string,
  params?: unknown,
  targetId?: string | null,
  signal?: AbortSignal,
): Promise<ActionResult> {
  return authorizedRequest<ActionResult>(
    runtime,
    `/connector-instances/${encodeURIComponent(instanceId)}/actions/${encodeURIComponent(actionId)}`,
    {
      method: "POST",
      body: targetId === undefined ? params : { targetId, params: params ?? null },
      signal,
    },
  );
}

/* -------------------------------------------------------------------------- */
/* Dashboards                                                                  */
/* -------------------------------------------------------------------------- */

/** `GET /dashboards` — every dashboard the current user can access. */
function getDashboards(
  runtime: ApiRuntime,
  signal?: AbortSignal,
): Promise<DashboardSummary[]> {
  return authorizedRequest<DashboardSummary[]>(runtime, "/dashboards", { signal });
}

/** `POST /dashboards` — creates a dashboard owned by the current user. */
function createDashboard(
  runtime: ApiRuntime,
  name: string,
  signal?: AbortSignal,
): Promise<DashboardSummary> {
  return authorizedRequest<DashboardSummary>(runtime, "/dashboards", {
    method: "POST",
    body: { name },
    signal,
  });
}

/** `GET /dashboards/{id}` — requires Viewer or better. */
function getDashboard(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<DashboardDetail> {
  return authorizedRequest<DashboardDetail>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}`,
    { signal },
  );
}

/** `PATCH /dashboards/{id}` — owner only. */
function renameDashboard(
  runtime: ApiRuntime,
  id: string,
  name: string,
  signal?: AbortSignal,
): Promise<DashboardDetail> {
  return authorizedRequest<DashboardDetail>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}`,
    { method: "PATCH", body: { name }, signal },
  );
}

/** `DELETE /dashboards/{id}` — owner only. */
function deleteDashboard(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(runtime, `/dashboards/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/** `POST /dashboards/{id}/pin` — pins only for the current user. */
function pinDashboard(runtime: ApiRuntime, id: string, signal?: AbortSignal): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/pin`,
    { method: "POST", signal },
  );
}

/** `DELETE /dashboards/{id}/pin` — unpins only for the current user. */
function unpinDashboard(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/pin`,
    { method: "DELETE", signal },
  );
}

/** `GET /dashboards/{id}/shares` — owner only. */
function getDashboardShares(
  runtime: ApiRuntime,
  id: string,
  signal?: AbortSignal,
): Promise<DashboardShare[]> {
  return authorizedRequest<DashboardShare[]>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/shares`,
    { signal },
  );
}

/** `POST /dashboards/{id}/shares` — owner only. */
function addDashboardShare(
  runtime: ApiRuntime,
  id: string,
  data: CreateDashboardShareRequest,
  signal?: AbortSignal,
): Promise<DashboardShare> {
  return authorizedRequest<DashboardShare>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/shares`,
    { method: "POST", body: data, signal },
  );
}

/** `DELETE /dashboards/{id}/shares/{shareId}` — owner only. */
function removeDashboardShare(
  runtime: ApiRuntime,
  id: string,
  shareId: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/shares/${encodeURIComponent(shareId)}`,
    { method: "DELETE", signal },
  );
}

/** `POST /dashboards/{id}/placements` — Editor or Owner. */
function createDashboardPlacement(
  runtime: ApiRuntime,
  id: string,
  data: CreateDashboardPlacementRequest,
  signal?: AbortSignal,
): Promise<DashboardPlacement> {
  return authorizedRequest<DashboardPlacement>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placements`,
    { method: "POST", body: data, signal },
  );
}

/** `PATCH /dashboards/{id}/placements/{placementId}` — Editor or Owner. */
function updateDashboardPlacement(
  runtime: ApiRuntime,
  id: string,
  placementId: string,
  data: UpdateDashboardPlacementRequest,
  signal?: AbortSignal,
): Promise<DashboardPlacement> {
  return authorizedRequest<DashboardPlacement>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placements/${encodeURIComponent(placementId)}`,
    { method: "PATCH", body: data, signal },
  );
}

/** `DELETE /dashboards/{id}/placements/{placementId}` — Editor or Owner. */
function deleteDashboardPlacement(
  runtime: ApiRuntime,
  id: string,
  placementId: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placements/${encodeURIComponent(placementId)}`,
    { method: "DELETE", signal },
  );
}

/** `POST /dashboards/{id}/placement-groups` — Editor or Owner. */
function createDashboardPlacementGroup(
  runtime: ApiRuntime,
  id: string,
  data: CreateDashboardPlacementGroupRequest,
  signal?: AbortSignal,
): Promise<DashboardPlacementGroup> {
  return authorizedRequest<DashboardPlacementGroup>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placement-groups`,
    { method: "POST", body: data, signal },
  );
}

/** `PATCH /dashboards/{id}/placement-groups/{groupId}` — move, resize, and/or
 *  reorder members. */
function updateDashboardPlacementGroup(
  runtime: ApiRuntime,
  id: string,
  groupId: string,
  data: UpdateDashboardPlacementGroupRequest,
  signal?: AbortSignal,
): Promise<DashboardPlacementGroup> {
  return authorizedRequest<DashboardPlacementGroup>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placement-groups/${encodeURIComponent(groupId)}`,
    { method: "PATCH", body: data, signal },
  );
}

/** `POST /dashboards/{id}/placement-groups/{groupId}/members` — appends one
 *  standalone placement after the current last member. */
function addDashboardPlacementGroupMember(
  runtime: ApiRuntime,
  id: string,
  groupId: string,
  placementId: string,
  signal?: AbortSignal,
): Promise<DashboardPlacementGroup> {
  return authorizedRequest<DashboardPlacementGroup>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placement-groups/${encodeURIComponent(groupId)}/members`,
    { method: "POST", body: { placementId }, signal },
  );
}

/**
 * `DELETE /dashboards/{id}/placement-groups/{groupId}/members/{placementId}`.
 *
 * **This can dissolve the group.** If the removal leaves fewer than two
 * members, the group is deleted and its remaining member also returns to
 * standalone — so a placement the caller did not name can change. Nothing is
 * returned for that reason; refetch the dashboard rather than patching local
 * state.
 */
function deleteDashboardPlacementGroupMember(
  runtime: ApiRuntime,
  id: string,
  groupId: string,
  placementId: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placement-groups/${encodeURIComponent(groupId)}` +
      `/members/${encodeURIComponent(placementId)}`,
    { method: "DELETE", signal },
  );
}

/** `DELETE /dashboards/{id}/placement-groups/{groupId}` — splits the tile
 *  apart. Every member returns to standalone; no placement is deleted. */
function deleteDashboardPlacementGroup(
  runtime: ApiRuntime,
  id: string,
  groupId: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(
    runtime,
    `/dashboards/${encodeURIComponent(id)}/placement-groups/${encodeURIComponent(groupId)}`,
    { method: "DELETE", signal },
  );
}

/* -------------------------------------------------------------------------- */
/* Administration                                                              */
/* -------------------------------------------------------------------------- */

/**
 * A user account, as returned by every `/users` route.
 *
 * There is **no password field, and there must never be one** — see the note in
 * docs/API_CONTRACT.md. If one ever appears in a response, that is a backend
 * bug to fix rather than a field to mirror here.
 */
export type User = {
  id: string;
  username: string;
  isActive: boolean;
  /** RFC 3339. */
  createdAt: string;
  /** The groups this user belongs to. Membership is stated wholesale, never as
   *  a delta. */
  groupIds: string[];
};

/** `POST /users` body. `groupIds` may be omitted — an account with no groups
 *  can sign in and do nothing, which is a valid state. */
export type CreateUserRequest = {
  username: string;
  password: string;
  groupIds?: string[];
};

/**
 * `PATCH /users/{id}` body. Every field is optional; an absent field is left
 * alone, and `groupIds` **replaces** membership rather than adding to it.
 *
 * Note the difference between absent and empty: omitting `groupIds` keeps the
 * current groups, sending `[]` removes them all.
 */
export type UpdateUserRequest = {
  isActive?: boolean;
  groupIds?: string[];
};

/** A group with its grants, as returned by every `/groups` route. */
export type Group = {
  id: string;
  name: string;
  description: string | null;
  /** RFC 3339. */
  createdAt: string;
  /** True for a group that cannot be deleted. Hide or disable the delete
   *  control rather than letting the user discover it through a 409. */
  isProtected: boolean;
  memberCount: number;
  permissions: PermissionGrant[];
};

/** `POST /groups` body. New groups are never protected. */
export type CreateGroupRequest = {
  name: string;
  description: string | null;
  permissions: PermissionGrant[];
};

/** `PATCH /groups/{id}` body. All fields optional; `permissions` replaces the
 *  group's grants wholesale, on the same reasoning as user membership. */
export type UpdateGroupRequest = {
  name?: string;
  description?: string | null;
  permissions?: PermissionGrant[];
};

/**
 * One entry of the permission catalog from `GET /permissions`.
 *
 * The catalog exists so a client can build a grant-assignment form without
 * hardcoding a list that falls out of date the next time a migration registers
 * a key. Treat it as the authoritative set, not `PERMISSION_KEYS`.
 */
export type PermissionCatalogEntry = {
  key: string;
  description: string;
};

/** `GET /users` — requires a global `users.manage` grant. */
function getUsers(runtime: ApiRuntime, signal?: AbortSignal): Promise<User[]> {
  return authorizedRequest<User[]>(runtime, "/users", { signal });
}

/**
 * `POST /users` — creates an account.
 *
 * 400 for an empty username, a password under 8 characters, or an unknown group
 * id; 409 when the username is taken.
 */
function createUser(runtime: ApiRuntime, 
  data: CreateUserRequest,
  signal?: AbortSignal,
): Promise<User> {
  return authorizedRequest<User>(runtime, "/users", { method: "POST", body: data, signal });
}

/**
 * `PATCH /users/{id}`.
 *
 * A 409 here is a safeguard (see docs/API_CONTRACT.md): the change
 * would leave no active administrator, or the caller is trying to modify their
 * own account. Show the backend's message — it says which.
 */
function updateUser(runtime: ApiRuntime, 
  id: string,
  data: UpdateUserRequest,
  signal?: AbortSignal,
): Promise<User> {
  return authorizedRequest<User>(runtime, `/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: data,
    signal,
  });
}

/**
 * `DELETE /users/{id}` — a hard delete, taking the user's group memberships and
 * refresh tokens with it, which also ends their sessions.
 *
 * Subject to the same safeguards as `updateUser`, with the same 409.
 */
function deleteUser(runtime: ApiRuntime, id: string, signal?: AbortSignal): Promise<void> {
  return authorizedRequest<void>(runtime, `/users/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/** `GET /groups` — requires a global `groups.manage` grant. */
function getGroups(runtime: ApiRuntime, signal?: AbortSignal): Promise<Group[]> {
  return authorizedRequest<Group[]>(runtime, "/groups", { signal });
}

/** `POST /groups`. 400 for an empty name or an unregistered permission key;
 *  409 when the name is taken. */
function createGroup(runtime: ApiRuntime, 
  data: CreateGroupRequest,
  signal?: AbortSignal,
): Promise<Group> {
  return authorizedRequest<Group>(runtime, "/groups", { method: "POST", body: data, signal });
}

/** `PATCH /groups/{id}`. A protected group may be renamed and re-granted —
 *  only deletion is refused. */
function updateGroup(runtime: ApiRuntime, 
  id: string,
  data: UpdateGroupRequest,
  signal?: AbortSignal,
): Promise<Group> {
  return authorizedRequest<Group>(runtime, `/groups/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: data,
    signal,
  });
}

/** `DELETE /groups/{id}`. 409 when the group is protected — which a client
 *  should have prevented by reading `isProtected`. */
function deleteGroup(runtime: ApiRuntime, id: string, signal?: AbortSignal): Promise<void> {
  return authorizedRequest<void>(runtime, `/groups/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/** `GET /permissions` — the catalog of registered keys. Requires
 *  `groups.manage`, since assigning grants is the only thing it is for. */
function getPermissions(runtime: ApiRuntime, 
  signal?: AbortSignal,
): Promise<PermissionCatalogEntry[]> {
  return authorizedRequest<PermissionCatalogEntry[]>(runtime, "/permissions", { signal });
}

/* -------------------------------------------------------------------------- */
/* Account (self-service)                                                      */
/* -------------------------------------------------------------------------- */

/**
 * A group the signed-in user belongs to, as reported by `GET /account`.
 *
 * Read-only context. Membership is changed through the admin `/users/{id}`
 * route, never from here — see the Account section of docs/API_CONTRACT.md.
 */
export type AccountGroup = {
  id: string;
  name: string;
};

/** The signed-in user's own profile. As everywhere, no password field. */
export type Account = {
  id: string;
  username: string;
  displayName: string | null;
  /**
   * A **relative** path like `/avatars/{uuid}.png`, or null when unset.
   *
   * Resolve it against the same base as the API: the backend cannot know the
   * origin it is reached through, so it never sends an absolute URL. Use
   * [`avatarSrc`] rather than reading this straight into an `<img src>`, or the
   * web frontend will request it on its own origin instead of through the proxy.
   */
  avatarUrl: string | null;
  createdAt: string;
  groups: AccountGroup[];
};

/**
 * `PATCH /account` body. Both optional; an absent field is left alone.
 *
 * `displayName: null` clears it — note that this is distinct from omitting the
 * field, which keeps the current value.
 */
export type UpdateAccountRequest = {
  username?: string;
  displayName?: string | null;
};

/** `POST /account/avatar` response. */
export type AvatarUploadResponse = {
  avatarUrl: string;
};

/**
 * Turns an `avatarUrl` from the API into something an `<img>` can load.
 *
 * The backend serves avatars at `/avatars/…`, and every other path in this
 * client is reached through [`API_URL`] — `/api` for the browser, an absolute
 * server URL for the desktop and mobile clients. The avatar is no different, so
 * it gets the same prefix. Reading `avatarUrl` directly would work only in the
 * one deployment where the frontend and backend share an origin *and* no proxy
 * prefix is in play.
 */
function avatarSrc(runtime: ApiRuntime, avatarUrl: string): string {
  return `${runtime.baseUrl ?? ""}${avatarUrl}`;
}

/** `GET /account` — the caller's own profile. Needs a token, no permission. */
function getAccount(runtime: ApiRuntime, signal?: AbortSignal): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account", { signal });
}

/**
 * `PATCH /account` — change your own username and/or display name.
 *
 * Throws an `ApiError` with status 409 when the username is taken by another
 * account; the check excludes your own row, so resubmitting your current
 * username is not a conflict.
 */
function updateAccount(runtime: ApiRuntime, 
  data: UpdateAccountRequest,
  signal?: AbortSignal,
): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account", {
    method: "PATCH",
    body: data,
    signal,
  });
}

/**
 * `POST /account/password`.
 *
 * A 401 here means `currentPassword` was wrong — **not** that the session
 * expired. That distinction matters to the transport as much as to the UI: see
 * the note in `authorizedRequest` about why this call opts out of the automatic
 * refresh-and-retry.
 */
function changePassword(runtime: ApiRuntime, 
  currentPassword: string,
  newPassword: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(runtime, "/account/password", {
    method: "POST",
    body: { currentPassword, newPassword },
    signal,
    retryOnUnauthorized: false,
  });
}

/**
 * `POST /account/avatar` — multipart upload of a single image.
 *
 * The backend decides what is acceptable by decoding the bytes, not by reading
 * the declared type, so a rejection here carries a real explanation (too large,
 * not a decodable image, wrong format). Show its message rather than a generic
 * failure.
 */
function uploadAvatar(runtime: ApiRuntime, 
  file: File,
  signal?: AbortSignal,
): Promise<AvatarUploadResponse> {
  const body = new FormData();
  body.append("file", file);

  return authorizedRequest<AvatarUploadResponse>(runtime, "/account/avatar", {
    method: "POST",
    body,
    signal,
  });
}

/**
 * `DELETE /account/avatar` — removes the stored file and clears the field.
 *
 * Returns the updated profile rather than an acknowledgement, and deleting when
 * there is no avatar is not an error.
 */
function deleteAvatar(runtime: ApiRuntime, signal?: AbortSignal): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account/avatar", {
    method: "DELETE",
    signal,
  });
}

/** A platform-configured instance of the complete Loom API surface. */
export type ApiClient = ReturnType<typeof createApiClient>;

/**
 * Constructs an isolated API client from platform adapters.
 *
 * Web, desktop, and mobile decide how tokens are persisted and how the backend
 * base URL is resolved. Browser `fetch` is the default transport; native clients
 * inject another one when their platform policy differs.
 */
export function createApiClient(options: {
  baseUrlProvider: BaseUrlProvider;
  tokenStorage: TokenStorageAdapter;
  httpTransport?: HttpTransport;
}) {
  const runtime: ApiRuntime = {
    baseUrlProvider: options.baseUrlProvider,
    httpTransport: options.httpTransport ?? { fetch: globalThis.fetch.bind(globalThis) },
    tokenStore: new TokenStore(options.tokenStorage),
    baseUrl: null,
    initialization: null,
    inFlightRefresh: null,
  };

  return {
    tokenStore: runtime.tokenStore,
    initialize: () => initializeRuntime(runtime),
    getBaseUrl: async () => {
      await initializeRuntime(runtime);
      return runtime.baseUrl ?? "";
    },
    refreshSession: () => refreshSession(runtime),
    getHealth: (signal?: AbortSignal) => getHealth(runtime, signal),
    getSetupStatus: (signal?: AbortSignal) => getSetupStatus(runtime, signal),
    completeSetup: (data: SetupRequest, signal?: AbortSignal) =>
      completeSetup(runtime, data, signal),
    login: (username: string, password: string, signal?: AbortSignal) =>
      login(runtime, username, password, signal),
    refreshTokens: (refreshToken: string, signal?: AbortSignal) =>
      refreshTokens(runtime, refreshToken, signal),
    logout: (refreshToken: string, signal?: AbortSignal) =>
      logout(runtime, refreshToken, signal),
    getSession: (signal?: AbortSignal) => getSession(runtime, signal),
    getConnectorTypes: (signal?: AbortSignal) => getConnectorTypes(runtime, signal),
    getConnectorInstances: (signal?: AbortSignal) => getConnectorInstances(runtime, signal),
    getConnectorTags: (signal?: AbortSignal) => getConnectorTags(runtime, signal),
    getConnectorInstance: (id: string, signal?: AbortSignal) =>
      getConnectorInstance(runtime, id, signal),
    getSubTargets: (id: string, signal?: AbortSignal) => getSubTargets(runtime, id, signal),
    getResourceKinds: (id: string, targetId?: string | null, signal?: AbortSignal) =>
      getResourceKinds(runtime, id, targetId, signal),
    getResourceItems: (
      id: string,
      kind: string,
      targetId?: string | null,
      signal?: AbortSignal,
    ) => getResourceItems(runtime, id, kind, targetId, signal),
    discoverConnectorResources: (id: string, signal?: AbortSignal) =>
      discoverConnectorResources(runtime, id, signal),
    discoverForType: (typeId: string, candidateConfig: unknown, signal?: AbortSignal) =>
      discoverForType(runtime, typeId, candidateConfig, signal),
    testConnectionForType: (typeId: string, candidateConfig: unknown, signal?: AbortSignal) =>
      testConnectionForType(runtime, typeId, candidateConfig, signal),
    createConnectorInstance: (data: CreateConnectorInstanceRequest, signal?: AbortSignal) =>
      createConnectorInstance(runtime, data, signal),
    updateConnectorInstance: (
      id: string,
      data: UpdateConnectorInstanceRequest,
      signal?: AbortSignal,
    ) => updateConnectorInstance(runtime, id, data, signal),
    deleteConnectorInstance: (id: string, signal?: AbortSignal) =>
      deleteConnectorInstance(runtime, id, signal),
    executeConnectorAction: (
      instanceId: string,
      actionId: string,
      params?: unknown,
      targetId?: string | null,
      signal?: AbortSignal,
    ) => executeConnectorAction(runtime, instanceId, actionId, params, targetId, signal),
    getDashboards: (signal?: AbortSignal) => getDashboards(runtime, signal),
    createDashboard: (name: string, signal?: AbortSignal) =>
      createDashboard(runtime, name, signal),
    getDashboard: (id: string, signal?: AbortSignal) => getDashboard(runtime, id, signal),
    renameDashboard: (id: string, name: string, signal?: AbortSignal) =>
      renameDashboard(runtime, id, name, signal),
    deleteDashboard: (id: string, signal?: AbortSignal) =>
      deleteDashboard(runtime, id, signal),
    pinDashboard: (id: string, signal?: AbortSignal) => pinDashboard(runtime, id, signal),
    unpinDashboard: (id: string, signal?: AbortSignal) =>
      unpinDashboard(runtime, id, signal),
    getDashboardShares: (id: string, signal?: AbortSignal) =>
      getDashboardShares(runtime, id, signal),
    addDashboardShare: (
      id: string,
      data: CreateDashboardShareRequest,
      signal?: AbortSignal,
    ) => addDashboardShare(runtime, id, data, signal),
    removeDashboardShare: (id: string, shareId: string, signal?: AbortSignal) =>
      removeDashboardShare(runtime, id, shareId, signal),
    createDashboardPlacement: (
      id: string,
      data: CreateDashboardPlacementRequest,
      signal?: AbortSignal,
    ) => createDashboardPlacement(runtime, id, data, signal),
    updateDashboardPlacement: (
      id: string,
      placementId: string,
      data: UpdateDashboardPlacementRequest,
      signal?: AbortSignal,
    ) => updateDashboardPlacement(runtime, id, placementId, data, signal),
    createDashboardPlacementGroup: (
      id: string,
      data: CreateDashboardPlacementGroupRequest,
      signal?: AbortSignal,
    ) => createDashboardPlacementGroup(runtime, id, data, signal),
    updateDashboardPlacementGroup: (
      id: string,
      groupId: string,
      data: UpdateDashboardPlacementGroupRequest,
      signal?: AbortSignal,
    ) => updateDashboardPlacementGroup(runtime, id, groupId, data, signal),
    addDashboardPlacementGroupMember: (
      id: string,
      groupId: string,
      placementId: string,
      signal?: AbortSignal,
    ) => addDashboardPlacementGroupMember(runtime, id, groupId, placementId, signal),
    deleteDashboardPlacementGroupMember: (
      id: string,
      groupId: string,
      placementId: string,
      signal?: AbortSignal,
    ) => deleteDashboardPlacementGroupMember(runtime, id, groupId, placementId, signal),
    deleteDashboardPlacementGroup: (id: string, groupId: string, signal?: AbortSignal) =>
      deleteDashboardPlacementGroup(runtime, id, groupId, signal),
    deleteDashboardPlacement: (id: string, placementId: string, signal?: AbortSignal) =>
      deleteDashboardPlacement(runtime, id, placementId, signal),
    getUsers: (signal?: AbortSignal) => getUsers(runtime, signal),
    createUser: (data: CreateUserRequest, signal?: AbortSignal) =>
      createUser(runtime, data, signal),
    updateUser: (id: string, data: UpdateUserRequest, signal?: AbortSignal) =>
      updateUser(runtime, id, data, signal),
    deleteUser: (id: string, signal?: AbortSignal) => deleteUser(runtime, id, signal),
    getGroups: (signal?: AbortSignal) => getGroups(runtime, signal),
    createGroup: (data: CreateGroupRequest, signal?: AbortSignal) =>
      createGroup(runtime, data, signal),
    updateGroup: (id: string, data: UpdateGroupRequest, signal?: AbortSignal) =>
      updateGroup(runtime, id, data, signal),
    deleteGroup: (id: string, signal?: AbortSignal) => deleteGroup(runtime, id, signal),
    getPermissions: (signal?: AbortSignal) => getPermissions(runtime, signal),
    avatarSrc: (avatarUrl: string) => avatarSrc(runtime, avatarUrl),
    getAccount: (signal?: AbortSignal) => getAccount(runtime, signal),
    updateAccount: (data: UpdateAccountRequest, signal?: AbortSignal) =>
      updateAccount(runtime, data, signal),
    changePassword: (
      currentPassword: string,
      newPassword: string,
      signal?: AbortSignal,
    ) => changePassword(runtime, currentPassword, newPassword, signal),
    uploadAvatar: (file: File, signal?: AbortSignal) => uploadAvatar(runtime, file, signal),
    deleteAvatar: (signal?: AbortSignal) => deleteAvatar(runtime, signal),
  };
}
