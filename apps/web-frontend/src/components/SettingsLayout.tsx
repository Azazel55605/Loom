import { Outlet, useLocation, useNavigate } from "react-router-dom";

import { AppShell } from "@/components/AppShell";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useAuth } from "@/lib/auth-context";
import { hasPermission, PERMISSION_KEYS } from "@/lib/permissions";

/**
 * The settings chrome: a tab bar over the settings sub-routes.
 *
 * Tabs are driven by the URL rather than by internal state, so a settings page
 * is linkable and survives a reload. The tab bar reads the location and the
 * triggers navigate; there is no second copy of "which section is open".
 *
 * ## What the gating here is and is not
 *
 * The Permissions tab is rendered only for a user holding one of the two
 * administrative grants. That is **visibility, not access** — see
 * `hasPermission` for why. Deep-linking to a hidden section still renders its
 * panel, and that panel's queries come back 403, which is what it should look
 * like: the backend is the only thing deciding, and it says no. Hiding the tab
 * spares people a section full of errors; it does not pretend to protect
 * anything.
 *
 * General, Account and Appearance are visible to everyone, because they are:
 * Account acts only on the caller's own row, and Appearance never leaves the
 * browser.
 */
export function SettingsLayout() {
  const navigate = useNavigate();
  const location = useLocation();
  const { user } = useAuth();

  const permissions = user?.permissions ?? [];
  const canManageUsers = hasPermission(permissions, PERMISSION_KEYS.usersManage);
  const canManageGroups = hasPermission(permissions, PERMISSION_KEYS.groupsManage);
  // Either grant is enough to have something to do in there; the sub-nav inside
  // decides which half of it they see.
  const canAdminister = canManageUsers || canManageGroups;

  // The top-level section from the path. Taking only the third segment means a
  // nested route like `/settings/permissions/groups` still lights up the
  // Permissions tab rather than matching nothing.
  const section = location.pathname.split("/")[2] ?? "general";

  return (
    <AppShell>
      <div className="space-y-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
          <p className="text-sm text-muted-foreground">
            Instance configuration, your account, and access.
          </p>
        </div>

        <Tabs value={section} onValueChange={(value) => navigate(`/settings/${value}`)}>
          <TabsList>
            <TabsTrigger value="general">General</TabsTrigger>
            <TabsTrigger value="account">Account</TabsTrigger>
            <TabsTrigger value="appearance">Appearance</TabsTrigger>
            {canAdminister && <TabsTrigger value="permissions">Permissions</TabsTrigger>}
          </TabsList>
        </Tabs>

        <Outlet />
      </div>
    </AppShell>
  );
}
