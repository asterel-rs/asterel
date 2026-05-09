import { type ReactNode, useState } from "react";
import { useI18n } from "@/lib/i18n";

export function ConfirmAction({
  trigger,
  onConfirm,
  confirmLabel,
  cancelLabel,
  isPending,
  variant = "error",
}: {
  trigger: ReactNode;
  onConfirm: () => void;
  confirmLabel?: string;
  cancelLabel?: string;
  isPending?: boolean;
  variant?: "error" | "warn";
}) {
  const { t } = useI18n();
  const [confirming, setConfirming] = useState(false);

  if (!confirming) {
    return (
      <button
        type="button"
        onClick={() => setConfirming(true)}
        className={`ui-button ${variant === "error" ? "ui-button-error-soft" : "ui-button-muted"} px-3 py-1 text-xs font-bold uppercase`}
      >
        {trigger}
      </button>
    );
  }

  const btnClass =
    variant === "error" ? "ui-button ui-button-error-fill" : "ui-button ui-button-warn-fill";

  return (
    <div className="flex flex-wrap items-center gap-2">
      <span
        className="text-xs"
        style={{ color: variant === "error" ? "var(--error)" : "var(--warn)" }}
      >
        {confirmLabel ? `${confirmLabel}?` : t("Are you sure?")}
      </span>
      <button
        type="button"
        onClick={() => {
          onConfirm();
          setConfirming(false);
        }}
        disabled={isPending}
        className={`${btnClass} px-3 py-1 text-xs font-bold uppercase`}
      >
        {isPending ? t("Processing...") : t("Confirm")}
      </button>
      <button
        type="button"
        onClick={() => setConfirming(false)}
        className="ui-button ui-button-muted px-3 py-1 text-xs font-bold uppercase"
      >
        {cancelLabel ?? t("Cancel")}
      </button>
    </div>
  );
}
