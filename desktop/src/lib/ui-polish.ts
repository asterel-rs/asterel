import type { CSSProperties } from "react";

export const MIN_INTERACTIVE_SIZE_PX = 40;

type OpticalDensity = "compact" | "regular";
type OpticalIconLayout = "none" | "leading" | "trailing" | "both";

const DENSITY_SPECS = {
  compact: {
    gap: 6,
    paddingInline: 12,
    paddingBlock: 8,
    panelInline: 20,
    panelTop: 18,
    panelBottom: 20,
  },
  regular: {
    gap: 8,
    paddingInline: 14,
    paddingBlock: 10,
    panelInline: 22,
    panelTop: 20,
    panelBottom: 22,
  },
} as const;

const ICON_SIDE_CORRECTION_PX = 2;

export function getOpticalInlineControlStyle({
  density = "compact",
  icon = "none",
}: {
  density?: OpticalDensity;
  icon?: OpticalIconLayout;
} = {}): CSSProperties {
  const spec = DENSITY_SPECS[density];
  let paddingInlineStart = spec.paddingInline;
  let paddingInlineEnd = spec.paddingInline;

  if (icon === "leading") {
    paddingInlineStart -= ICON_SIDE_CORRECTION_PX;
  } else if (icon === "trailing") {
    paddingInlineEnd -= ICON_SIDE_CORRECTION_PX;
  } else if (icon === "both") {
    paddingInlineStart -= 1;
    paddingInlineEnd -= 1;
  }

  return {
    minHeight: `${MIN_INTERACTIVE_SIZE_PX}px`,
    paddingBlock: `${spec.paddingBlock}px`,
    paddingInlineStart: `${paddingInlineStart}px`,
    paddingInlineEnd: `${paddingInlineEnd}px`,
    columnGap: `${spec.gap}px`,
  };
}

export function getOpticalPanelInsetStyle({
  density = "regular",
}: {
  density?: OpticalDensity;
} = {}): CSSProperties {
  const spec = DENSITY_SPECS[density];

  return {
    paddingTop: `${spec.panelTop}px`,
    paddingRight: `${spec.panelInline}px`,
    paddingBottom: `${spec.panelBottom}px`,
    paddingLeft: `${spec.panelInline}px`,
  };
}
