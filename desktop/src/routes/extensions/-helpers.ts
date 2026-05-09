import { translateNow } from "@/lib/i18n-core";

export function formatRelativeTime(iso: string): string {
  try {
    const target = new Date(iso).getTime();
    const now = Date.now();
    const diffMs = target - now;

    if (Number.isNaN(target)) return iso;

    const absDiff = Math.abs(diffMs);
    const isPast = diffMs < 0;

    if (absDiff < 60_000) {
      return isPast ? translateNow("just now") : translateNow("in < 1m");
    }

    const minutes = Math.floor(absDiff / 60_000);
    if (minutes < 60) {
      return isPast
        ? translateNow("{count}m ago", { count: minutes })
        : translateNow("in {count}m", { count: minutes });
    }

    const hours = Math.floor(minutes / 60);
    if (hours < 24) {
      return isPast
        ? translateNow("{count}h ago", { count: hours })
        : translateNow("in {count}h", { count: hours });
    }

    const days = Math.floor(hours / 24);
    return isPast
      ? translateNow("{count}d ago", { count: days })
      : translateNow("in {count}d", { count: days });
  } catch {
    return iso;
  }
}

export function statusVariant(status?: string): "ok" | "degraded" | "error" | "neutral" {
  if (!status) return "neutral";
  if (status === "ok" || status === "success") return "ok";
  if (status === "error" || status === "failed") return "error";
  return "degraded";
}
