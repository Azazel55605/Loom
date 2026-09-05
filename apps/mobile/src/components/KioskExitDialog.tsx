import { useMobileKioskMode } from "@/components/mobileKioskMode";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@loom/ui-kit/components/ui/dialog";
import { LoginForm } from "@loom/ui-kit/components/LoginForm";
import { useAuth } from "@loom/ui-kit/lib/auth-context";

export function KioskExitDialog({
  open,
  activeKioskUserId,
  onOpenChange,
  onExit,
}: {
  open: boolean;
  activeKioskUserId: string;
  onOpenChange: (open: boolean) => void;
  onExit: () => void;
}) {
  const {
    authenticateWithoutPersisting,
    activateAuthentication,
    discardAuthentication,
  } = useAuth();
  const kiosk = useMobileKioskMode();

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="p-0 sm:max-w-md">
        <DialogHeader className="sr-only">
          <DialogTitle>Exit Kiosk Mode</DialogTitle>
          <DialogDescription>
            Sign in with a different, non-kiosk account to exit.
          </DialogDescription>
        </DialogHeader>
        <LoginForm
          embedded
          title="Exit Kiosk Mode"
          description="Sign in with a different, non-kiosk account. The current kiosk session stays active unless verification succeeds."
          submitLabel="Verify and exit"
          onSubmitCredentials={async (username, password) => {
            const candidate = await authenticateWithoutPersisting(username, password);
            if (candidate.account.id === activeKioskUserId) {
              await discardAuthentication(candidate);
              throw new Error("Enter a different account's credentials to exit kiosk mode");
            }
            if (candidate.account.isKiosk) {
              await discardAuthentication(candidate);
              throw new Error("Enter a different, non-kiosk account's credentials to exit kiosk mode");
            }
            await kiosk.exitWith(() => activateAuthentication(candidate));
          }}
          onSuccess={onExit}
        />
      </DialogContent>
    </Dialog>
  );
}
