import { LoginForm } from "@loom/ui-kit/components/LoginForm";
import { useAuth } from "@loom/ui-kit/lib/auth-context";

export function KioskRecoveryScreen({ expectedAccountId }: { expectedAccountId: string | null }) {
  const {
    authenticateWithoutPersisting,
    activateAuthentication,
    discardAuthentication,
  } = useAuth();

  return (
    <main className="mobile-kiosk-recovery flex min-h-screen items-center justify-center px-4">
      <LoginForm
        embedded
        title="Kiosk session expired"
        description="Re-enter the kiosk account's credentials to restore this display. Another account cannot use this screen to exit kiosk mode."
        submitLabel="Restore kiosk"
        onSubmitCredentials={async (username, password) => {
          const candidate = await authenticateWithoutPersisting(username, password);
          if (
            expectedAccountId === null ||
            candidate.account.id !== expectedAccountId ||
            !candidate.account.isKiosk
          ) {
            await discardAuthentication(candidate);
            throw new Error("Enter the same kiosk account's credentials to restore this display");
          }
          await activateAuthentication(candidate);
        }}
        onSuccess={() => undefined}
      />
    </main>
  );
}
