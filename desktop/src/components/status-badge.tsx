import type React from "react";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { InkCheck, InkCircle, InkCross, InkDash, InkWavy } from "./ink-marks";

type Variant = "ok" | "degraded" | "error" | "info" | "neutral";

/* ── Masking-tape palette ──
   Semi-transparent pastels that feel like real washi tape —
   the colour bleeds through slightly, edges aren't perfectly clean. */

const tapeColors: Record<Variant, string> = {
  ok: "var(--tape-ok)",
  error: "var(--tape-error)",
  degraded: "var(--tape-degraded)",
  info: "var(--tape-info)",
  neutral: "var(--tape-neutral)",
};

const tapeBackgrounds: Record<Variant, string> = {
  ok: "var(--tape-ok-bg)",
  error: "var(--tape-error-bg)",
  degraded: "var(--tape-degraded-bg)",
  info: "var(--tape-info-bg)",
  neutral: "var(--tape-neutral-bg)",
};

const tapeRotations: Record<Variant, string> = {
  ok: "-0.3deg",
  error: "0.3deg",
  degraded: "-0.2deg",
  info: "0.3deg",
  neutral: "-0.15deg",
};

interface StatusBadgeProps {
  variant: Variant;
  label: string;
  className?: string;
}

export function StatusBadge({ variant, label, className }: StatusBadgeProps) {
  const { t } = useI18n();

  return (
    <span
      role="status"
      className={cn("app-badge-enter inline-flex items-center gap-1.5", className)}
      style={
        {
          "--badge-rotation": tapeRotations[variant],
          padding: "3px 10px",
          fontSize: "10px",
          fontWeight: 600,
          fontFamily: "'Inter', 'Zen Maru Gothic', system-ui, sans-serif",
          letterSpacing: "0.03em",
          color: tapeColors[variant],
          background: tapeBackgrounds[variant],
          borderRadius: "2px",
          transform: `rotate(${tapeRotations[variant]})`,
          boxShadow: "0 1px 2px oklch(0.40 0.02 68 / 0.06)",
        } as React.CSSProperties
      }
    >
      {variant === "ok" ? (
        <InkCheck size={10} color={tapeColors[variant]} />
      ) : variant === "error" ? (
        <InkCross size={10} color={tapeColors[variant]} />
      ) : variant === "degraded" ? (
        <InkWavy size={10} color={tapeColors[variant]} />
      ) : variant === "info" ? (
        <InkCircle size={10} color={tapeColors[variant]} />
      ) : (
        <InkDash size={10} color={tapeColors[variant]} />
      )}
      <span>{t(label)}</span>
    </span>
  );
}
