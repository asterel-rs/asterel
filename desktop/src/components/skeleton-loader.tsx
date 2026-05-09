import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";

interface SkeletonLoaderProps {
  className?: string;
}

export function SkeletonLoader({ className }: SkeletonLoaderProps) {
  const { t } = useI18n();

  return (
    <output className={cn("block", className)} aria-label={t("Loading")}>
      <div className="app-panel space-y-3 px-5 py-5">
        <div className="ui-meta-label uppercase-label">{t("Loading interface")}</div>
        <span className="ui-skeleton w-40" />
        <span className="ui-skeleton w-full" />
        <span className="ui-skeleton w-3/4" />
      </div>
    </output>
  );
}
