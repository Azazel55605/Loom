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

  return renderShell(
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-semibold tracking-tight">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Instance configuration, your account, and access.
        </p>
      </div>

      <Tabs value={activeSection} onValueChange={onSectionChange}>
        <TabsList>
          <TabsTrigger value="general">General</TabsTrigger>
          <TabsTrigger value="account">Account</TabsTrigger>
          <TabsTrigger value="appearance">Appearance</TabsTrigger>
          {canAdminister && <TabsTrigger value="permissions">Permissions</TabsTrigger>}
        </TabsList>
      </Tabs>

      {children}
    </div>,
  );
}
