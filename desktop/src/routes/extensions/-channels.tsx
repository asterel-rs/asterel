import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { channelAction, fetchChannels, patchChannel } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { Channel } from "@/lib/types";

// ---------------------------------------------------------------------------
// Channels tab — inventory + inspector workbench
// ---------------------------------------------------------------------------

export function ChannelsTab() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [selectedChannelId, setSelectedChannelId] = useState<string | null>(null);
  const [actionResult, setActionResult] = useState<{
    channelName: string;
    action: string;
    result?: string;
    detail?: string;
    error?: string;
  } | null>(null);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["channels"],
    queryFn: fetchChannels,
  });

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["channels"] });
  }, [queryClient]);

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      patchChannel(id, { enabled }),
    onSuccess: () => invalidate(),
  });

  const actionMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "doctor" | "test"; channelName: string }) =>
      channelAction(id, action),
    onSuccess: (res: { status: string; result?: string; detail?: string }, variables) => {
      setActionResult({
        channelName: variables.channelName,
        action: variables.action,
        result: res.result ?? res.status,
        ...(res.detail != null ? { detail: res.detail } : {}),
      });
      invalidate();
    },
    onError: (err, variables) => {
      setActionResult({
        channelName: variables.channelName,
        action: variables.action,
        error: err instanceof Error ? err.message : t("Action failed"),
      });
    },
  });

  const channelItems = data?.items;
  const channels = channelItems ?? [];
  const activeNames = data?.active_names ?? [];
  const highFreedom = data?.high_freedom ?? false;
  const selectedChannel = channels.find((ch) => ch.id === selectedChannelId) ?? null;

  useEffect(() => {
    const nextChannels = channelItems ?? [];

    if (nextChannels.length === 0) {
      if (selectedChannelId !== null) {
        setSelectedChannelId(null);
      }
      return;
    }

    if (!selectedChannelId || !nextChannels.some((ch) => ch.id === selectedChannelId)) {
      setSelectedChannelId(nextChannels[0]?.id ?? null);
    }
  }, [channelItems, selectedChannelId]);

  if (isLoading) return <SkeletonLoader />;

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load channels")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  if (channels.length === 0) {
    return (
      <EmptyState
        title={t("No channels available")}
        description={t(
          "Configure channels in the daemon config file. Supported types include Discord, Slack, and HTTP webhook.",
        )}
      />
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap gap-3">
        <StatPill label={t("Channels")} value={String(channels.length)} />
        <StatPill
          label={t("Active")}
          value={String(activeNames.length)}
          tone="var(--accent-strong)"
        />
        <StatPill
          label={t("High freedom")}
          value={highFreedom ? t("Enabled") : t("Disabled")}
          tone={highFreedom ? "var(--warn)" : "var(--fg)"}
        />
      </div>

      {actionResult ? (
        <Panel variant="stage" className="px-5 py-5">
          <SectionLead
            title={t("Action result")}
            description={`${actionResult.action} on ${actionResult.channelName}`}
          />
          <div className="mt-4">
            {actionResult.error ? (
              <p className="text-sm text-error" style={{ lineHeight: 1.85 }}>
                {actionResult.error}
              </p>
            ) : (
              <div>
                <p className="text-accent text-sm" style={{ lineHeight: 1.85 }}>
                  {actionResult.result}
                </p>
                {actionResult.detail ? (
                  <p
                    className="mt-1 text-xs"
                    style={{ color: "var(--fg-muted)", lineHeight: 1.65 }}
                  >
                    {actionResult.detail}
                  </p>
                ) : null}
              </div>
            )}
            <button
              type="button"
              onClick={() => setActionResult(null)}
              className="ui-button ui-button-muted mt-3 px-3 py-1.5 text-[10px] font-bold uppercase"
            >
              {t("Dismiss")}
            </button>
          </div>
        </Panel>
      ) : null}

      <div className="grid gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(320px,0.9fr)]">
        <Panel strong variant="stage" className="flex min-h-0 flex-col px-5 py-5">
          <SectionLead
            title={t("Route inventory")}
            description={t("Select a channel to inspect its posture and run diagnostics.")}
          />
          <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
            {channels.map((channel, index) => (
              <ChannelQueueRow
                key={channel.id}
                channel={channel}
                index={index}
                isActive={activeNames.includes(channel.name ?? "")}
                isSelected={channel.id === selectedChannel?.id}
                onSelect={() => setSelectedChannelId(channel.id)}
              />
            ))}
          </div>
        </Panel>

        <ChannelInspector
          channel={selectedChannel}
          isActive={activeNames.includes(selectedChannel?.name ?? "")}
          onToggle={(enabled) => {
            if (selectedChannel) {
              toggleMutation.mutate({ id: selectedChannel.id, enabled });
            }
          }}
          isToggling={
            toggleMutation.isPending && toggleMutation.variables?.id === selectedChannel?.id
          }
          onAction={(action) => {
            if (selectedChannel) {
              actionMutation.mutate({
                id: selectedChannel.id,
                action,
                channelName: selectedChannel.name,
              });
            }
          }}
          isActioning={
            actionMutation.isPending && actionMutation.variables?.id === selectedChannel?.id
          }
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Queue row
// ---------------------------------------------------------------------------

function ChannelQueueRow({
  channel,
  index,
  isActive,
  isSelected,
  onSelect,
}: {
  channel: Channel;
  index: number;
  isActive: boolean;
  isSelected: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();
  const isEnabled = channel.enabled !== false;
  const variant: "ok" | "degraded" | "neutral" = !isEnabled
    ? "neutral"
    : channel.configured
      ? isActive
        ? "ok"
        : "neutral"
      : "degraded";
  const statusLabel = !isEnabled
    ? "disabled"
    : channel.configured
      ? isActive
        ? "active"
        : "configured"
      : "not configured";

  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={isSelected}
      aria-label={t("Select channel {name}", { name: channel.name ?? String(index + 1) })}
      className="ui-ledger-button ui-ledger-card px-4 py-4"
    >
      <div className="ui-ledger-columns">
        <div className="ui-ledger-cell">
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <p className="text-fg text-sm" style={{ fontWeight: 600 }}>
              {channel.name ?? t("—")}
            </p>
          </div>
        </div>
        <ChannelQueueMeta label={t("Type")} value={channel.type ?? t("—")} />
        <div className="ui-ledger-cell flex items-start justify-between gap-3 xl:justify-end">
          <div className="flex flex-col items-start gap-3 xl:items-end">
            <StatusBadge variant={variant} label={t(statusLabel)} />
            <span className="ui-chip">{isSelected ? t("Pinned") : t("Inspect")}</span>
          </div>
        </div>
      </div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Inspector
// ---------------------------------------------------------------------------

function ChannelInspector({
  channel,
  isActive,
  onToggle,
  isToggling,
  onAction,
  isActioning,
}: {
  channel: Channel | null;
  isActive: boolean;
  onToggle: (enabled: boolean) => void;
  isToggling: boolean;
  onAction: (action: "doctor" | "test") => void;
  isActioning: boolean;
}) {
  const { t } = useI18n();

  if (!channel) {
    return (
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Inspector")}
          description={t("Select a channel from the inventory to inspect it.")}
        />
        <EmptyState
          title={t("No channel selected")}
          description={t("Pick a channel from the left inventory to view its posture.")}
        />
      </Panel>
    );
  }

  const isEnabled = channel.enabled !== false;
  const variant: "ok" | "degraded" | "neutral" = !isEnabled
    ? "neutral"
    : channel.configured
      ? isActive
        ? "ok"
        : "neutral"
      : "degraded";
  const statusLabel = !isEnabled
    ? "disabled"
    : channel.configured
      ? isActive
        ? "active"
        : "configured"
      : "not configured";

  return (
    <Panel strong variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Inspector")}
        description={t("Pinned channel detail with diagnostics and controls.")}
        action={<StatusBadge variant={variant} label={t(statusLabel)} />}
      />

      <div className="ui-rule-list mt-4">
        <ChannelMetaRow label={t("Name")} value={channel.name ?? t("—")} />
        <ChannelMetaRow label={t("Type")} value={channel.type ?? t("—")} />
        <ChannelMetaRow
          label={t("Configured")}
          value={channel.configured ? t("Enabled") : t("Disabled")}
        />
        <ChannelMetaRow label={t("Status")} value={t(statusLabel)} />
      </div>

      <div className="mt-5 border-t border-[var(--border)] pt-4">
        <p className="app-section-title">{t("Channel controls")}</p>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <button
            type="button"
            onClick={() => onToggle(!isEnabled)}
            disabled={isToggling || !channel.configured}
            className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
            style={{ color: isEnabled ? "var(--warn)" : "var(--accent)" }}
          >
            {isToggling ? t("Working...") : isEnabled ? t("Disable") : t("Enable")}
          </button>
          <button
            type="button"
            onClick={() => onAction("doctor")}
            disabled={isActioning || !channel.configured}
            className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
          >
            {isActioning ? t("Working...") : t("Doctor")}
          </button>
          <button
            type="button"
            onClick={() => onAction("test")}
            disabled={isActioning || !channel.configured}
            className="ui-button ui-button-accent-hint px-3 py-2 text-[10px] font-bold uppercase"
          >
            {t("Test")}
          </button>
        </div>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Shared sub-components
// ---------------------------------------------------------------------------

function ChannelQueueMeta({ label, value }: { label: string; value: string }) {
  return (
    <div className="ui-ledger-cell">
      <p className="app-section-title">{label}</p>
      <p className="mt-2 text-sm" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function ChannelMetaRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{label}</p>
      <p className="ui-rule-value" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}
