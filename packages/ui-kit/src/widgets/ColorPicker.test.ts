import { describe, expect, it } from "vitest";

import { normalizeHexColor } from "@loom/ui-kit/widgets/ColorPicker";

describe("normalizeHexColor", () => {
  it("accepts the shared six-digit hex convention and normalizes case", () => {
    expect(normalizeHexColor("#a1b2c3")).toBe("#A1B2C3");
    expect(normalizeHexColor("#2F7FED")).toBe("#2F7FED");
  });

  it("rejects native or abbreviated color representations", () => {
    expect(normalizeHexColor("#37e")).toBeNull();
    expect(normalizeHexColor("rgb(47, 127, 237)")).toBeNull();
    expect(normalizeHexColor(null)).toBeNull();
  });
});
