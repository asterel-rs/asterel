import { formatDateTime, type Locale, translateNow } from "@/lib/i18n-core";

export function formatUptime(seconds: number): string {
  if (seconds < 60) return translateNow("{count}s", { count: seconds });
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return translateNow("{count}m", { count: minutes });
  const hours = Math.floor(minutes / 60);
  const remainMinutes = minutes % 60;
  return `${hours}h ${remainMinutes}m`;
}

export function formatTokenCount(value?: number): string {
  if (value === undefined) return "-";
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}

export function shortId(value: string): string {
  return value.length > 12 ? `${value.slice(0, 12)}...` : value;
}

export function formatDate(iso: string | null | undefined, locale?: Locale): string {
  if (!iso) return "---";
  try {
    return formatDateTime(
      iso,
      {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      },
      locale,
    );
  } catch {
    return iso;
  }
}
