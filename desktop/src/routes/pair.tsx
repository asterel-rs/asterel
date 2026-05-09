import { useQuery } from "@tanstack/react-query";
// Route: how does this desktop pair with the running companion daemon?
import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { type ReactNode, useCallback, useRef, useState } from "react";
import { StatusBadge } from "@/components/status-badge";
import { healthCheck, pairWithDaemon } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { useConnectionStore } from "@/stores/connection";

export const Route = createFileRoute("/pair")({
  component: PairPage,
});

function PairPage() {
  const { t } = useI18n();

  usePageTitle("Connect");

  const navigate = useNavigate();
  const setToken = useConnectionStore((s) => s.setToken);
  const setStatus = useConnectionStore((s) => s.setStatus);
  const currentStatus = useConnectionStore((s) => s.status);

  const [code, setCode] = useState("");
  const [pairing, setPairing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const healthQuery = useQuery({
    queryKey: ["health"],
    queryFn: healthCheck,
    refetchInterval: 5000,
    retry: false,
  });

  const daemonReachable = healthQuery.isSuccess;
  const daemonLoading = healthQuery.isLoading;

  const handlePair = useCallback(async () => {
    if (!code.trim() || pairing) return;
    setPairing(true);
    setError(null);
    setStatus("connecting");

    try {
      const res = await pairWithDaemon(code.trim());
      if (res.status >= 400) {
        throw new Error(t("Pairing rejected by daemon"));
      }
      setToken(res.body.token);
      navigate({ to: "/dashboard" });
    } catch (err) {
      const message = err instanceof Error ? err.message : t("Pairing failed");
      setError(message);
      setStatus("error");
    } finally {
      setPairing(false);
    }
  }, [code, pairing, setToken, setStatus, navigate, t]);

  const focusInput = () => inputRef.current?.focus();

  return (
    <div className="app-page flex min-h-full items-center px-6 py-10 md:px-8">
      <div className="grid w-full gap-10 xl:grid-cols-[minmax(0,1fr)_minmax(22rem,30rem)]">
        <section className="flex flex-col justify-between py-6">
          <div className="space-y-8">
            <span className="app-kicker">{t("Secure link")}</span>

            <div className="space-y-4">
              <h1
                className="font-display"
                style={{
                  fontSize: "clamp(3rem, 6vw, 5.3rem)",
                  fontWeight: 700,
                  letterSpacing: "-0.07em",
                  lineHeight: 0.92,
                  color: "var(--fg)",
                  maxWidth: "12ch",
                }}
              >
                {t("Link this desktop with your daemon.")}
              </h1>

              <p
                style={{
                  maxWidth: "52rem",
                  fontSize: "15px",
                  lineHeight: 1.9,
                  color: "var(--fg-soft)",
                }}
              >
                {t(
                  "Enter the pairing code from your daemon to connect. Once linked, you can move straight into the operator console.",
                )}
              </p>
            </div>

            <div className="space-y-5 border-t pt-6" style={{ borderColor: "var(--border)" }}>
              <PairStep
                index="01"
                title={t("Start the local gateway")}
                body={t(
                  "Wait for the daemon to expose its health check before entering the handoff code.",
                )}
              />
              <PairStep
                index="02"
                title={t("Use the six-digit terminal code")}
                body={t(
                  "The desktop only accepts the numeric pairing code printed by the running daemon.",
                )}
              />
              <PairStep
                index="03"
                title={t("Open the operator console")}
                body={t(
                  "As soon as the link is established, the dashboard becomes the entry point for sessions, memory, and channel review.",
                )}
              />
            </div>
          </div>

          <div
            className="mt-10 grid gap-5 border-t pt-6 md:grid-cols-2"
            style={{ borderColor: "var(--border)" }}
          >
            <StatusRail
              label={t("Daemon line")}
              description={t(
                "The gateway needs to answer health checks before pairing can succeed.",
              )}
              badge={
                daemonLoading ? (
                  <StatusBadge variant="neutral" label={t("checking")} />
                ) : daemonReachable ? (
                  <StatusBadge variant="ok" label={t("reachable")} />
                ) : (
                  <StatusBadge variant="error" label={t("unreachable")} />
                )
              }
            />
            <StatusRail
              label={t("Desktop link")}
              description={t(
                "The local desktop stores the daemon token after a successful handoff.",
              )}
              badge={
                <StatusBadge
                  variant={
                    currentStatus === "connected"
                      ? "ok"
                      : currentStatus === "error"
                        ? "error"
                        : currentStatus === "connecting"
                          ? "info"
                          : "neutral"
                  }
                  label={t(currentStatus)}
                />
              }
            />
          </div>
        </section>

        <section
          className="app-stage-strong flex flex-col justify-between px-7 py-7"
          style={{ borderLeft: "1px solid var(--border)" }}
        >
          <div className="space-y-6">
            <div className="space-y-2">
              <p className="app-section-title">{t("Pairing gate")}</p>
              <p className="text-sm" style={{ color: "var(--fg-soft)", lineHeight: 1.85 }}>
                {t("Enter the six-digit code shown in your daemon's terminal.")}
              </p>
            </div>

            <div>
              <input
                ref={inputRef}
                type="text"
                inputMode="numeric"
                autoComplete="one-time-code"
                value={code}
                onChange={(e) => {
                  const val = e.target.value.replace(/\D/g, "").slice(0, 6);
                  setCode(val);
                  if (error) setError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") handlePair();
                }}
                className="sr-only"
                aria-label={t("6-digit pairing code")}
              />

              <button
                type="button"
                onClick={focusInput}
                aria-label={t("Focus pairing code input")}
                className="w-full cursor-text"
                style={{
                  background: "transparent",
                  border: "none",
                  padding: 0,
                }}
              >
                <div className="grid grid-cols-6 gap-3">
                  {[0, 1, 2, 3, 4, 5].map((i) => (
                    <DigitCell
                      key={`d${i}`}
                      digit={i < code.length ? code[i] : undefined}
                      isActive={i === code.length}
                      hasError={!!error}
                    />
                  ))}
                </div>
              </button>
            </div>

            {error ? (
              <div
                style={{
                  border: "1px solid color-mix(in oklch, var(--error) 32%, transparent)",
                  background: "var(--error-soft)",
                  color: "var(--error)",
                  padding: "12px 14px",
                  fontSize: "12px",
                  lineHeight: 1.65,
                  borderRadius: "var(--radius-md)",
                }}
              >
                {error}
              </div>
            ) : null}

            <div className="grid gap-3">
              <button
                type="button"
                onClick={handlePair}
                disabled={code.length < 6 || pairing || !daemonReachable}
                className="ui-button ui-button-accent-fill flex w-full items-center justify-center py-3 text-xs font-bold uppercase"
                style={{
                  letterSpacing: "0.16em",
                }}
              >
                {pairing
                  ? t("Connecting...")
                  : currentStatus === "connected"
                    ? t("Connected")
                    : t("Connect desktop")}
              </button>

              {currentStatus === "connected" ? (
                <button
                  type="button"
                  onClick={() => navigate({ to: "/dashboard" })}
                  className="ui-button ui-button-muted flex w-full items-center justify-center py-3 text-xs font-bold uppercase"
                  style={{
                    letterSpacing: "0.16em",
                    color: "var(--fg)",
                  }}
                >
                  {t("Open console")}
                </button>
              ) : null}
            </div>
          </div>

          <div className="mt-8 border-t pt-5" style={{ borderColor: "var(--border)" }}>
            <p className="app-section-title">{t("Memo")}</p>
            <p className="mt-3 text-xs" style={{ color: "var(--fg-muted)", lineHeight: 1.9 }}>
              {t(
                "If the daemon is still unreachable, keep the pairing panel open and start the local gateway first. The code can only be redeemed while that daemon instance is alive.",
              )}
            </p>
          </div>
        </section>
      </div>
    </div>
  );
}

function PairStep({ index, title, body }: { index: string; title: string; body: string }) {
  return (
    <div className="grid gap-3 md:grid-cols-[72px_minmax(0,1fr)]">
      <span className="ui-chip w-fit">{index}</span>
      <div>
        <p className="text-fg text-sm" style={{ fontWeight: 600 }}>
          {title}
        </p>
        <p className="mt-2 text-sm" style={{ color: "var(--fg-soft)", lineHeight: 1.85 }}>
          {body}
        </p>
      </div>
    </div>
  );
}

function StatusRail({
  label,
  description,
  badge,
}: {
  label: string;
  description: string;
  badge: ReactNode;
}) {
  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-3">
        <p className="app-section-title">{label}</p>
        {badge}
      </div>
      <p className="text-sm" style={{ color: "var(--fg-soft)", lineHeight: 1.85 }}>
        {description}
      </p>
    </div>
  );
}

function DigitCell({
  digit,
  isActive,
  hasError,
}: {
  digit: string | undefined;
  isActive: boolean;
  hasError: boolean;
}) {
  const isFilled = digit != null;
  const borderColor =
    isFilled && hasError
      ? "var(--error)"
      : isFilled
        ? "var(--accent-strong)"
        : isActive
          ? "var(--info)"
          : "var(--border)";

  const textColor =
    isFilled && hasError ? "var(--error)" : isFilled ? "var(--fg)" : "var(--fg-muted)";

  return (
    <div
      className={cn("flex h-20 select-none items-center justify-center font-mono text-3xl")}
      style={{
        border: `1px solid ${borderColor}`,
        borderRadius: "var(--radius-md)",
        background: isFilled
          ? "var(--bg-panel)"
          : isActive
            ? "var(--info-soft)"
            : "var(--bg-raised)",
        color: textColor,
        fontWeight: 700,
        boxShadow: isFilled ? "var(--shadow-sm)" : "none",
      }}
    >
      {isFilled ? (
        digit
      ) : isActive ? (
        <span
          className="animate-blink"
          style={{
            display: "inline-block",
            width: "2px",
            height: "28px",
            background: "var(--info)",
          }}
        />
      ) : null}
    </div>
  );
}
