import * as React from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { AlertCircle, Loader2, Upload } from "lucide-react";
import { toast } from "sonner";
import { z } from "zod";

import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@loom/ui-kit/components/ui/alert-dialog";
import { Alert, AlertDescription, AlertTitle } from "@loom/ui-kit/components/ui/alert";
import { Avatar, AvatarFallback, AvatarImage } from "@loom/ui-kit/components/ui/avatar";
import { Badge } from "@loom/ui-kit/components/ui/badge";
import { Button, buttonVariants } from "@loom/ui-kit/components/ui/button";
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
import { SessionManager } from "@loom/ui-kit/components/SessionManager";
import { PasswordChangeForm } from "@loom/ui-kit/pages/settings/PasswordChangeForm";
import { Skeleton } from "@loom/ui-kit/components/ui/skeleton";
import {
  ApiError,
  type Account,
} from "@loom/ui-kit/lib/api";
import { useApiClient } from "@loom/ui-kit/lib/api-context";
import { useAuth } from "@loom/ui-kit/lib/auth-context";
import { describeAdminFailure } from "@loom/ui-kit/lib/admin-error";
import { cn } from "@loom/ui-kit/lib/utils";

/** Query key for the caller's own profile, shared with the avatar mutations. */
const ACCOUNT_QUERY_KEY = ["account"];

/**
 * Your own account: profile, avatar, and password.
 *
 * Everything here acts on the signed-in user and needs no permission — the
 * backend reads the subject from the token, and there is no id to point
 * anywhere else. See the Account section of docs/API_CONTRACT.md.
 */
export function AccountPanel() {
  const api = useApiClient();
  const { signOut } = useAuth();
  const account = useQuery({
    queryKey: ACCOUNT_QUERY_KEY,
    queryFn: ({ signal }) => api.getAccount(signal),
    retry: false,
  });

  if (account.isPending) {
    return (
      <div className="space-y-4">
        <Card>
          <CardHeader>
            <Skeleton className="h-5 w-32" />
          </CardHeader>
          <CardContent className="space-y-3">
            <Skeleton className="h-9 w-full" />
            <Skeleton className="h-9 w-full" />
          </CardContent>
        </Card>
      </div>
    );
  }

  if (account.isError) {
    return (
      <Alert variant="destructive">
        <AlertCircle className="h-4 w-4" aria-hidden="true" />
        <AlertTitle>Could not load your account</AlertTitle>
        <AlertDescription>
          {describeAdminFailure(account.error).message}
        </AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="space-y-4">
      <AvatarSection account={account.data} />
      <ProfileForm account={account.data} />
      <PasswordChangeForm />
      <Card>
        <CardHeader>
          <CardTitle className="text-lg">Active Sessions</CardTitle>
          <CardDescription>
            Devices that can renew access to this account.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <SessionManager
            userId={account.data.id}
            selfService
            onSelfRevokedAll={signOut}
          />
        </CardContent>
      </Card>
    </div>
  );
}

/* -------------------------------------------------------------------------- */
/* Profile                                                                     */
/* -------------------------------------------------------------------------- */

const profileSchema = z.object({
  username: z.string().trim().min(1, "Enter a username."),
  displayName: z.string(),
});

type ProfileValues = z.infer<typeof profileSchema>;

function ProfileForm({ account }: { account: Account }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [failure, setFailure] = React.useState<string | null>(null);

  const form = useForm<ProfileValues>({
    resolver: zodResolver(profileSchema),
    defaultValues: {
      username: account.username,
      // The API distinguishes null from empty; a text input cannot hold null,
      // so it round-trips through "" and is mapped back on submit.
      displayName: account.displayName ?? "",
    },
  });

  async function onSubmit(values: ProfileValues) {
    setFailure(null);
    const displayName = values.displayName.trim();

    try {
      await api.updateAccount({
        username: values.username.trim(),
        displayName: displayName === "" ? null : displayName,
      });
    } catch (error: unknown) {
      // A taken username belongs on the username field, not in a toast: it is
      // that input that has to change, and a toast leaves the user looking at a
      // form with no indication of which field is at fault.
      if (error instanceof ApiError && error.isConflict) {
        form.setError("username", {
          type: "server",
          message: error.message || "That username is already taken.",
        });
        form.setFocus("username");
        return;
      }

      setFailure(describeAdminFailure(error).message);
      return;
    }

    // Both the profile query and the session: the header shows the username
    // from the auth context, which reads the token's claims and so still says
    // the old name until the next refresh. Refetching the account is what makes
    // this panel agree with itself immediately.
    await queryClient.invalidateQueries({ queryKey: ACCOUNT_QUERY_KEY });
    toast.success("Profile updated.");
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Profile</CardTitle>
        <CardDescription>
          How you are identified in this instance.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-4">
            {failure !== null && (
              <Alert variant="destructive">
                <AlertCircle className="h-4 w-4" aria-hidden="true" />
                <AlertTitle>Could not save your profile</AlertTitle>
                <AlertDescription>{failure}</AlertDescription>
              </Alert>
            )}

            <FormField
              control={form.control}
              name="username"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Username</FormLabel>
                  <FormControl>
                    <Input autoComplete="username" {...field} />
                  </FormControl>
                  <FormDescription>
                    What you sign in with. Changing it takes effect immediately;
                    your current session keeps working.
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <FormField
              control={form.control}
              name="displayName"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Display name</FormLabel>
                  <FormControl>
                    <Input autoComplete="name" {...field} />
                  </FormControl>
                  <FormDescription>Optional. Leave empty to clear it.</FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />

            <Button type="submit" disabled={form.formState.isSubmitting}>
              {form.formState.isSubmitting && (
                <Loader2 className="animate-spin" aria-hidden="true" />
              )}
              Save profile
            </Button>
          </form>
        </Form>

        <div className="space-y-1 border-t pt-4">
          <p className="text-sm text-muted-foreground">Groups</p>
          {account.groups.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              You do not belong to any group.
            </p>
          ) : (
            <div className="flex flex-wrap gap-1">
              {account.groups.map((group) => (
                <Badge key={group.id} variant="outline">
                  {group.name}
                </Badge>
              ))}
            </div>
          )}
          <p className="pt-1 text-xs text-muted-foreground">
            Membership is set by an administrator and cannot be changed here.
          </p>
        </div>
      </CardContent>
    </Card>
  );
}

/* -------------------------------------------------------------------------- */
/* Avatar                                                                      */
/* -------------------------------------------------------------------------- */

/** What the file picker offers, matching what the backend accepts. */
const ACCEPTED_IMAGE_TYPES = "image/png,image/jpeg,image/webp";

function AvatarSection({ account }: { account: Account }) {
  const api = useApiClient();
  const queryClient = useQueryClient();
  const [failure, setFailure] = React.useState<string | null>(null);
  const [confirmingRemoval, setConfirmingRemoval] = React.useState(false);

  const upload = useMutation({
    mutationFn: (file: File) => api.uploadAvatar(file),
    onSuccess: async () => {
      setFailure(null);
      await queryClient.invalidateQueries({ queryKey: ACCOUNT_QUERY_KEY });
      toast.success("Avatar updated.");
    },
    // The backend decides by decoding the bytes, so its message says what is
    // actually wrong — too large, not a decodable image, wrong format. Showing
    // "upload failed" instead would throw away the only useful part.
    onError: (error: unknown) => setFailure(describeAdminFailure(error).message),
  });

  const remove = useMutation({
    mutationFn: () => api.deleteAvatar(),
    onSuccess: async () => {
      setFailure(null);
      setConfirmingRemoval(false);
      await queryClient.invalidateQueries({ queryKey: ACCOUNT_QUERY_KEY });
      toast.success("Avatar removed.");
    },
    onError: (error: unknown) => setFailure(describeAdminFailure(error).message),
  });

  function onFileSelected(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    // Reset the input's value so choosing the *same* file again still fires a
    // change event — otherwise a failed upload cannot be retried without
    // picking a different file first.
    event.target.value = "";
    if (file !== undefined) upload.mutate(file);
  }

  const initials = (account.displayName ?? account.username)
    .trim()
    .slice(0, 2)
    .toUpperCase();

  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">Avatar</CardTitle>
        <CardDescription>
          A PNG, JPEG, or WebP image, up to 2 MB.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-4">
          <div className="relative">
            <Avatar className="h-20 w-20">
              {account.avatarUrl !== null && (
                <AvatarImage src={api.avatarSrc(account.avatarUrl)} alt="" />
              )}
              <AvatarFallback className="text-lg">{initials}</AvatarFallback>
            </Avatar>

            {upload.isPending && (
              <Skeleton className="absolute inset-0 rounded-full" />
            )}
          </div>

          <div className="flex flex-wrap gap-2">
            {/*
              A label styled as a button, driving a visually-hidden file input.
              The no-native-controls rule in docs/UI_GUIDELINES.md is about
              elements the browser renders in its own chrome; the OS file
              picker itself is not something any web app can replace, and the
              part that *is* ours — the trigger — is themed like every other
              button. `htmlFor` is what makes the label activate the input,
              including from the keyboard.
            */}
            <label
              htmlFor="avatar-file"
              className={cn(
                buttonVariants({ variant: "outline", size: "sm" }),
                "cursor-pointer",
                upload.isPending && "pointer-events-none opacity-50",
              )}
            >
              {upload.isPending ? (
                <Loader2 className="animate-spin" aria-hidden="true" />
              ) : (
                <Upload aria-hidden="true" />
              )}
              {account.avatarUrl === null ? "Upload image" : "Replace image"}
            </label>
            <input
              id="avatar-file"
              type="file"
              accept={ACCEPTED_IMAGE_TYPES}
              onChange={onFileSelected}
              disabled={upload.isPending}
              // `sr-only` rather than `display: none`: a hidden input is not
              // focusable, which would put the control out of reach of a
              // keyboard user even though the label is clickable.
              className="sr-only"
            />

            {account.avatarUrl !== null && (
              <Button
                variant="ghost"
                size="sm"
                onClick={() => setConfirmingRemoval(true)}
                disabled={remove.isPending || upload.isPending}
              >
                Remove
              </Button>
            )}
          </div>
        </div>

        {failure !== null && (
          <Alert variant="destructive">
            <AlertCircle className="h-4 w-4" aria-hidden="true" />
            <AlertTitle>Could not update your avatar</AlertTitle>
            <AlertDescription>{failure}</AlertDescription>
          </Alert>
        )}
      </CardContent>

      <AlertDialog
        open={confirmingRemoval}
        onOpenChange={(open) => {
          if (!open) setConfirmingRemoval(false);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove your avatar?</AlertDialogTitle>
            <AlertDialogDescription>
              The image is deleted from the server. You can upload a new one at
              any time.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={remove.isPending}>Cancel</AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={remove.isPending}
              onClick={(event) => {
                // Kept open on failure, so the explanation has somewhere to
                // land — same pattern as the users and groups panels.
                event.preventDefault();
                remove.mutate();
              }}
            >
              {remove.isPending && <Loader2 className="animate-spin" aria-hidden="true" />}
              Remove
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}
