import { ApiError } from "@loom/ui-kit/lib/api";
import { describeConnectorError } from "@loom/ui-kit/lib/connector-error";

/**
 * How an administration request failed, split by what the user can do about it.
 *
 * The distinction matters because the backend's refusals are *designed to be
 * read*. "That would leave no active administrator" is instruction; collapsing
 * it into "Something went wrong" throws away the only part that helps. See the
 * Safeguards section of docs/API_CONTRACT.md.
 */
export type AdminFailureKind =
  /** 409 — the request was understood and refused because of the state it
   *  would produce. A safeguard, a taken name. Nothing changed. */
  | "refused"
  /** 403 — authenticated, and still not allowed. Refreshing will not help. */
  | "forbidden"
  /** Anything else: a network failure, a 5xx, a malformed request. */
  | "failed";

export type AdminFailure = {
  kind: AdminFailureKind;
  /** One sentence for a person. For a refusal this is the backend's own text. */
  message: string;
};

export function describeAdminFailure(error: unknown): AdminFailure {
  if (error instanceof ApiError) {
    if (error.isConflict) {
      return {
        kind: "refused",
        // The backend always sends a message on these. The fallback is for a
        // 409 from somewhere that does not, which would otherwise render blank.
        message: error.message || "The instance refused that change.",
      };
    }
    if (error.isForbidden) {
      return {
        kind: "forbidden",
        message:
          error.message ||
          "You do not have permission to do that. Ask an administrator for the " +
            "grant, then sign out and back in — permission changes reach a " +
            "signed-in session within 15 minutes.",
      };
    }
  }

  return { kind: "failed", message: describeConnectorError(error) };
}
