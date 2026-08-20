import { Navigate, Outlet, useLocation, useNavigate } from "react-router-dom";

import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAuth } from "@/lib/auth-context";
import { hasPermission, PERMISSION_KEYS } from "@/lib/permissions";

/**
 * The Permissions section: users and groups, under one tab.
 *
 * They were two top-level tabs before. They belong together — a group is only
 * meaningful as something users are put into, and administering access means
 * moving between the two constantly — so the top level now carries one entry
 * and the split lives inside it.
 *
 * ## The sub-nav appears only when there is a choice
 *
 * A user holding just one of the two grants gets that panel directly, with no
 * sub-nav at all. A tab list with a single tab is not navigation; it is a
 * label pretending to be a control, and it invites the question "what is the
 * other one?" for a section the user cannot reach anyway.
 */
export function PermissionsLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const { user } = useAuth();

  const permissions = user?.permissions ?? [];
  const canManageUsers = hasPermission(permissions, PERMISSION_KEYS.usersManage);
  const canManageGroups = hasPermission(permissions, PERMISSION_KEYS.groupsManage);
  const showSubNav = canManageUsers && canManageGroups;

  // The fourth segment: `/settings/permissions/{subsection}`.
  const subsection = location.pathname.split("/")[3] ?? "users";

  return (
    <div className="space-y-4">
      {showSubNav && (
        <Tabs
          value={subsection}
          onValueChange={(value) => navigate(`/settings/permissions/${value}`)}
        >
          {/* Smaller than the section tabs above it, so the hierarchy is legible
              at a glance rather than looking like two peer tab bars. */}
          <TabsList className="h-8">
            <TabsTrigger value="users" className="text-xs">
              Users
            </TabsTrigger>
            <TabsTrigger value="groups" className="text-xs">
              Groups
            </TabsTrigger>
          </TabsList>
        </Tabs>
      )}

      <Outlet />
    </div>
  );
}

/**
 * Where `/settings/permissions` with no subsection lands.
 *
 * Users when that grant is held, groups otherwise. A user with neither is sent
 * to Users as well: the panel then reports the backend's 403, which is the
 * honest outcome for someone who navigated to a section they cannot use, and
 * is better than a redirect loop or a blank page.
 */
export function PermissionsIndexRedirect() {
  const { user } = useAuth();
  const canManageUsers = hasPermission(
    user?.permissions ?? [],
    PERMISSION_KEYS.usersManage,
  );

  return (
    <Navigate to={canManageUsers ? "users" : "groups"} replace />
  );
}
