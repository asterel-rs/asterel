// Route: what needs operator attention first across runtime health, sessions, memory, and channels?
import { useMutation, useQuery } from "@tanstack/react-query";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { ErrorState } from "@/components/error-state";
import { InkArrow } from "@/components/ink-marks";
import { PageHeader, PageShell, Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import {
  fetchActivityTimeline,
  fetchChannels,
  fetchGovernanceSummary,
  fetchMemoryEntities,
  fetchRuntime,
  fetchSessions,
  fetchUsage,
  restartGateway,
} from "@/lib/api";
import { formatDate, formatTokenCount } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { formatNumber } from "@/lib/i18n-core";
import type { ActivityEvent } from "@/lib/types";
import { getOpticalInlineControlStyle, getOpticalPanelInsetStyle } from "@/lib/ui-polish";
import { usePageTitle } from "@/lib/use-page-title";

export const Route = createFileRoute("/dashboard")({
  component: DashboardPage,
});

function DashboardPage() {
  const { locale, t } = useI18n();
  const navigate = useNavigate();

  usePageTitle("Dashboard");

  const [feedback, setFeedback] = useState<string | null>(null);

  const showFeedback = (msg: string) => {
    setFeedback(msg);
    setTimeout(() => setFeedback(null), 2000);
  };

  const restartMutation = useMutation({
    mutationFn: () => restartGateway(),
    onSuccess: () => showFeedback(t("Restart requested")),
    onError: () => showFeedback(t("Restart failed")),
  });

  const {
    data: runtime,
    isLoading,
    isError,
    error,
    refetch,
  } = useQuery({
    queryKey: ["runtime"],
    queryFn: fetchRuntime,
    refetchInterval: 15_000,
  });

  const usageQuery = useQuery({
    queryKey: ["usage"],
    queryFn: fetchUsage,
    refetchInterval: 60_000,
  });

  const sessionsQuery = useQuery({
    queryKey: ["sessions"],
    queryFn: fetchSessions,
    refetchInterval: 30_000,
  });

  const channelsQuery = useQuery({
    queryKey: ["channels"],
    queryFn: fetchChannels,
    refetchInterval: 60_000,
  });

  const memoryQuery = useQuery({
    queryKey: ["memory", "entities"],
    queryFn: fetchMemoryEntities,
    refetchInterval: 30_000,
  });

  const activityQuery = useQuery({
    queryKey: ["activity"],
    queryFn: fetchActivityTimeline,
    refetchInterval: 30_000,
  });

  const governanceQuery = useQuery({
    queryKey: ["governance-summary"],
    queryFn: fetchGovernanceSummary,
    refetchInterval: 60_000,
  });

  if (isLoading) {
    return <DashboardLoadingState />;
  }

  if (isError || !runtime) {
    return (
      <div className="flex h-full items-center justify-center p-6">
        <ErrorState
          title={t("Failed to load runtime status")}
          message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
          onRetry={() => refetch()}
        />
      </div>
    );
  }

  const capabilityEntries = Object.entries(runtime.capabilities);
  const wsLoadLabel = `${formatNumber(runtime.gateway.ws_connections, locale)} / ${formatNumber(runtime.gateway.max_ws_connections, locale)}`;
  const recentSessions = (sessionsQuery.data?.items ?? []).slice(0, 5);
  const channels = channelsQuery.data?.items ?? [];
  const activeChannels = channelsQuery.data?.active_names ?? [];
  const memoryEntities = memoryQuery.data?.items ?? [];
  const compactControlStyle = getOpticalInlineControlStyle({ density: "compact" });
  const trailingIconControlStyle = getOpticalInlineControlStyle({
    density: "compact",
    icon: "trailing",
  });
  const compactPanelInsetStyle = getOpticalPanelInsetStyle({ density: "compact" });

  return (
    <PageShell accent="var(--section-overview)">
      {feedback ? (
        <div
          aria-live="polite"
          className="app-badge-enter fixed right-6 top-6"
          style={{
            zIndex: 50,
            padding: "8px 16px",
            background: "var(--bg-panel)",
            border: "1px solid var(--border)",
            boxShadow: "var(--shadow-md)",
            fontSize: "12px",
            fontWeight: 600,
            color: "var(--accent-strong)",
          }}
        >
          {feedback}
        </div>
      ) : null}
      <PageHeader
        eyebrow={t("Overview")}
        title={t("Dashboard")}
        description={t(
          "Runtime health, channel posture, trust state, and recent conversation activity.",
        )}
        actions={
          <>
            <StatPill label={t("Release")} value={runtime.version} />
            <StatPill label={t("Model")} value={runtime.model} />
            <StatPill label={t("WS load")} value={wsLoadLabel} tone="var(--info)" />
            <StatPill
              label={t("Database")}
              value={runtime.db.engine}
              tone={runtime.db.status === "connected" ? "var(--accent-strong)" : "var(--error)"}
            />
            {usageQuery.data ? (
              <StatPill
                label={t("Tokens")}
                value={formatTokenCount(usageQuery.data.total_tokens)}
                hint={`${formatTokenCount(usageQuery.data.total_input_tokens)} in / ${formatTokenCount(usageQuery.data.total_output_tokens)} out`}
              />
            ) : null}
          </>
        }
        aside={
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => navigate({ to: "/sessions" })}
              className="ui-button ui-button-accent-fill text-xs font-bold"
              style={compactControlStyle}
            >
              {t("Open sessions")}
            </button>
            <button
              type="button"
              onClick={() => {
                refetch().then(() => showFeedback(t("Health OK")));
              }}
              className="ui-button ui-button-muted text-xs font-bold"
              style={compactControlStyle}
            >
              {t("Health check")}
            </button>
            <button
              type="button"
              onClick={() => restartMutation.mutate()}
              disabled={restartMutation.isPending}
              className="ui-button ui-button-warn-fill text-xs font-bold"
              style={compactControlStyle}
            >
              {restartMutation.isPending ? t("Restarting...") : t("Restart")}
            </button>
            <StatusBadge
              variant={runtime.status === "ok" ? "ok" : "degraded"}
              label={runtime.status === "ok" ? t("stable") : t("degraded")}
            />
          </div>
        }
      />

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_minmax(18rem,24rem)]">
        {/* ── Left column ── */}
        <div className="space-y-4">
          <Panel strong variant="stage" style={compactPanelInsetStyle}>
            <SectionLead title={t("Capabilities")} />
            <div className="mt-4 grid gap-2 md:grid-cols-2">
              {capabilityEntries.map(([key, enabled]) => (
                <div
                  key={key}
                  className="flex items-center justify-between gap-3 border-b border-[var(--border)] px-3 py-2"
                >
                  <p className="text-sm text-[var(--fg)]">{t(formatCapability(key))}</p>
                  <StatusBadge
                    variant={enabled ? "ok" : "neutral"}
                    label={enabled ? t("on") : t("off")}
                  />
                </div>
              ))}
            </div>
          </Panel>

          <Panel variant="stage" style={compactPanelInsetStyle}>
            <SectionLead
              title={t("Recent sessions")}
              {...(sessionsQuery.data
                ? {
                    description: t("{count} total", {
                      count: sessionsQuery.data.items.length,
                    }),
                  }
                : {})}
              action={
                <Link
                  to="/sessions"
                  className="ui-button ui-button-muted inline-flex items-center text-xs font-bold no-underline"
                  style={trailingIconControlStyle}
                >
                  {t("View all")}
                  <InkArrow size={11} color="currentColor" />
                </Link>
              }
            />
            <div className="mt-3">
              {recentSessions.length === 0 ? (
                <p className="text-muted py-4 text-center text-xs">{t("No sessions yet")}</p>
              ) : (
                recentSessions.map((session) => (
                  <Link
                    key={session.id}
                    to="/sessions/$sessionId"
                    params={{ sessionId: session.id }}
                    className="ui-ledger-link px-4 py-3"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0">
                        <p className="text-fg truncate text-sm" style={{ fontWeight: 500 }}>
                          {session.id.slice(0, 12)}…
                        </p>
                        <p className="text-muted mt-0.5 text-xs">
                          {session.surface} · {formatDate(session.updated_at)}
                        </p>
                      </div>
                      <StatusBadge
                        variant={
                          session.state === "active"
                            ? "ok"
                            : session.state === "closed"
                              ? "neutral"
                              : "degraded"
                        }
                        label={session.state}
                      />
                    </div>
                  </Link>
                ))
              )}
            </div>
          </Panel>
        </div>

        {/* ── Right column ── */}
        <div className="space-y-4">
          {/* Activity timeline */}
          {activityQuery.data && activityQuery.data.events.length > 0 ? (
            <Panel variant="stage" style={compactPanelInsetStyle}>
              <SectionLead
                title={t("Today's activity")}
                description={t("{count} events", {
                  count: activityQuery.data.count,
                })}
              />
              <div className="mt-3" style={{ position: "relative", paddingLeft: "20px" }}>
                <div
                  style={{
                    position: "absolute",
                    left: "6px",
                    top: "4px",
                    bottom: "4px",
                    width: "2px",
                    background: "color-mix(in oklch, var(--page-accent) 25%, var(--border))",
                  }}
                />
                {activityQuery.data.events.map((event: ActivityEvent) => (
                  <div key={`${event.kind}-${event.id}`} className="relative py-2">
                    <div
                      style={{
                        position: "absolute",
                        left: "-18px",
                        top: "12px",
                        width: "6px",
                        height: "6px",
                        borderRadius: "48% 52% 55% 45%",
                        background: event.state === "active" ? "var(--accent)" : "var(--fg-muted)",
                        opacity: event.state === "active" ? 1 : 0.5,
                      }}
                    />
                    <p className="text-fg text-xs" style={{ fontWeight: 500 }}>
                      {event.label}
                    </p>
                    <p
                      style={{
                        fontSize: "10px",
                        color: "var(--fg-muted)",
                        marginTop: "2px",
                      }}
                    >
                      {formatDate(event.timestamp)}
                    </p>
                  </div>
                ))}
              </div>
            </Panel>
          ) : null}

          <Panel variant="stage" style={compactPanelInsetStyle}>
            <SectionLead title={t("System pulse")} />
            <div className="mt-3 space-y-0">
              <PulseRow
                to="/memory"
                label={t("Memory entities")}
                value={String(memoryEntities.length)}
                variant={memoryEntities.length > 0 ? "ok" : "neutral"}
              />
              <PulseRow
                to="/channels"
                label={t("Channels")}
                value={`${activeChannels.length} / ${channels.length}`}
                variant={activeChannels.length > 0 ? "ok" : "neutral"}
              />
              <PulseRow
                to="/settings"
                label={t("Governance")}
                value={
                  governanceQuery.data
                    ? t("{count} windows", {
                        count: governanceQuery.data.runtime.companion_surface_windows,
                      })
                    : t("loading")
                }
                variant={
                  governanceQuery.data?.runtime.companion_surface_windows ? "degraded" : "ok"
                }
              />
              {usageQuery.data ? (
                <PulseRow
                  to="/sessions"
                  label={t("Messages")}
                  value={formatNumber(usageQuery.data.message_count, locale)}
                  variant="ok"
                />
              ) : null}
            </div>
          </Panel>

          {governanceQuery.data ? <GovernanceStatusWidget summary={governanceQuery.data} /> : null}
        </div>
      </div>
    </PageShell>
  );
}

function GovernanceStatusWidget({
  summary,
}: {
  summary: {
    runtime: { memory_backend: string; companion_surface_windows: number };
    domain_trust: Array<{ domain: string }>;
  };
}) {
  const { t } = useI18n();

  return (
    <Panel variant="stage" style={getOpticalPanelInsetStyle({ density: "compact" })}>
      <SectionLead title={t("Governance status")} />
      <div className="mt-3 space-y-0">
        <div className="flex items-center justify-between border-b border-[var(--border)] px-3 py-2">
          <span className="text-fg text-sm" style={{ fontWeight: 500 }}>
            {t("Memory backend")}
          </span>
          <span className="text-muted text-xs font-semibold">{summary.runtime.memory_backend}</span>
        </div>
        <div className="flex items-center justify-between border-b border-[var(--border)] px-3 py-2">
          <span className="text-fg text-sm" style={{ fontWeight: 500 }}>
            {t("Pending windows")}
          </span>
          <StatusBadge
            variant={summary.runtime.companion_surface_windows > 0 ? "degraded" : "ok"}
            label={String(summary.runtime.companion_surface_windows)}
          />
        </div>
        <div className="flex items-center justify-between border-b border-[var(--border)] px-3 py-2">
          <span className="text-fg text-sm" style={{ fontWeight: 500 }}>
            {t("Tracked domains")}
          </span>
          <span className="text-muted text-xs font-semibold">{summary.domain_trust.length}</span>
        </div>
      </div>
      <div className="mt-3">
        <Link
          to="/settings"
          className="ui-link-accent-hover text-xs font-semibold"
          style={{ color: "var(--fg-muted)" }}
        >
          {t("Open runtime settings")}
          <InkArrow size={10} color="currentColor" className="ml-1 inline-block" />
        </Link>
      </div>
    </Panel>
  );
}

function PulseRow({
  to,
  label,
  value,
  variant,
}: {
  to: "/memory" | "/channels" | "/sessions" | "/settings";
  label: string;
  value: string;
  variant: "ok" | "degraded" | "neutral";
}) {
  return (
    <Link
      to={to}
      search={{}}
      className="ui-ledger-link flex items-center justify-between gap-3 px-4 py-2.5"
    >
      <span className="text-fg text-sm" style={{ fontWeight: 500 }}>
        {label}
      </span>
      <div className="flex items-center gap-2">
        <span className="text-muted text-xs font-semibold">{value}</span>
        <StatusBadge variant={variant} label={variant} />
      </div>
    </Link>
  );
}

function DashboardLoadingState() {
  const { t } = useI18n();

  return (
    <PageShell>
      <div className="space-y-3">
        <span className="app-kicker">{t("Overview")}</span>
        <div className="ui-skeleton h-14 w-[440px]" />
        <div className="ui-skeleton h-4 w-[400px]" />
      </div>
      <SkeletonLoader />
    </PageShell>
  );
}

function formatCapability(key: string): string {
  if (key === "memory_review") return "memory review";
  if (key === "channel_posture") return "channel posture";
  if (key === "session_review") return "session review";
  return key.split("_").join(" ");
}
