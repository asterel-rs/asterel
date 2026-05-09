/**
 * Hand-drawn SVG marks — tiny imperfect shapes that feel like
 * someone sketched them with a pen on notebook paper.
 *
 * Every path is intentionally wobbly: control points are offset
 * by 0.3-0.8px from "perfect" to avoid looking machine-generated.
 */

import type { CSSProperties } from "react";

interface MarkProps {
  size?: number;
  color?: string;
  className?: string;
  style?: CSSProperties;
}

/** A hand-drawn checkmark — slightly lopsided like a quick pen stroke */
export function InkCheck({ size = 12, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 14 14"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 2.5 7.2 C 3.1 7.9 4.2 9.6 5.3 11 C 6.1 9.1 8.4 4.8 11.5 2.5"
        stroke={color}
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A hand-drawn cross — two slightly uneven strokes */
export function InkCross({ size = 12, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 14 14"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 3 3.2 C 4.5 4.8 6.8 7.5 10.8 10.5"
        stroke={color}
        strokeWidth="1.5"
        strokeLinecap="round"
      />
      <path
        d="M 10.5 3.5 C 8.8 5.2 5.5 7.8 3.2 10.8"
        stroke={color}
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** A hand-drawn wavy tilde — for "degraded" or "in-between" states */
export function InkWavy({ size = 12, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 14 14"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 2 7.5 C 3.5 5 5.2 9.5 7 7 C 8.8 4.5 10.5 9 12 6.5"
        stroke={color}
        strokeWidth="1.5"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** A hand-drawn dash — for neutral/unknown states */
export function InkDash({ size = 12, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 14 14"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path d="M 3 7.3 C 5 6.8 9 7.5 11 7" stroke={color} strokeWidth="1.5" strokeLinecap="round" />
    </svg>
  );
}

/** A hand-drawn circle — for info, slightly oval and imperfect */
export function InkCircle({ size = 12, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 14 14"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 7 2.5 C 10 2.2 12 4.8 11.8 7.2 C 11.5 9.8 9 12 6.8 11.8 C 4 11.5 2 9.2 2.3 6.5 C 2.6 4 4.5 2.8 7 2.5 Z"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** A hand-drawn right arrow — pen-stroke style, for navigation */
export function InkArrow({ size = 14, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 14"
      width={size}
      height={(size / 16) * 14}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 2 7.2 C 4.5 7 8.5 6.8 12.5 7"
        stroke={color}
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <path
        d="M 9.5 3.8 C 10.5 5 12 6.5 13 7 C 12 7.8 10.5 9.2 9.2 10.5"
        stroke={color}
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** A hand-drawn empty-page doodle — a small notebook sketch with a pencil.
 *  Used in EmptyState to make "nothing here" feel warm. */
export function InkEmptyPage({ size = 48, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 48 48"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      {/* Notebook page outline — slightly wonky rectangle */}
      <path
        d="M 10 6 C 10.5 5.8 35 5.5 37 6 C 37.3 6.5 37.5 40 37 41 C 36 41.5 11 42 10 41 C 9.5 40.5 9.8 7 10 6 Z"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
        opacity="0.6"
      />
      {/* Ruling lines */}
      <line x1="14" y1="14" x2="33" y2="14.3" stroke={color} strokeWidth="0.6" opacity="0.2" />
      <line x1="14" y1="20" x2="33" y2="19.7" stroke={color} strokeWidth="0.6" opacity="0.2" />
      <line x1="14" y1="26" x2="33" y2="26.2" stroke={color} strokeWidth="0.6" opacity="0.2" />
      <line x1="14" y1="32" x2="25" y2="31.8" stroke={color} strokeWidth="0.6" opacity="0.2" />
      {/* Pencil — resting diagonally on the page */}
      <path
        d="M 28 38 L 39 27 L 41 28.5 L 30 39.5 Z"
        stroke={color}
        strokeWidth="1"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.5"
      />
      {/* Pencil tip */}
      <path
        d="M 28 38 L 26.5 40 L 30 39.5"
        stroke={color}
        strokeWidth="0.8"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.5"
      />
      {/* Margin line */}
      <line x1="13" y1="8" x2="13" y2="39" stroke={color} strokeWidth="0.7" opacity="0.15" />
    </svg>
  );
}

/** A hand-drawn broken-pencil doodle — used in ErrorState. */
export function InkBrokenPencil({
  size = 48,
  color = "currentColor",
  className,
  style,
}: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 48 48"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      {/* Upper half of pencil */}
      <path
        d="M 32 8 L 38 12 L 28 24 L 22 20 Z"
        stroke={color}
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.55"
      />
      {/* Break gap — jagged edges */}
      <path
        d="M 22 20 L 23 22 L 21 22.5"
        stroke={color}
        strokeWidth="1"
        strokeLinecap="round"
        opacity="0.5"
      />
      <path
        d="M 28 24 L 27 25.5 L 29 26"
        stroke={color}
        strokeWidth="1"
        strokeLinecap="round"
        opacity="0.5"
      />
      {/* Lower half — slightly offset to show it's broken */}
      <path
        d="M 19 25 L 25 29 L 16 40 L 10 36 Z"
        stroke={color}
        strokeWidth="1.2"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.55"
      />
      {/* Pencil tip */}
      <path
        d="M 10 36 L 8 42 L 16 40"
        stroke={color}
        strokeWidth="1"
        strokeLinecap="round"
        strokeLinejoin="round"
        opacity="0.5"
      />
      {/* Small scribble marks near the break — frustration */}
      <path
        d="M 30 28 C 31 27 33 28 32 29"
        stroke={color}
        strokeWidth="0.8"
        strokeLinecap="round"
        opacity="0.3"
      />
      <path
        d="M 33 26 C 34 25 36 26 35 27"
        stroke={color}
        strokeWidth="0.8"
        strokeLinecap="round"
        opacity="0.3"
      />
    </svg>
  );
}

/** A hand-drawn globe icon — wobbly circle with latitude/longitude lines */
export function InkGlobe({ size = 16, color = "currentColor", className, style }: MarkProps) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      className={className}
      style={style}
    >
      <path
        d="M 8 1.5 C 11.2 1.3 14.8 4.2 14.6 8.3 C 14.3 12.1 11.1 14.8 7.8 14.5 C 4.2 14.2 1.2 11.3 1.4 7.7 C 1.6 4.3 4.5 1.7 8 1.5 Z"
        stroke={color}
        strokeWidth="1.3"
        strokeLinecap="round"
      />
      <path
        d="M 8.1 1.7 C 10.2 3.8 11.3 6.2 11.2 8.2 C 11.1 10.3 10 12.5 8 14.4"
        stroke={color}
        strokeWidth="1.05"
        strokeLinecap="round"
      />
      <path
        d="M 7.9 1.7 C 5.9 3.7 4.8 6.1 4.9 8.1 C 5 10.2 6.1 12.6 8 14.4"
        stroke={color}
        strokeWidth="1.05"
        strokeLinecap="round"
      />
      <path
        d="M 2.2 5.8 C 4.7 5.1 11.3 5.2 13.8 5.9"
        stroke={color}
        strokeWidth="0.95"
        strokeLinecap="round"
      />
      <path
        d="M 2.4 10.3 C 4.8 11 11.2 10.9 13.6 10.2"
        stroke={color}
        strokeWidth="0.95"
        strokeLinecap="round"
      />
    </svg>
  );
}

/** A hand-drawn wavy divider line — organic replacement for straight rules */
export function InkDivider({ color = "currentColor", className, style }: Omit<MarkProps, "size">) {
  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 200 6"
      preserveAspectRatio="none"
      width="100%"
      height="5"
      fill="none"
      className={className}
      style={{ display: "block", ...style }}
    >
      <path
        d="M 0 3 C 8 1.5 16 4.5 24 3 C 32 1.5 40 4.2 48 3 C 56 1.8 64 4.5 72 3 C 80 1.5 88 4.2 96 3 C 104 1.8 112 4.5 120 3 C 128 1.5 136 4 144 3 C 152 2 160 4.5 168 3 C 176 1.5 184 4.2 192 3 L 200 3"
        stroke={color}
        strokeWidth="1"
        strokeLinecap="round"
      />
    </svg>
  );
}
