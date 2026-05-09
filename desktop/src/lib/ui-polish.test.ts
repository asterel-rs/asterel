import { describe, expect, test } from "bun:test";
import {
  MIN_INTERACTIVE_SIZE_PX,
  getOpticalInlineControlStyle,
  getOpticalPanelInsetStyle,
} from "./ui-polish";

describe("getOpticalInlineControlStyle", () => {
  test("compact controls keep a 40px hit target", () => {
    const style = getOpticalInlineControlStyle({ density: "compact" });

    expect(style.minHeight).toBe(`${MIN_INTERACTIVE_SIZE_PX}px`);
  });

  test("trailing-icon controls tighten the end padding", () => {
    const style = getOpticalInlineControlStyle({
      density: "compact",
      icon: "trailing",
    });

    expect(style.paddingInlineStart).toBe("12px");
    expect(style.paddingInlineEnd).toBe("10px");
    expect(style.columnGap).toBe("6px");
  });
});

describe("getOpticalPanelInsetStyle", () => {
  test("compact panel insets bias slightly heavier toward the bottom", () => {
    const style = getOpticalPanelInsetStyle({ density: "compact" });

    expect(style.paddingTop).toBe("18px");
    expect(style.paddingRight).toBe("20px");
    expect(style.paddingBottom).toBe("20px");
    expect(style.paddingLeft).toBe("20px");
  });
});
