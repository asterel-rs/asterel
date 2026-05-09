// Route: which recent conversations need review, and on which runtime surface did they happen?
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute, Link, useNavigate } from "@tanstack/react-router";
import { memo } from "react";
import { ConfirmAction } from "@/components/confirm-action";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader, PageShell, Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { createSession, deleteSession, fetchSessions } from "@/lib/api";
import { formatDate } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { getOpticalInlineControlStyle, getOpticalPanelInsetStyle } from "@/lib/ui-polish";
import { usePageTitle } from "@/lib/use-page-title";

export const Route = createFileRoute("/sessions/")({
  component: SessionsPage,
});

type SessionRowVariant = "ok" | "degraded" | "error" | "neutral";

interface SessionRowViewModel {
  id: string;
  shortId: string;
  channel: string;
  userLabel: string;
  stateLabel: string;
  stateVariant: SessionRowVariant;
  createdAtLabel: string;
  updatedAtLabel: string;
}

function SessionsPage() {
  const { t } = useI18n();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  usePageTitle("Sessions");

  const createMutation = useMutation({
    mutationFn: () => createSession(),
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
      navigate({
        to: "/sessions/$sessionId",
        params: { sessionId: session.id },
      });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteSession(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["sessions"],
    queryFn: fetchSessions,
    refetchInterval: 10_000,
    select: (response): SessionRowViewModel[] =>
      response.items.map((session) => {
        const stateLabel = session.state ?? "unknown";
        const ownerScope = session.owner_scope ?? "";

        return {
          id: session.id,
          shortId: `${session.id.slice(0, 12)}...`,
          channel: session.surface ?? "unknown",
          userLabel: ownerScope ? `${ownerScope.slice(0, 14)}...` : t("shared"),
          stateLabel,
          stateVariant: stateVariant(stateLabel),
          createdAtLabel: formatDate(session.created_at),
          updatedAtLabel: formatDate(session.updated_at),
        };
      }),
  });

  const sessions = data ?? [];
  const activeCount = sessions.filter((session) => session.stateVariant === "ok").length;
  const issueCount = sessions.filter((session) => session.stateVariant === "error").length;
  const channelSummary = summarizeChannels(sessions);
  const busiestChannel = channelSummary[0];
  const compactControlStyle = getOpticalInlineControlStyle({ density: "compact" });
  const compactPanelInsetStyle = getOpticalPanelInsetStyle({ density: "compact" });

  return (
    <PageShell accent="var(--section-operations)">
      <PageHeader
        eyebrow={t("Session management")}
        title={t("Sessions")}
        description={t("Live and recent conversations across channels.")}
        actions={
          <>
            <StatPill label={t("Sessions")} value={String(sessions.length)} />
            <StatPill label={t("Active")} value={String(activeCount)} tone="var(--accent-strong)" />
            <StatPill label={t("Issues")} value={String(issueCount)} tone="var(--error)" />
            {busiestChannel ? (
              <StatPill
                label={t("Top channel")}
                value={busiestChannel.label}
                hint={t("{count} live sessions", { count: busiestChannel.count })}
              />
            ) : null}
          </>
        }
        aside={
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => createMutation.mutate()}
              disabled={createMutation.isPending}
              className="ui-button ui-button-accent-fill text-xs font-bold uppercase"
              style={compactControlStyle}
            >
              {createMutation.isPending ? t("Creating...") : t("New session")}
            </button>
            <button
              type="button"
              onClick={() => refetch()}
              className="ui-button ui-button-muted text-xs font-bold uppercase text-fg"
              style={compactControlStyle}
            >
              {t("Refresh view")}
            </button>
          </div>
        }
      />

      {isLoading ? (
        <SkeletonLoader />
      ) : isError ? (
        <ErrorState
          title={t("Failed to load sessions")}
          message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
          onRetry={() => refetch()}
        />
      ) : sessions.length === 0 ? (
        <EmptyState
          title={t("No sessions yet")}
          description={t(
            "Sessions appear when conversations start through any connected surface. Check that at least one channel is enabled in Channels.",
          )}
        />
      ) : (
        <>
          <Panel strong variant="stage" style={compactPanelInsetStyle}>
            <SectionLead
              title={t("Live sessions")}
              description={t("Rows update every 10 seconds.")}
            />
            <div className="mt-4">
              {sessions.map((session) => (
                <SessionRow
                  key={session.id}
                  session={session}
                  onDeleteConfirm={(id) => deleteMutation.mutate(id)}
                  isDeleting={deleteMutation.isPending && deleteMutation.variables === session.id}
                />
              ))}
            </div>
          </Panel>
        </>
      )}
    </PageShell>
  );
}

const SessionRow = memo(function SessionRow({
  session,
  onDeleteConfirm,
  isDeleting,
}: {
  session: SessionRowViewModel;
  onDeleteConfirm: (id: string) => void;
  isDeleting: boolean;
}) {
  const { t } = useI18n();

  return (
    <div className="ui-ledger-card px-4 py-4">
      <Link
        to="/sessions/$sessionId"
        params={{ sessionId: session.id }}
        className="block no-underline"
        style={{ textDecoration: "none" }}
      >
        <div className="ui-ledger-columns">
          <div className="ui-ledger-cell">
            <p className="app-section-title">{t("Session ID")}</p>
            <p className="text-fg mt-2 text-sm" style={{ lineHeight: 1.65 }}>
              {session.shortId}
            </p>
          </div>
          <SessionMeta label={t("Channel")} value={session.channel} className="ui-ledger-cell" />
          <SessionMeta label={t("User")} value={session.userLabel} className="ui-ledger-cell" />
          <SessionMeta
            label={t("Created")}
            value={session.createdAtLabel}
            className="ui-ledger-cell"
          />
          <SessionMeta
            label={t("Updated")}
            value={session.updatedAtLabel}
            className="ui-ledger-cell"
          />
          <div className="ui-ledger-cell flex items-start justify-between gap-3 xl:justify-end">
            <div className="flex flex-col items-start gap-3 xl:items-end">
              <StatusBadge variant={session.stateVariant} label={session.stateLabel} />
              <span
                className="text-xs"
                style={{
                  color: "var(--fg-muted)",
                  letterSpacing: "0.08em",
                  textTransform: "uppercase",
                }}
              >
                {t("Open transcript")}
              </span>
            </div>
          </div>
        </div>
      </Link>
      <div className="flex items-center justify-end gap-2 pt-2">
        <ConfirmAction
          trigger={t("Delete")}
          onConfirm={() => onDeleteConfirm(session.id)}
          isPending={isDeleting}
          confirmLabel={t("Delete this session")}
        />
      </div>
    </div>
  );
});

function SessionMeta({
  label,
  value,
  className,
}: {
  label: string;
  value: string;
  className?: string;
}) {
  return (
    <div
      className={className}
      style={{
        paddingTop: "2px",
      }}
    >
      <p className="app-section-title">{label}</p>
      <p className="mt-2 text-sm" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function summarizeChannels(
  sessions: SessionRowViewModel[],
): Array<{ label: string; count: number }> {
  const counts = new Map<string, number>();

  sessions.forEach((session) => {
    counts.set(session.channel, (counts.get(session.channel) ?? 0) + 1);
  });

  return [...counts.entries()]
    .map(([label, count]) => ({ label, count }))
    .sort((left, right) => right.count - left.count);
}

function stateVariant(state: string): "ok" | "degraded" | "error" | "neutral" {
  switch (state) {
    case "active":
      return "ok";
    case "idle":
      return "neutral";
    case "error":
      return "error";
    default:
      return "neutral";
  }
}
