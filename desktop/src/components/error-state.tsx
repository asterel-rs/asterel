import { useI18n } from "@/lib/i18n";
import { InkBrokenPencil } from "./ink-marks";

interface ErrorStateProps {
  title: string;
  message: string;
  onRetry?: () => void;
}

export function ErrorState({ title, message, onRetry }: ErrorStateProps) {
  const { t } = useI18n();

  return (
    <div className="app-panel mx-auto flex max-w-xl flex-col items-center justify-center px-6 py-16 text-center">
      <InkBrokenPencil
        size={52}
        color="var(--error)"
        className="app-error-wobble mb-4 opacity-50"
      />
      <p className="font-display text-[22px] font-semibold tracking-[-0.02em] text-[var(--error)]">
        {t(title)}
      </p>
      <p className="text-muted mt-3 max-w-[420px] text-[13px] leading-[1.8]">{t(message)}</p>
      {onRetry && (
        <button
          type="button"
          onClick={onRetry}
          className="ui-button ui-button-error-fill ui-meta-label mt-4 px-3 py-1 uppercase"
        >
          {t("Retry")}
        </button>
      )}
    </div>
  );
}
