import type { PermissionGrant } from "@/lib/api";

/**
 * Whether the signed-in user holds a grant for `key`, at any scope.
 *
 * **This function controls visibility only. It is not a security boundary.**
 * The backend enforces the real check on every request, against the claims in
 * the access token it was handed — see the "Permission enforcement" section of
 * docs/API_CONTRACT.md. Everything here can be edited away in a browser
 * console, and doing so gains nothing: the request still comes back 403.
 *
 * ## Why scope is ignored
 *
 * Scope is deliberately not matched. This answers "is this section worth
 * showing you at all", and holding `connectors.control` over a single connector
 * is a reason to show the connectors area, not to hide it. The backend's check
 * is the opposite — a scoped grant does **not** satisfy a global check there,
 * precisely so a narrow grant is never silently widened.
 *
 * Mirroring that asymmetry here would mean maintaining a second copy of the
 * matching rules whose only job is to decide what a menu looks like, and a
 * second copy is a second thing to get subtly wrong. Where a scope genuinely
 * matters — which connector a user may act on — the server already decides, and
 * the UI reports what it said.
 *
 * The consequence to be aware of: a user with only a scoped grant sees a tab
 * whose contents may then 403. That is the right trade for a menu, and it is
 * visible rather than silent.
 */
export function hasPermission(permissions: PermissionGrant[], key: string): boolean {
  return permissions.some((grant) => grant.key === key);
}

/** Registered permission keys, as of the current migrations. Not authoritative
 *  — `GET /permissions` is; these exist so a call site can avoid a bare string
 *  literal for the handful of keys the UI itself gates on. */
export const PERMISSION_KEYS = {
  connectorsView: "connectors.view",
  connectorsControl: "connectors.control",
  usersManage: "users.manage",
  groupsManage: "groups.manage",
  systemSettings: "system.settings",
} as const;
