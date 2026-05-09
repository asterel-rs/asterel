import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { cancelCompanionWindow, confirmCompanionWindow, fetchGovernanceSummary } from "@/lib/api";
import { formatDate } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import type { GovernanceSummary } from "@/lib/types";

export function GovernanceTab() {
  const { t } = useI18n();
  const queryClient = useQueryClient();

  const {
    data: summary,
    isLoading,
    isError,
    error,
    refetch,
  } = useQuery({
    queryKey: ["governance-summary"],
    queryFn: fetchGovernanceSummary,
    refetchInterval: 30_000,
  });

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError || !summary) {
    return (
      <ErrorState
        title={t("Failed to load governance data")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex flex-wrap items-center gap-3">
        <StatPill label={t("Memory backend")} value={summary.runtime.memory_backend} />
        <StatPill
          label={t("Pending windows")}
          value={String(summary.runtime.companion_surface_windows)}
          tone={summary.runtime.companion_surface_windows > 0 ? "var(--warn)" : "var(--accent)"}
        />
        <StatPill label={t("Tracked domains")} value={String(summary.domain_trust.length)} />
      </div>

      <GovernanceRuntimeSection summary={summary} />

      <DomainTrustSection summary={summary} />

      <PendingWindowsSection
        summary={summary}
        onRefresh={() => queryClient.invalidateQueries({ queryKey: ["governance-summary"] })}
      />
    </div>
  );
}

function DomainTrustSection({ summary }: { summary: GovernanceSummary }) {
  const { t } = useI18n();

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Runtime domain trust")}
        description={t(
          "Live trust posture for companion-runtime domains such as tools and connectors.",
        )}
      />

      {summary.domain_trust.length === 0 ? (
        <p className="text-sm text-[var(--fg-muted)]">
          {t("No runtime trust signals have accumulated yet.")}
        </p>
      ) : (
        <div className="ui-rule-list mt-4">
          {summary.domain_trust.map((entry) => (
            <div key={entry.domain} className="ui-rule-row">
              <div>
                <p className="ui-rule-key">{entry.domain}</p>
                <p className="mt-1 text-xs text-[var(--fg-muted)]">
                  {t("success {success} / violations {violations}", {
                    success: entry.success_count,
                    violations: entry.violation_count,
                  })}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <span className="ui-rule-value">{entry.score.toFixed(2)}</span>
                <StatusBadge
                  variant={entry.score >= 0.75 ? "ok" : entry.score >= 0.45 ? "degraded" : "error"}
                  label={entry.autonomy}
                />
              </div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function GovernanceRuntimeSection({ summary }: { summary: GovernanceSummary }) {
  const { t } = useI18n();

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Runtime governance")}
        description={t("Operator-facing trust, memory review, and companion approval posture.")}
      />

      <div className="ui-rule-list mt-4">
        <TrustRow label={t("Memory backend")} value={summary.runtime.memory_backend} />
        <div className="ui-rule-row">
          <p className="ui-rule-key">{t("Memory review")}</p>
          <StatusBadge
            variant={summary.runtime.memory_review ? "ok" : "neutral"}
            label={summary.runtime.memory_review ? t("available") : t("offline")}
          />
        </div>
        <TrustRow
          label={t("Pending companion windows")}
          value={String(summary.runtime.companion_surface_windows)}
          tone={summary.runtime.companion_surface_windows > 0 ? "var(--warn)" : "var(--fg-soft)"}
        />
        <TrustRow
          label={t("Companion scopes")}
          value={String(summary.runtime.companion_surface_scopes)}
        />
      </div>
    </Panel>
  );
}

function PendingWindowsSection({
  summary,
  onRefresh,
}: {
  summary: GovernanceSummary;
  onRefresh: () => void;
}) {
  const { t } = useI18n();

  const confirmMutation = useMutation({
    mutationFn: ({ scope, windowId }: { scope: string; windowId: string }) =>
      confirmCompanionWindow(scope, windowId),
    onSuccess: () => onRefresh(),
  });
  const cancelMutation = useMutation({
    mutationFn: ({ scope, windowId }: { scope: string; windowId: string }) =>
      cancelCompanionWindow(scope, windowId),
    onSuccess: () => onRefresh(),
  });

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Pending companion approvals")}
        description={t("Confirm or cancel request windows from the main governance surface.")}
      />

      {summary.pending_windows.length === 0 ? (
        <p className="text-sm text-[var(--fg-muted)]">
          {t("No companion approvals are waiting right now.")}
        </p>
      ) : (
        <div className="mt-4 space-y-3">
          {summary.pending_windows.map((windowEntry) => (
            <div key={windowEntry.window_id} className="ui-ledger-card px-4 py-4">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <p className="app-section-title">{windowEntry.scope}</p>
                  <p className="mt-2 text-sm text-[var(--fg)]">{windowEntry.requested_action}</p>
                  <p className="mt-2 text-xs text-[var(--fg-muted)]">
                    {t("Created {value}", { value: formatDate(windowEntry.created_at) })}
                  </p>
                  <p className="mt-1 text-xs text-[var(--fg-muted)]">
                    {t("Expires {value}", { value: formatDate(windowEntry.expires_at) })}
                  </p>
                </div>
                <StatusBadge variant="degraded" label={t("pending")} />
              </div>
              <div className="mt-3 flex flex-wrap gap-2">
                <button
                  type="button"
                  onClick={() =>
                    confirmMutation.mutate({
                      scope: windowEntry.scope,
                      windowId: windowEntry.window_id,
                    })
                  }
                  disabled={confirmMutation.isPending}
                  className="ui-button ui-button-accent-fill px-3 py-1 text-xs font-bold uppercase"
                >
                  {confirmMutation.isPending ? t("Confirming...") : t("Confirm")}
                </button>
                <button
                  type="button"
                  onClick={() =>
                    cancelMutation.mutate({
                      scope: windowEntry.scope,
                      windowId: windowEntry.window_id,
                    })
                  }
                  disabled={cancelMutation.isPending}
                  className="ui-button ui-button-muted px-3 py-1 text-xs font-bold uppercase"
                  style={{ color: "var(--error)" }}
                >
                  {cancelMutation.isPending ? t("Cancelling...") : t("Cancel")}
                </button>
              </div>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

function TrustRow({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{label}</p>
      <p className="ui-rule-value" style={{ color: tone ?? "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}
