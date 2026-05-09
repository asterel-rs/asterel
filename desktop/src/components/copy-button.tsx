import { useCallback, useState } from "react";
import { InkCheck } from "@/components/ink-marks";
import { useI18n } from "@/lib/i18n";

interface CopyButtonProps {
  text: string;
}

export function CopyButton({ text }: CopyButtonProps) {
  const { t } = useI18n();
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(() => {
    navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => {});
  }, [text]);

  return (
    <button
      type="button"
      onClick={handleCopy}
      className="ui-copy-button"
      title={t("Copy")}
      aria-label={t("Copy")}
      style={copied ? { opacity: 1 } : undefined}
    >
      {copied ? (
        <InkCheck size={13} color="var(--accent)" />
      ) : (
        <svg
          width="12"
          height="12"
          viewBox="0 0 16 16"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <rect x="5.5" y="5.5" width="8" height="8" rx="1" />
          <path d="M10.5 5.5 V3.5 a1 1 0 0 0 -1 -1 H3.5 a1 1 0 0 0 -1 1 v6 a1 1 0 0 0 1 1 H5.5" />
        </svg>
      )}
    </button>
  );
}
