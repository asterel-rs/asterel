import { useI18n } from "@/lib/i18n";
import { InkEmptyPage } from "./ink-marks";

interface EmptyStateProps {
  title: string;
  description: string;
  action?: {
    label: string;
    onClick: () => void;
  };
}

export function EmptyState({ title, description, action }: EmptyStateProps) {
  const { t } = useI18n();

  return (
    <div className="app-panel mx-auto flex max-w-xl flex-col items-center justify-center px-6 py-16 text-center">
      <InkEmptyPage size={52} color="var(--fg-muted)" className="app-empty-bob mb-4 opacity-50" />
      <p className="font-display text-[22px] font-semibold tracking-[-0.02em]">{t(title)}</p>
      <p className="text-muted mt-3 max-w-[420px] text-[13px] leading-[1.8]">{t(description)}</p>
      {action && (
        <button
          type="button"
          onClick={action.onClick}
          className="ui-button ui-button-accent-hint ui-meta-label mt-4 px-3 py-1 uppercase"
        >
          [{t(action.label)}]
        </button>
      )}
    </div>
  );
}
