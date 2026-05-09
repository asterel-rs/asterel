import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import {
  cancelCompanionWindow,
  confirmCompanionWindow,
  fetchCompanionCaptions,
  fetchCompanionScopes,
  fetchCompanionWidgets,
  fetchCompanionWindows,
  patchCompanionConfig,
  postCompanionIngress,
} from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { formatDateTime as formatLocalizedDateTime } from "@/lib/i18n-core";
import type {
  CompanionCaption,
  CompanionConfigPatch,
  CompanionIngressPayload,
  CompanionScope,
  CompanionWidget,
  CompanionWindowEntry,
} from "@/lib/types";
function formatDateTime(value: string): string {
  return formatLocalizedDateTime(value, {});
}

// ---------------------------------------------------------------------------
// Companion helpers
// ---------------------------------------------------------------------------

interface CompanionWidgetViewModel extends CompanionWidget {
  expiresAtLabel: string | null;
  payloadPreview: string;
}

function sortScopes(scopes: CompanionScope[]) {
  return [...scopes].sort((left, right) => {
    const leftTotal = left.captions + left.widgets + left.windows;
    const rightTotal = right.captions + right.widgets + right.windows;
    return rightTotal - leftTotal;
  });
}

function windowStateVariant(state: CompanionWindowEntry["state"]) {
  switch (state) {
    case "confirmed":
      return "ok";
    case "pending":
      return "degraded";
    case "cancelled":
    case "expired":
      return "neutral";
    default:
      return "neutral";
  }
}

// ---------------------------------------------------------------------------
// Companion tab
// ---------------------------------------------------------------------------

export function CompanionTab() {
  const { t } = useI18n();
  const [selectedScope, setSelectedScope] = useState<string | null>(null);
  const [surfaceTab, setSurfaceTab] = useState<"captions" | "widgets" | "windows">("captions");

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["companions"],
    queryFn: fetchCompanionScopes,
  });

  const scopeItems = data?.items;
  const scopes = scopeItems ?? [];
  const selectedScopeData = scopes.find((scope) => scope.scope === selectedScope) ?? null;

  useEffect(() => {
    const nextScopes = scopeItems ?? [];

    if (nextScopes.length === 0) {
      if (selectedScope !== null) {
        setSelectedScope(null);
        setSurfaceTab("captions");
      }
      return;
    }

    const firstScope = nextScopes[0];
    if (!selectedScope || !nextScopes.some((scope) => scope.scope === selectedScope)) {
      if (!firstScope) {
        return;
      }
      setSelectedScope(firstScope.scope);
      setSurfaceTab("captions");
    }
  }, [scopeItems, selectedScope]);

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load companion scopes")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  if (scopes.length === 0) {
    return (
      <EmptyState
        title={t("No active companion scopes")}
        description={t("Companion surfaces activate when events are sent.")}
      />
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-end">
        <button
          type="button"
          onClick={() => refetch()}
          className="ui-button ui-button-muted text-fg px-4 py-2 text-xs font-bold uppercase"
        >
          {t("Refresh scopes")}
        </button>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.82fr)_minmax(0,1.18fr)]">
        <Panel strong variant="stage" className="flex min-h-0 flex-col px-5 py-5">
          <SectionLead
            title={t("Scope rail")}
            description={t(
              "Select a scope to pin its captions, widgets, and confirmation windows in the center surface.",
            )}
          />
          <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
            {sortScopes(scopes).map((scope, index) => (
              <ScopeRailRow
                key={scope.scope}
                scope={scope}
                index={index}
                isSelected={scope.scope === selectedScopeData?.scope}
                onSelect={() => {
                  setSelectedScope(scope.scope);
                  setSurfaceTab("captions");
                }}
              />
            ))}
          </div>
        </Panel>

        <SelectedScopeSurface
          scope={selectedScopeData}
          tab={surfaceTab}
          onTabChange={setSurfaceTab}
        />
      </div>

      <div className="grid gap-4 xl:grid-cols-2">
        <ContextIngressPanel scopes={scopes} selectedScope={selectedScope} />
        <CompanionConfigPanel />
      </div>
    </div>
  );
}

function ScopeRailRow({
  scope,
  index,
  isSelected,
  onSelect,
}: {
  scope: CompanionScope;
  index: number;
  isSelected: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();

  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={isSelected}
      aria-label={t("Select scope {name}", { name: scope.scope })}
      className="ui-ledger-button ui-ledger-card px-4 py-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <p className="app-section-title">{t("Scope")}</p>
          </div>
          <p className="text-fg mt-3 break-all font-mono text-sm" style={{ lineHeight: 1.65 }}>
            {scope.scope}
          </p>
        </div>
        <span className="ui-chip">{t("{count} gates", { count: scope.context_gate_entries })}</span>
      </div>

      <div className="ui-rule-list mt-4">
        <ScopeMetric label={t("Captions")} value={String(scope.captions)} tone="var(--info)" />
        <ScopeMetric label={t("Widgets")} value={String(scope.widgets)} tone="var(--accent)" />
        <ScopeMetric label={t("Windows")} value={String(scope.windows)} tone="var(--warn)" />
      </div>
    </button>
  );
}

function SelectedScopeSurface({
  scope,
  tab,
  onTabChange,
}: {
  scope: CompanionScope | null;
  tab: "captions" | "widgets" | "windows";
  onTabChange: (next: "captions" | "widgets" | "windows") => void;
}) {
  const { t } = useI18n();

  if (!scope) {
    return (
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Selected scope")}
          description={t("Choose a scope from the rail to inspect its live companion surfaces.")}
        />
        <EmptyState
          title={t("No scope selected")}
          description={t("Pick a scope from the left rail to inspect it.")}
        />
      </Panel>
    );
  }

  return (
    <Panel strong variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Selected scope")}
        description={t(
          "Keep one ambient scope pinned while you inspect captions, widgets, and request windows.",
        )}
        action={
          <span className="ui-chip">
            {t("{count} context gates", { count: scope.context_gate_entries })}
          </span>
        }
      />

      <div className="ui-rule-list mt-4">
        <ScopeMetric label={t("Scope ID")} value={scope.scope} mono />
        <ScopeMetric label={t("Captions")} value={String(scope.captions)} />
        <ScopeMetric label={t("Widgets")} value={String(scope.widgets)} />
        <ScopeMetric label={t("Windows")} value={String(scope.windows)} />
      </div>

      <div className="mt-5 border-t border-[var(--border)] pt-4">
        <div className="flex flex-wrap gap-2">
          {(["captions", "widgets", "windows"] as const).map((entry) => (
            <button
              key={entry}
              type="button"
              onClick={() => onTabChange(entry)}
              data-active={tab === entry}
              className="ui-segment-button"
            >
              {t(entry)}
            </button>
          ))}
        </div>

        <div className="mt-4">
          {tab === "captions" ? <CaptionsSurface scope={scope.scope} /> : null}
          {tab === "widgets" ? <WidgetsSurface scope={scope.scope} /> : null}
          {tab === "windows" ? <WindowsSurface scope={scope.scope} /> : null}
        </div>
      </div>
    </Panel>
  );
}

function CaptionsSurface({ scope }: { scope: string }) {
  const { t } = useI18n();
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["companions", scope, "captions"],
    queryFn: () => fetchCompanionCaptions(scope),
  });

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <div className="ui-ledger-card px-4 py-4">
        <p className="text-error text-sm">
          {error instanceof Error ? error.message : t("Could not load captions.")}
        </p>
      </div>
    );
  }

  const items = data?.items ?? [];
  if (items.length === 0) {
    return <MiniEmpty label={t("No captions yet")} />;
  }

  return (
    <div className="space-y-3">
      {items.map((caption) => (
        <CaptionCard key={caption.caption_id} caption={caption} />
      ))}
    </div>
  );
}

function WidgetsSurface({ scope }: { scope: string }) {
  const { t } = useI18n();
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["companions", scope, "widgets"],
    queryFn: () => fetchCompanionWidgets(scope),
    select: (response): CompanionWidgetViewModel[] =>
      response.items.map((widget) => ({
        ...widget,
        expiresAtLabel: widget.expires_at ? formatLocalizedDateTime(widget.expires_at, {}) : null,
        payloadPreview: JSON.stringify(widget.payload, null, 2),
      })),
  });

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <div className="ui-ledger-card px-4 py-4">
        <p className="text-error text-sm">
          {error instanceof Error ? error.message : t("Could not load widgets.")}
        </p>
      </div>
    );
  }

  const items = data ?? [];
  if (items.length === 0) {
    return <MiniEmpty label={t("No active widgets")} />;
  }

  return (
    <div className="space-y-3">
      {items.map((widget) => (
        <WidgetCard key={widget.widget_id} widget={widget} />
      ))}
    </div>
  );
}

function WindowsSurface({ scope }: { scope: string }) {
  const { t } = useI18n();
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ["companions", scope, "windows"],
    queryFn: () => fetchCompanionWindows(scope),
  });

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <div className="ui-ledger-card px-4 py-4">
        <p className="text-error text-sm">
          {error instanceof Error ? error.message : t("Could not load request windows.")}
        </p>
      </div>
    );
  }

  const items = data?.items ?? [];
  if (items.length === 0) {
    return <MiniEmpty label={t("No request windows")} />;
  }

  return (
    <div className="space-y-3">
      {items.map((windowEntry) => (
        <WindowCard key={windowEntry.window_id} windowEntry={windowEntry} scope={scope} />
      ))}
    </div>
  );
}

function CaptionCard({ caption }: { caption: CompanionCaption }) {
  const { t } = useI18n();

  return (
    <div className="ui-ledger-card px-4 py-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <StatusBadge
              variant={caption.channel === "speaker" ? "info" : "neutral"}
              label={caption.channel}
            />
            <span className="ui-chip">{t("seq {value}", { value: caption.sequence })}</span>
            <span className="ui-chip">{formatDateTime(caption.emitted_at)}</span>
          </div>
          <p className="mt-3 text-sm" style={{ color: "var(--fg-soft)", lineHeight: 1.85 }}>
            {caption.text}
          </p>
        </div>
      </div>
    </div>
  );
}

function WidgetCard({ widget }: { widget: CompanionWidgetViewModel }) {
  const { t } = useI18n();

  return (
    <div className="ui-ledger-card px-4 py-4">
      <div className="flex items-center justify-between gap-3">
        <div className="min-w-0">
          <p className="app-section-title">{t("Widget ID")}</p>
          <p className="text-fg mt-2 break-all text-sm">{widget.widget_id}</p>
        </div>
        {widget.expiresAtLabel ? (
          <span className="ui-chip">{t("exp {value}", { value: widget.expiresAtLabel })}</span>
        ) : null}
      </div>
      <pre className="ui-code-block mt-4 max-h-44 overflow-auto font-mono">
        {widget.payloadPreview}
      </pre>
    </div>
  );
}

function WindowCard({ windowEntry, scope }: { windowEntry: CompanionWindowEntry; scope: string }) {
  const { t } = useI18n();
  const queryClient = useQueryClient();

  const confirmMutation = useMutation({
    mutationFn: () => confirmCompanionWindow(scope, windowEntry.window_id),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["companions", scope, "windows"],
      });
    },
  });

  const cancelMutation = useMutation({
    mutationFn: () => cancelCompanionWindow(scope, windowEntry.window_id),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["companions", scope, "windows"],
      });
    },
  });

  return (
    <div className="ui-ledger-card px-4 py-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="app-section-title">{t("Requested action")}</p>
          <p className="text-fg mt-2 text-sm" style={{ lineHeight: 1.65 }}>
            {windowEntry.requested_action}
          </p>
          <div className="mt-3 space-y-2">
            <p className="break-all font-mono text-xs" style={{ color: "var(--fg-muted)" }}>
              {windowEntry.window_id}
            </p>
            <p className="text-xs" style={{ color: "var(--fg-muted)" }}>
              {t("Created {value}", {
                value: formatDateTime(windowEntry.created_at),
              })}
            </p>
            <p className="text-xs" style={{ color: "var(--fg-muted)" }}>
              {t("Expires {value}", {
                value: formatDateTime(windowEntry.expires_at),
              })}
            </p>
          </div>
          {windowEntry.state === "pending" ? (
            <div className="mt-3 flex gap-2">
              <button
                type="button"
                onClick={() => confirmMutation.mutate()}
                disabled={confirmMutation.isPending}
                className="ui-button ui-button-accent-fill px-3 py-1 text-xs font-bold uppercase"
              >
                {confirmMutation.isPending ? t("Confirming...") : t("Confirm")}
              </button>
              <button
                type="button"
                onClick={() => cancelMutation.mutate()}
                disabled={cancelMutation.isPending}
                className="ui-button ui-button-muted text-error px-3 py-1 text-xs font-bold uppercase"
              >
                {cancelMutation.isPending ? t("Cancelling...") : t("Cancel")}
              </button>
            </div>
          ) : null}
        </div>
        <StatusBadge variant={windowStateVariant(windowEntry.state)} label={windowEntry.state} />
      </div>
    </div>
  );
}

function ScopeMetric({
  label,
  value,
  mono,
  tone,
}: {
  label: string;
  value: string;
  mono?: boolean;
  tone?: string;
}) {
  const { t } = useI18n();

  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{t(label)}</p>
      <p
        className={mono ? "ui-rule-value break-all font-mono" : "ui-rule-value"}
        style={{ color: tone ?? "var(--fg-soft)" }}
      >
        {value}
      </p>
    </div>
  );
}

function ContextIngressPanel({
  scopes,
  selectedScope,
}: {
  scopes: CompanionScope[];
  selectedScope: string | null;
}) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [targetScope, setTargetScope] = useState("");
  const [ingressKind, setIngressKind] = useState<CompanionIngressPayload["kind"]>("text");
  const [ingressContent, setIngressContent] = useState("");

  useEffect(() => {
    if (selectedScope && !targetScope) {
      setTargetScope(selectedScope);
    }
  }, [selectedScope, targetScope]);

  const ingress = useMutation({
    mutationFn: (payload: { scope: string; body: CompanionIngressPayload }) =>
      postCompanionIngress(payload.scope, payload.body),
    onSuccess: () => {
      setIngressContent("");
      queryClient.invalidateQueries({ queryKey: ["companions"] });
    },
  });

  const resolvedScope = targetScope || selectedScope || "";
  const canSubmit = resolvedScope.length > 0 && ingressContent.trim().length > 0;

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Context ingress")}
        description={t(
          "Send context to a companion scope. Accepts text, clipboard, file references, or screenshot data.",
        )}
      />

      <div className="mt-4 space-y-3">
        <div>
          <label className="ui-field-label" htmlFor="ingress-scope">
            {t("Target scope")}
          </label>
          {scopes.length > 0 ? (
            <select
              id="ingress-scope"
              value={resolvedScope}
              onChange={(e) => setTargetScope(e.target.value)}
              className="ui-field"
            >
              <option value="">{t("Select scope")}</option>
              {scopes.map((s) => (
                <option key={s.scope} value={s.scope}>
                  {s.scope}
                </option>
              ))}
            </select>
          ) : (
            <input
              id="ingress-scope"
              type="text"
              value={targetScope}
              onChange={(e) => setTargetScope(e.target.value)}
              placeholder={t("Enter scope identifier")}
              className="ui-field"
            />
          )}
        </div>

        <div>
          <label className="ui-field-label" htmlFor="ingress-kind">
            {t("Kind")}
          </label>
          <div className="flex flex-wrap gap-2">
            {(["text", "clipboard", "file", "screenshot"] as const).map((kind) => (
              <button
                key={kind}
                type="button"
                onClick={() => setIngressKind(kind)}
                data-active={ingressKind === kind}
                className="ui-segment-button"
              >
                {t(kind)}
              </button>
            ))}
          </div>
        </div>

        <div>
          <label className="ui-field-label" htmlFor="ingress-content">
            {t("Content")}
          </label>
          <textarea
            id="ingress-content"
            value={ingressContent}
            onChange={(e) => setIngressContent(e.target.value)}
            placeholder={
              ingressKind === "text"
                ? t("Paste or type context to send")
                : ingressKind === "clipboard"
                  ? t("Paste clipboard content")
                  : ingressKind === "file"
                    ? t("File path or reference")
                    : t("Base64 screenshot data or path")
            }
            rows={3}
            className="ui-field"
            style={{ resize: "vertical", minHeight: "60px" }}
          />
        </div>

        {ingress.isError ? (
          <p className="text-error text-xs">
            {ingress.error instanceof Error ? ingress.error.message : t("Ingress failed.")}
          </p>
        ) : null}

        {ingress.isSuccess ? <p className="text-accent text-xs">{t("Context sent.")}</p> : null}

        <button
          type="button"
          disabled={!canSubmit || ingress.isPending}
          onClick={() =>
            ingress.mutate({
              scope: resolvedScope,
              body: { kind: ingressKind, content: ingressContent.trim() },
            })
          }
          className="ui-button ui-button-accent-fill px-4 py-2 text-xs font-bold uppercase"
        >
          {ingress.isPending ? t("Sending...") : t("Send context")}
        </button>
      </div>
    </Panel>
  );
}

function CompanionConfigPanel() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [configDraft, setConfigDraft] = useState<CompanionConfigPatch>({});
  const [showConfig, setShowConfig] = useState(false);

  const configMutation = useMutation({
    mutationFn: (patch: CompanionConfigPatch) => patchCompanionConfig(patch),
    onSuccess: () => {
      setConfigDraft({});
      queryClient.invalidateQueries({ queryKey: ["companions"] });
    },
  });

  const hasChanges =
    configDraft.enabled !== undefined ||
    configDraft.caption_retention_seconds !== undefined ||
    configDraft.widget_ttl_seconds !== undefined;

  return (
    <Panel variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Companion config")}
        description={t("Adjust companion behavior. Changes apply on next cycle.")}
        action={
          <button
            type="button"
            onClick={() => setShowConfig(!showConfig)}
            className="ui-button ui-button-muted text-fg px-3 py-1 text-xs font-bold uppercase"
          >
            {showConfig ? t("Collapse") : t("Expand")}
          </button>
        }
      />

      {showConfig ? (
        <div className="mt-4 space-y-3">
          <div>
            <label className="ui-field-label" htmlFor="config-enabled">
              {t("Enabled")}
            </label>
            <div className="flex gap-2">
              {([true, false] as const).map((val) => (
                <button
                  key={String(val)}
                  type="button"
                  data-active={configDraft.enabled === val}
                  onClick={() =>
                    setConfigDraft((prev) => {
                      if (prev.enabled === val) {
                        const { enabled: _enabled, ...rest } = prev;
                        return rest;
                      }
                      return { ...prev, enabled: val };
                    })
                  }
                  className="ui-segment-button"
                >
                  {val ? t("on") : t("off")}
                </button>
              ))}
            </div>
          </div>

          <div>
            <label className="ui-field-label" htmlFor="config-caption-ttl">
              {t("Caption retention (seconds)")}
            </label>
            <input
              id="config-caption-ttl"
              type="number"
              min={0}
              value={configDraft.caption_retention_seconds ?? ""}
              onChange={(e) =>
                setConfigDraft((prev) => {
                  if (!e.target.value) {
                    const { caption_retention_seconds: _captionRetentionSeconds, ...rest } = prev;
                    return rest;
                  }
                  return { ...prev, caption_retention_seconds: Number(e.target.value) };
                })
              }
              placeholder={t("Default")}
              className="ui-field"
            />
          </div>

          <div>
            <label className="ui-field-label" htmlFor="config-widget-ttl">
              {t("Widget TTL (seconds)")}
            </label>
            <input
              id="config-widget-ttl"
              type="number"
              min={0}
              value={configDraft.widget_ttl_seconds ?? ""}
              onChange={(e) =>
                setConfigDraft((prev) => {
                  if (!e.target.value) {
                    const { widget_ttl_seconds: _widgetTtlSeconds, ...rest } = prev;
                    return rest;
                  }
                  return { ...prev, widget_ttl_seconds: Number(e.target.value) };
                })
              }
              placeholder={t("Default")}
              className="ui-field"
            />
          </div>

          {configMutation.isError ? (
            <p className="text-error text-xs">
              {configMutation.error instanceof Error
                ? configMutation.error.message
                : t("Config update failed.")}
            </p>
          ) : null}

          {configMutation.isSuccess ? (
            <p className="text-accent text-xs">{t("Config updated.")}</p>
          ) : null}

          <button
            type="button"
            disabled={!hasChanges || configMutation.isPending}
            onClick={() => configMutation.mutate(configDraft)}
            className="ui-button ui-button-accent-soft px-4 py-2 text-xs font-bold uppercase"
          >
            {configMutation.isPending ? t("Saving...") : t("Apply changes")}
          </button>
        </div>
      ) : null}
    </Panel>
  );
}

function MiniEmpty({ label }: { label: string }) {
  const { t } = useI18n();

  return (
    <div className="ui-ledger-card px-4 py-4">
      <p className="text-sm" style={{ color: "var(--fg-muted)" }}>
        {t(label)}
      </p>
    </div>
  );
}
