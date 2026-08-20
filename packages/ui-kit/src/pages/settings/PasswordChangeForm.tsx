import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useForm } from "react-hook-form";
import { AlertCircle, Loader2 } from "lucide-react";
import { toast } from "sonner";
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
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";

/**
 * Changing your own password.
 *
 * Its own component and its own form, separate from the profile form, because
 * it is a separate transaction: it needs the current password as proof, it can
 * fail in ways the profile form cannot, and combining them would mean a user
 * changing their display name has to think about their password.
 */

const passwordSchema = z
  .object({
    currentPassword: z.string().min(1, "Enter your current password."),
    // The same floor the backend applies, from setup and user creation too.
    // Stated here as well so the user hears about it before a round trip.
    newPassword: z.string().min(8, "Use at least 8 characters."),
    confirmPassword: z.string().min(1, "Repeat the new password."),
  })
  .refine((values) => values.newPassword === values.confirmPassword, {
    // Attached to the confirm field so the message appears where the fix is,
    // matching SetupPage.
    path: ["confirmPassword"],
    message: "Passwords do not match.",
  });

type PasswordValues = z.infer<typeof passwordSchema>;

const EMPTY: PasswordValues = {
  currentPassword: "",
  newPassword: "",
  confirmPassword: "",
};

export function PasswordChangeForm() {
  const api = useApiClient();
  const [failure, setFailure] = React.useState<string | null>(null);

  const form = useForm<PasswordValues>({
    resolver: zodResolver(passwordSchema),
    defaultValues: EMPTY,
  });

  async function onSubmit(values: PasswordValues) {
    setFailure(null);
    try {
      await api.changePassword(values.currentPassword, values.newPassword);
    } catch (error: unknown) {
      // A 401 here means the *current password* was wrong, not that the session
      // expired — the API client opts this call out of its refresh-and-retry so
      // that distinction survives the transport. Reported on the field that is
      // actually wrong, and nothing is cleared: the new password the user
      // already typed is still what they want, and making them retype it to
      // correct a different field would be a small cruelty.
      if (error instanceof ApiError && error.isUnauthorized) {
        form.setError("currentPassword", {
          type: "server",
          message: error.message || "Current password is incorrect.",
        });
        form.setFocus("currentPassword");
        return;
      }

      setFailure(describeAdminFailure(error).message);
      return;
    }

    // Cleared only on success, so nothing sensitive lingers in the form state.
    form.reset(EMPTY);
    toast.success("Password changed.");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Password</CardTitle>
        <CardDescription>
          Changing your password does not sign out your other sessions.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {failure !== null && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" aria-hidden="true" />
                <AlertTitle>Could not change your password</AlertTitle>
                <AlertDescription>{failure}</AlertDescription>
              </Alert>
            )}

            <FormField
              control={form.control}
              name="currentPassword"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Current password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="current-password" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="newPassword"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>New password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="new-password" {...field} />
                  </FormControl>
                  <FormDescription>At least 8 characters.</FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="confirmPassword"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Confirm new password</FormLabel>
                  <FormControl>
                    <Input type="password" autoComplete="new-password" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />

            <Button type="submit" disabled={form.formState.isSubmitting}>
              {form.formState.isSubmitting && (
                <Loader2 className="animate-spin" aria-hidden="true" />
              )}
              Change password
            </Button>
          </form>
        </Form>
      </CardContent>
    </Card>
  );
}
