import { describe, expect, it } from "vitest";

import {
  describeDisplayWidget,
  displayWidgetFromKey,
  getCompatibleWidgetTypes,
} from "@loom/ui-kit/widgets/compatibility";

describe("Image widget compatibility", () => {
  it("offers only the Image display for Image-valued data points", () => {
    expect(getCompatibleWidgetTypes("image")).toEqual(["image"]);
    expect(displayWidgetFromKey("image")).toBe("image");
    expect(describeDisplayWidget("image")).toBe("Image");
  });
});
