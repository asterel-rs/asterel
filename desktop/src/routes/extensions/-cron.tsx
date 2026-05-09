import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { FormInput } from "@/components/form-input";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { createCronJob, fetchCronJobs, patchCronJob, removeCronJob, runCronJob } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { CronJob } from "@/lib/types";
import { formatRelativeTime, statusVariant } from "./-helpers";

// ---------------------------------------------------------------------------
// Cron tab — queue + inspector workbench
// ---------------------------------------------------------------------------

export function CronTab() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [selectedJobId, setSelectedJobId] = useState<string | null>(null);
  const [showCreate, setShowCreate] = useState(false);
  const [expression, setExpression] = useState("");
  const [command, setCommand] = useState("");

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["cron-jobs"],
    queryFn: fetchCronJobs,
    refetchInterval: 15_000,
  });

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["cron-jobs"] });
  }, [queryClient]);

  const createMutation = useMutation({
    mutationFn: () => createCronJob(expression, command),
    onSuccess: () => {
      setExpression("");
      setCommand("");
      setShowCreate(false);
      invalidate();
    },
  });

  const jobItems = data?.items;
  const jobs = jobItems ?? [];
  const selectedJob = jobs.find((job) => job.id === selectedJobId) ?? null;
  const failingCount = jobs.filter(
    (job) =>
      job.consecutive_failures > 0 ||
      statusVariant(job.last_status) === "error" ||
      Boolean(job.breaker_open_until),
  ).length;
  const agentCount = jobs.filter((job) => job.origin === "agent").length;
  const healthyCount = jobs.filter((job) => statusVariant(job.last_status) === "ok").length;

  useEffect(() => {
    const nextJobs = jobItems ?? [];

    if (nextJobs.length === 0) {
      if (selectedJobId !== null) {
        setSelectedJobId(null);
      }
      return;
    }

    if (!selectedJobId || !nextJobs.some((job) => job.id === selectedJobId)) {
      setSelectedJobId(nextJobs[0]?.id ?? null);
    }
  }, [jobItems, selectedJobId]);

  if (isLoading) return <SkeletonLoader />;

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load cron jobs")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap gap-3">
          <StatPill label={t("Schedules")} value={String(jobs.length)} />
          <StatPill label={t("Agent owned")} value={String(agentCount)} tone="var(--info)" />
          <StatPill label={t("Needs review")} value={String(failingCount)} tone="var(--warn)" />
          <StatPill label={t("Healthy")} value={String(healthyCount)} tone="var(--accent-strong)" />
        </div>
        <button
          type="button"
          onClick={() => setShowCreate((v) => !v)}
          className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-fg"
        >
          {showCreate ? t("Close composer") : t("New schedule")}
        </button>
      </div>

      {showCreate ? (
        <Panel strong variant="stage" className="px-5 py-5">
          <SectionLead
            title={t("Compose a schedule")}
            description={t("Set a cron expression and the command to run.")}
          />
          <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,220px)_minmax(0,1fr)_auto]">
            <FormInput
              label={t("Cron expression")}
              value={expression}
              onChange={setExpression}
              placeholder={t("*/15 * * * *")}
            />
            <FormInput
              label={t("Command")}
              value={command}
              onChange={setCommand}
              placeholder={t("Summarize open tasks and send a digest")}
            />
            <div className="flex items-end gap-2">
              <button
                type="button"
                onClick={() => createMutation.mutate()}
                disabled={!expression.trim() || !command.trim() || createMutation.isPending}
                className="ui-button ui-button-accent-soft px-4 py-2 text-xs font-bold uppercase"
              >
                {createMutation.isPending ? t("Creating...") : t("Create job")}
              </button>
            </div>
          </div>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <span className="ui-chip">{t("5 field cron format")}</span>
            <span className="ui-chip">{t("Polled every 15s")}</span>
            {createMutation.isError ? (
              <span className="text-xs text-error">
                {createMutation.error instanceof Error
                  ? createMutation.error.message
                  : t("Failed to create job")}
              </span>
            ) : null}
          </div>
        </Panel>
      ) : null}

      {jobs.length === 0 ? (
        <EmptyState
          title={t("No scheduled jobs")}
          description={t(
            "Create a cron job to run commands on a schedule. Use the composer above to set a cron expression and command payload.",
          )}
          action={{ label: t("New Job"), onClick: () => setShowCreate(true) }}
        />
      ) : (
        <div className="grid gap-4 xl:grid-cols-[minmax(0,1.05fr)_minmax(360px,0.95fr)]">
          <Panel
            strong
            variant="stage"
            className="flex min-h-0 flex-col px-5 py-5"
            style={{ minHeight: "520px" }}
          >
            <SectionLead
              title={t("Schedule queue")}
              description={t(
                "Select a job to view its schedule detail and controls in the inspector.",
              )}
            />
            <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
              {jobs.map((job, index) => (
                <CronQueueRow
                  key={job.id}
                  job={job}
                  index={index}
                  isSelected={job.id === selectedJob?.id}
                  onSelect={() => setSelectedJobId(job.id)}
                />
              ))}
            </div>
          </Panel>

          <CronInspector key={selectedJob?.id ?? "none"} job={selectedJob} />
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Queue row — scannable summary per job
// ---------------------------------------------------------------------------

function CronQueueRow({
  job,
  index,
  isSelected,
  onSelect,
}: {
  job: CronJob;
  index: number;
  isSelected: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();
  const isEnabled = job.enabled !== false;
  const variant = statusVariant(job.last_status);
  const jobLabel = job.expression || job.job_kind;

  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={isSelected}
      aria-label={t("Select cron job {name}", { name: jobLabel })}
      className="ui-ledger-button ui-ledger-card px-4 py-4"
    >
      <div className="ui-ledger-columns">
        <div className="ui-ledger-cell">
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <span className="ui-chip">{job.job_kind}</span>
            {!isEnabled ? (
              <span className="ui-chip text-warn" style={{ borderColor: "var(--warn)" }}>
                {t("disabled")}
              </span>
            ) : null}
            {job.breaker_open_until ? <span className="ui-chip">{t("breaker open")}</span> : null}
          </div>
          <p
            className="mt-3 font-mono"
            style={{
              fontSize: "15px",
              fontWeight: 700,
              color: isEnabled ? "var(--fg)" : "var(--fg-muted)",
            }}
          >
            {job.expression}
          </p>
          <p className="mt-2 text-sm" style={{ color: "var(--fg-soft)", lineHeight: 1.65 }}>
            {truncateText(job.command, 120)}
          </p>
        </div>
        <CronQueueMeta
          label={t("Next run")}
          value={job.next_run ? formatRelativeTime(job.next_run) : t("Not scheduled")}
        />
        <CronQueueMeta label={t("Origin")} value={job.origin} />
        <div className="ui-ledger-cell flex items-start justify-between gap-3 xl:justify-end">
          <div className="flex flex-col items-start gap-3 xl:items-end">
            <StatusBadge variant={variant} label={job.last_status ?? "unknown"} />
            <span className="ui-chip">{isSelected ? t("Pinned") : t("Inspect")}</span>
          </div>
        </div>
      </div>
    </button>
  );
}

// ---------------------------------------------------------------------------
// Inspector — selected job detail + actions
// ---------------------------------------------------------------------------

function CronInspector({ job }: { job: CronJob | null }) {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [editMode, setEditMode] = useState(false);
  const [editExpression, setEditExpression] = useState("");
  const [editCommand, setEditCommand] = useState("");

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["cron-jobs"] });
  }, [queryClient]);

  const removeMutation = useMutation({
    mutationFn: () => {
      if (!job) throw new Error("No job");
      return removeCronJob(job.id);
    },
    onSuccess: () => {
      setConfirmRemove(false);
      invalidate();
    },
  });

  const runMutation = useMutation({
    mutationFn: () => {
      if (!job) throw new Error("No job");
      return runCronJob(job.id);
    },
    onSuccess: () => invalidate(),
  });

  const patchMutation = useMutation({
    mutationFn: (body: { expression?: string; command?: string; enabled?: boolean }) => {
      if (!job) throw new Error("No job");
      return patchCronJob(job.id, body);
    },
    onSuccess: () => {
      setEditMode(false);
      invalidate();
    },
  });

  if (!job) {
    return (
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Inspector")}
          description={t("Select a schedule from the queue to inspect it.")}
        />
        <EmptyState
          title={t("No job selected")}
          description={t("Pick a schedule from the left queue to view its detail and controls.")}
        />
      </Panel>
    );
  }

  const isEnabled = job.enabled !== false;
  const variant = statusVariant(job.last_status);

  const startEditing = () => {
    setEditExpression(job.expression);
    setEditCommand(job.command);
    setEditMode(true);
  };

  return (
    <Panel
      strong
      variant="stage"
      className="flex min-h-0 flex-col px-5 py-5"
      style={{ minHeight: "520px" }}
    >
      <SectionLead
        title={t("Inspector")}
        description={t("Pinned schedule detail. Queue stays visible to the left.")}
        action={<StatusBadge variant={variant} label={job.last_status ?? "unknown"} />}
      />

      <div className="ui-rule-list mt-4">
        <CronMetaRow label={t("Expression")} value={job.expression} mono />
        <CronMetaRow label={t("Command")} value={job.command} />
        <CronMetaRow label={t("Origin")} value={job.origin} />
        <CronMetaRow label={t("Kind")} value={job.job_kind} />
        <CronMetaRow
          label={t("Next run")}
          value={job.next_run ? formatRelativeTime(job.next_run) : t("Not scheduled")}
        />
        <CronMetaRow
          label={t("Last run")}
          value={job.last_run ? formatRelativeTime(job.last_run) : t("No runs yet")}
        />
        <CronMetaRow label={t("Failures")} value={String(job.consecutive_failures)} />
        <CronMetaRow label={t("Attempts")} value={String(job.max_attempts)} />
        <CronMetaRow
          label={t("Expires")}
          value={job.expires_at ? formatRelativeTime(job.expires_at) : t("No expiry")}
        />
        <CronMetaRow
          label={t("Breaker until")}
          value={job.breaker_open_until ? formatRelativeTime(job.breaker_open_until) : t("Closed")}
        />
      </div>

      <div className="mt-5 border-t border-[var(--border)] pt-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="app-section-title">{t("Schedule controls")}</p>
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{isEnabled ? t("enabled") : t("disabled")}</span>
          </div>
        </div>

        {editMode ? (
          <div className="mt-4 space-y-3">
            <FormInput
              label={t("Cron expression")}
              value={editExpression}
              onChange={setEditExpression}
              placeholder={t("*/15 * * * *")}
            />
            <FormInput
              label={t("Command")}
              value={editCommand}
              onChange={setEditCommand}
              placeholder={t("Command payload")}
            />
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() =>
                  patchMutation.mutate({
                    expression: editExpression,
                    command: editCommand,
                  })
                }
                disabled={!editExpression.trim() || !editCommand.trim() || patchMutation.isPending}
                className="ui-button ui-button-accent-soft px-3 py-1.5 text-[10px] font-bold uppercase"
              >
                {patchMutation.isPending ? t("Saving...") : t("Save")}
              </button>
              <button
                type="button"
                onClick={() => setEditMode(false)}
                className="ui-button ui-button-muted px-3 py-1.5 text-[10px] font-bold uppercase"
              >
                {t("Cancel")}
              </button>
            </div>
          </div>
        ) : (
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => runMutation.mutate()}
              disabled={runMutation.isPending}
              className="ui-button ui-button-accent-hint px-3 py-2 text-[10px] font-bold uppercase"
            >
              {runMutation.isPending ? t("Running...") : t("Run now")}
            </button>
            <button
              type="button"
              onClick={startEditing}
              className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
            >
              {t("Edit")}
            </button>
            <button
              type="button"
              onClick={() => patchMutation.mutate({ enabled: !isEnabled })}
              disabled={patchMutation.isPending}
              className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
              style={{ color: isEnabled ? "var(--warn)" : "var(--accent)" }}
            >
              {patchMutation.isPending ? "..." : isEnabled ? t("Disable") : t("Enable")}
            </button>
            {confirmRemove ? (
              <>
                <button
                  type="button"
                  onClick={() => removeMutation.mutate()}
                  disabled={removeMutation.isPending}
                  className="ui-button ui-button-error-soft px-3 py-2 text-[10px] font-bold uppercase"
                >
                  {removeMutation.isPending ? t("Removing...") : t("Confirm")}
                </button>
                <button
                  type="button"
                  onClick={() => setConfirmRemove(false)}
                  className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
                >
                  {t("Cancel")}
                </button>
              </>
            ) : (
              <button
                type="button"
                onClick={() => setConfirmRemove(true)}
                className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase text-error"
              >
                {t("Remove")}
              </button>
            )}
          </div>
        )}
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Shared sub-components
// ---------------------------------------------------------------------------

function CronQueueMeta({ label, value }: { label: string; value: string }) {
  return (
    <div className="ui-ledger-cell">
      <p className="app-section-title">{label}</p>
      <p className="mt-2 text-sm" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function CronMetaRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{label}</p>
      <p
        className={mono ? "ui-rule-value break-all font-mono" : "ui-rule-value"}
        style={{ color: "var(--fg-soft)" }}
      >
        {value}
      </p>
    </div>
  );
}

function truncateText(value: string, maxLength: number): string {
  if (value.length <= maxLength) return value;
  return `${value.slice(0, maxLength - 1).trimEnd()}...`;
}
