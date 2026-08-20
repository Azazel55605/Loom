import { useQuery } from "@tanstack/react-query";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { getHealth } from "@/lib/api";

/**
 * General settings — a stub, deliberately.
 *
 * The instance name collected during setup is persisted in `server_config` but
 * is not returned by any endpoint today, so there is nothing to display and
 * nothing to edit. Showing a name the frontend invented would be worse than
 * showing none. Surfacing it is a backend change first.
 *
 * The accent, blur, and motion controls belong here eventually — they are the
 * three customization axes in docs/UI_GUIDELINES.md and `AccentThemeProvider`
 * already writes the token. They are not built here on purpose; that is its own
 * piece of work, not a corner of this one.
 */
export function GeneralPanel() {
  const health = useQuery({
    queryKey: ["health"],
    queryFn: ({ signal }) => getHealth(signal),
    staleTime: 5 * 60 * 1000,
    retry: false,
  });

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-lg">This instance</CardTitle>
          <CardDescription>
            What this Loom is running. The instance name set during setup is not
            exposed by the API yet.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <dl className="grid gap-3 text-sm sm:grid-cols-[10rem_1fr]">
            <dt className="text-muted-foreground">Core version</dt>
            <dd className="font-mono">
              {health.isPending && <Skeleton className="h-4 w-20" />}
              {health.isError && (
                <span className="font-sans text-muted-foreground">
                  Unavailable — the backend did not answer.
                </span>
              )}
              {health.isSuccess && health.data.core_version}
            </dd>
          </dl>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Appearance and notifications</CardTitle>
          <CardDescription>
            Appearance and notification settings coming soon.
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  );
}
