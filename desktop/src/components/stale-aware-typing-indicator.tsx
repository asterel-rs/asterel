import { useEffect, useState } from "react";
import { useI18n } from "@/lib/i18n";
import { useChatStore } from "@/stores/chat";

/** Irregular ink-drop shapes for the typing dots */
const INK_DROP_RADII = ["48% 52% 55% 45%", "52% 48% 45% 55%", "45% 55% 52% 48%"];

export function StaleAwareTypingIndicator({ onCancel }: { onCancel: () => void }) {
  const { t } = useI18n();
  const typingSince = useChatStore((s) => s.typingSince);
  const [elapsed, setElapsed] = useState(0);

  useEffect(() => {
    if (!typingSince) return;
    const interval = setInterval(() => {
      setElapsed(Math.floor((Date.now() - typingSince) / 1000));
    }, 1000);
    return () => clearInterval(interval);
  }, [typingSince]);

  const isStale = elapsed > 90;
  const showTimer = elapsed > 10;

  return (
    <div style={{ padding: "8px 0 8px 16px" }} aria-live="polite">
      <div className="chat-agent-border inline-flex items-center gap-2 py-1">
        {[0, 1, 2].map((i) => (
          <span
            key={i}
            className="animate-typing-dot"
            style={{
              display: "inline-block",
              width: "5px",
              height: "5px",
              borderRadius: INK_DROP_RADII[i],
              background: isStale ? "var(--error)" : "var(--fg-muted)",
              animationDelay: `${i * 150}ms`,
            }}
          />
        ))}
        {showTimer ? (
          <span
            className={isStale ? "text-error" : "text-muted"}
            style={{
              fontSize: "10px",
              marginLeft: "4px",
            }}
          >
            {elapsed}s
          </span>
        ) : null}
        {isStale ? (
          <>
            <span className="text-error" style={{ fontSize: "11px" }}>
              {t("Agent may be stuck")}
            </span>
            <button
              type="button"
              className="ui-button ui-button-ink text-error"
              onClick={onCancel}
              style={{ fontSize: "11px", padding: "2px 8px" }}
            >
              {t("Cancel")}
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
