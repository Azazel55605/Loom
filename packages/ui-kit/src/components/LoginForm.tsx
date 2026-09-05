import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
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
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from "@loom/ui-kit/components/ui/form";
import { Input } from "@loom/ui-kit/components/ui/input";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";
import { cn } from "@loom/ui-kit/lib/utils";

/**
 * The sign-in screen.
 *
 * The stub backend accepts any credentials, but the form validates and
 * submits as though it did not: the shape of this page should not have to
 * change when real auth lands, only what the backend does with what it sends.
 */

const loginSchema = z.object({
  username: z.string().min(1, "Enter a username."),
  password: z.string().min(1, "Enter a password."),
});

type LoginValues = z.infer<typeof loginSchema>;

export function LoginForm({
  onSuccess,
  onSubmitCredentials,
  embedded = false,
  title = "Sign in to Loom",
  description = "Sign in with the administrator account created during setup.",
  submitLabel = "Sign in",
  className,
}: {
  onSuccess: () => void;
  /** Authenticate without assuming the result should replace the active session. */
  onSubmitCredentials?: (username: string, password: string) => Promise<void>;
  embedded?: boolean;
  title?: string;
  description?: string;
  submitLabel?: string;
  className?: string;
}) {
  const { signIn } = useAuth();
  const [submitError, setSubmitError] = React.useState<string | null>(null);

  const form = useForm<LoginValues>({
    resolver: zodResolver(loginSchema),
    defaultValues: { username: "", password: "" },
  });

  async function onSubmit(values: LoginValues) {
    setSubmitError(null);
    try {
      await (onSubmitCredentials ?? signIn)(values.username, values.password);
      onSuccess();
    } catch (error: unknown) {
      setSubmitError(describeConnectorError(error));
    }
  }

  return (
    <div
      className={cn(
        !embedded && "flex min-h-screen items-center justify-center bg-background px-4",
        className,
      )}
    >
      <Card className="surface-elevated w-full max-w-sm">
        <CardHeader>
          <CardTitle>{title}</CardTitle>
          <CardDescription>{description}</CardDescription>
        </CardHeader>

        <CardContent>
          <Form {...form}>
            <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
              <FormField
                control={form.control}
                name="username"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Username</FormLabel>
                    <FormControl>
                      <Input autoComplete="username" autoFocus {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />

              <FormField
                control={form.control}
                name="password"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Password</FormLabel>
                    <FormControl>
                      <Input
                        type="password"
                        autoComplete="current-password"
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
                  <AlertTitle>Sign-in failed</AlertTitle>
                  <AlertDescription>{submitError}</AlertDescription>
                </Alert>
              )}

              <Button
                type="submit"
                className="w-full"
                disabled={form.formState.isSubmitting}
              >
                {form.formState.isSubmitting && (
                  <Loader2 className="animate-spin" aria-hidden="true" />
                )}
                {submitLabel}
              </Button>
            </form>
          </Form>
        </CardContent>
      </Card>
    </div>
  );
}
