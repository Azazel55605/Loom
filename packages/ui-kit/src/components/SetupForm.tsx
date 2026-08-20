import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { AlertCircle, Loader2 } from "lucide-react";
import { z } from "zod";

import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Button } from "@loom/ui-kit/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@loom/ui-kit/components/ui/card";
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@loom/ui-kit/components/ui/form";
import { Input } from "@loom/ui-kit/components/ui/input";
import { ApiError } from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { SETUP_STATUS_QUERY_KEY } from "@loom/ui-kit/lib/use-setup-status";

/**
 * First-run setup.
 *
 * Names the instance and creates the first administrator. The stub backend
 * discards every value, but the form is built as though it did not: the shape
 * is the contract, and what changes later is what the backend does with it, not
 * what the wizard collects.
 */

const setupSchema = z
  .object({
    instanceName: z.string().min(1, "Give this instance a name."),
    adminUsername: z.string().min(1, "Choose an administrator username."),
    // A length floor rather than a composition rule: length is what actually
    // costs an attacker, and character-class rules mostly produce predictable
    // substitutions. The real implementation should check against a breached-
    // password list instead of adding more rules here.
    adminPassword: z.string().min(8, "Use at least 8 characters."),
    confirmPassword: z.string().min(1, "Repeat the password."),
  })
  .refine((values) => values.adminPassword === values.confirmPassword, {
    // Attached to the confirm field so the message appears where the fix is.
    path: ["confirmPassword"],
    message: "Passwords do not match.",
  });

type SetupValues = z.infer<typeof setupSchema>;

export function SetupForm({ onComplete }: { onComplete: () => void }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [submitError, setSubmitError] = React.useState<string | null>(null);

  const form = useForm<SetupValues>({
    resolver: zodResolver(setupSchema),
    defaultValues: {
      instanceName: "",
      adminUsername: "",
      adminPassword: "",
      confirmPassword: "",
    },
  });

  async function onSubmit(values: SetupValues) {
    setSubmitError(null);
    try {
      await api.completeSetup({
        instanceName: values.instanceName,
        adminUsername: values.adminUsername,
        adminPassword: values.adminPassword,
      });
    } catch (error: unknown) {
      // A 409 means someone else completed setup first — another tab, most
      // likely. The instance is configured, which is what this form was for,
      // so continue to login rather than reporting a failure the user cannot
      // act on and would not understand.
      if (!(error instanceof ApiError && error.isAlreadyComplete)) {
        setSubmitError(describeConnectorError(error));
        return;
      }
    }

    // Drop the cached "setup incomplete" answer before navigating, or the gate
    // reads a stale `false` and bounces straight back here.
    await queryClient.invalidateQueries({ queryKey: SETUP_STATUS_QUERY_KEY });
    onComplete();
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-background px-4 py-10">
      <Card className="surface-elevated w-full max-w-md">
        <CardHeader>
          <CardTitle>Set up Loom</CardTitle>
          <CardDescription>
            Name this instance and create its first administrator. This runs once.
          </CardDescription>
        </CardHeader>

        <CardContent>
          <Form {...form}>
            <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
              <FormField
                control={form.control}
                name="instanceName"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Instance name</FormLabel>
                    <FormControl>
                      <Input autoFocus placeholder="Example Homelab" {...field} />
                    </FormControl>
                    <FormDescription>
                      How this Loom shows up in the interface.
                    </FormDescription>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="adminUsername"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Administrator username</FormLabel>
                    <FormControl>
                      <Input autoComplete="username" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="adminPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="new-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="confirmPassword"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Confirm password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="new-password"
                        {...field}
                      />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              {submitError !== null && (
                <Alert variant="destructive">
                  <AlertCircle className="h-4 w-4" aria-hidden="true" />
                  <AlertTitle>Setup failed</AlertTitle>
                  <AlertDescription>{submitError}</AlertDescription>
                </Alert>
              )}

              <Alert>
                <AlertTitle>This build does not store what you enter</AlertTitle>
                <AlertDescription>
                  Setup is a stub: these values are discarded, and the instance
                  forgets it was set up when the backend restarts.
                </AlertDescription>
              </Alert>

              <Button
                type="submit"
                className="w-full"
                disabled={form.formState.isSubmitting}
              >
                {form.formState.isSubmitting && (
                  <Loader2 className="animate-spin" aria-hidden="true" />
                )}
                Complete setup
              </Button>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  );
}
