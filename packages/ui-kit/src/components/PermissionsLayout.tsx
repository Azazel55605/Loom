import * as React from "react";

import { Tabs, TabsList, TabsTrigger } from "@loom/ui-kit/components/ui/tabs";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { hasPermission, PERMISSION_KEYS } from "@loom/ui-kit/lib/permissions";

export function usePreferredPermissionsSection(): "users" | "groups" {
  const { user } = useAuth();
  return hasPermission(user?.permissions ?? [], PERMISSION_KEYS.usersManage)
    ? "users"
    : "groups";
}

export function PermissionsLayout({
  activeSection,
  onSectionChange,
  children,
}: {
  activeSection: string;
  onSectionChange: (section: string) => void;
  children: React.ReactNode;
}) {
  const { user } = useAuth();
  const permissions = user?.permissions ?? [];
  const showSubNav =
    hasPermission(permissions, PERMISSION_KEYS.usersManage) &&
    hasPermission(permissions, PERMISSION_KEYS.groupsManage);

  return (
    <div className="space-y-4">
      {showSubNav && (
        <Tabs value={activeSection} onValueChange={onSectionChange}>
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
      {children}
    </div>
  );
}
