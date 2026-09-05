import * as React from "react";

import { Tabs, TabsList, TabsTrigger } from "@loom/ui-kit/components/ui/tabs";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";

export function SettingsLayout({
  activeSection,
  onSectionChange,
  renderShell,
  children,
}: {
  activeSection: string;
  onSectionChange: (section: string) => void;
  renderShell: (content: React.ReactNode) => React.ReactNode;
  children: React.ReactNode;
}) {
  const { user } = useAuth();
  const permissions = user?.permissions ?? [];
  const canAdminister =
    hasPermission(permissions, PERMISSION_KEYS.usersManage) ||
    hasPermission(permissions, PERMISSION_KEYS.groupsManage);
  const canViewAuditLog = hasPermission(
    permissions,
    PERMISSION_KEYS.connectorsManage,
  );
  const canManageDashboards = hasPermission(
    permissions,
    PERMISSION_KEYS.dashboardsManage,
  );

  return renderShell(
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Instance configuration, your account, and access.
        </p>
      </div>

      <Tabs value={activeSection} onValueChange={onSectionChange}>
        <div className="max-w-full touch-pan-x overflow-x-auto pb-1">
          <TabsList className="w-max min-w-full justify-start sm:min-w-0">
            <TabsTrigger className="shrink-0" value="general">
              General
            </TabsTrigger>
            <TabsTrigger className="shrink-0" value="account">
              Account
            </TabsTrigger>
            <TabsTrigger className="shrink-0" value="appearance">
              Appearance
            </TabsTrigger>
            {canAdminister && (
              <TabsTrigger className="shrink-0" value="permissions">
                Permissions
              </TabsTrigger>
            )}
            {canViewAuditLog && (
              <TabsTrigger className="shrink-0" value="audit-log">
                Audit Log
              </TabsTrigger>
            )}
            {canManageDashboards && (
              <TabsTrigger className="shrink-0" value="dashboards">
                Dashboards
              </TabsTrigger>
            )}
          </TabsList>
        </div>
      </Tabs>

      {children}
    </div>,
  );
}
