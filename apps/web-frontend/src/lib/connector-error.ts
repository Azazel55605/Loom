import {
  ApiError,
  MISSING_STUB_BACKEND_MESSAGE,
  type ConnectorError,
} from "@/lib/api";

/**
 * Turns a failure into one sentence a person can act on.
 *
 * `ConnectorError` is externally tagged — one key naming the variant — so this
 * switches on that key rather than parsing a message string. The backend also
 * sends a rendered `error` string, and where one is available it is preferred:
 * it is the connector's own words. This exists for the cases where all we have
 * is the structured value, and so the variant names never reach a user.
 */
export function describeConnectorError(error: unknown): string {
  if (error instanceof ApiError) {
    // A 404 with no error body behind it is not a missing record — the route
    // does not exist at all, which means a backend built without the
    // `dev-stub-auth` feature. Say that, rather than passing on "404 Not
    // Found" and sending people hunting for a typo in a URL that is correct.
    // A 404 that *does* carry a body (an unknown connector or action id) is a
    // real answer and keeps its own message.
    if (error.isMissingRoute) return MISSING_STUB_BACKEND_MESSAGE;

    // The backend's rendered message is the better text; the structured value
    // is the fallback for the rare body that carries only the latter.
    if (error.message) return error.message;
    if (error.connectorError) return describeVariant(error.connectorError);
    return `Request failed with status ${error.status}.`;
  }

  if (isConnectorError(error)) return describeVariant(error);

  if (error instanceof Error) return error.message;

  return "An unknown error occurred.";
}

function isConnectorError(value: unknown): value is ConnectorError {
  if (typeof value !== "object" || value === null) return false;
  const keys = Object.keys(value);
  return (
    keys.length === 1 &&
    ["unreachable", "authFailed", "invalidAction", "invalidParams", "internal"].includes(
      keys[0],
    )
  );
}

function describeVariant(error: ConnectorError): string {
  if ("unreachable" in error) {
    return `The service could not be reached: ${error.unreachable.reason}`;
  }
  if ("authFailed" in error) {
    // Loom's stored credentials for the service were rejected — not the user's
    // session. Saying "sign in again" here would send them down a dead end.
    return `Loom's credentials for this service were rejected: ${error.authFailed.reason}`;
  }
  if ("invalidAction" in error) {
    return `The service does not support the action "${error.invalidAction.actionId}".`;
  }
  if ("invalidParams" in error) {
    return `Invalid parameters for "${error.invalidParams.actionId}": ${error.invalidParams.reason}`;
  }
  return `The connector failed internally: ${error.internal}`;
}
