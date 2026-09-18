import { describe, expect, it } from "vitest";

import { ApiError, SessionExpiredError } from "@loom/ui-kit/lib/api";
import {
  INITIAL_RETRY_DELAY_MS,
  MAX_RETRY_DELAY_MS,
  isSessionRejection,
  nextRetryDelayMs,
} from "@loom/ui-kit/lib/session-failure";

describe("isSessionRejection", () => {
  it("accepts the two failures that mean the server rejected the session", () => {
    expect(isSessionRejection(new SessionExpiredError())).toBe(true);
    expect(isSessionRejection(new ApiError(401, "Unauthorized"))).toBe(true);
  });

  it("rejects transport failures, which say nothing about the token", () => {
    // What a browser fetch and a native HTTP client raise when the host does
    // not resolve, the connection is refused, or the request times out.
    expect(isSessionRejection(new TypeError("Failed to fetch"))).toBe(false);
    expect(isSessionRejection(new Error("error sending request for url"))).toBe(false);
    expect(isSessionRejection(new DOMException("aborted", "AbortError"))).toBe(false);
    expect(isSessionRejection(undefined)).toBe(false);
  });

  it("rejects answered requests that are not a 401", () => {
    expect(isSessionRejection(new ApiError(403, "Forbidden"))).toBe(false);
    expect(isSessionRejection(new ApiError(502, "Bad Gateway"))).toBe(false);
    expect(isSessionRejection(new ApiError(503, "Service Unavailable"))).toBe(false);
  });
});

describe("nextRetryDelayMs", () => {
  it("doubles and settles at the shared ceiling", () => {
    expect(nextRetryDelayMs(INITIAL_RETRY_DELAY_MS)).toBe(2_000);
    expect(nextRetryDelayMs(16_000)).toBe(MAX_RETRY_DELAY_MS);
    expect(nextRetryDelayMs(MAX_RETRY_DELAY_MS)).toBe(MAX_RETRY_DELAY_MS);
  });
});
