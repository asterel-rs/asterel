// Route: what does the runtime remember, and what should I correct or forget?
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { createFileRoute } from "@tanstack/react-router";
import { useEffect, useMemo, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { PageHeader, PageShell, Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import {
  correctMemorySlot,
  fetchMemoryEntities,
  fetchMemorySlots,
  forgetMemorySlot,
} from "@/lib/api";
import { formatDate } from "@/lib/format";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";

export const Route = createFileRoute("/memory")({
  component: MemoryPage,
});

function MemoryPage() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [selectedEntityId, setSelectedEntityId] = useState<string | null>(null);
  const [selectedSlotKey, setSelectedSlotKey] = useState<string | null>(null);
  const [reason, setReason] = useState("");
  const [newValue, setNewValue] = useState("");
  const [feedback, setFeedback] = useState<string | null>(null);

  usePageTitle("Memory");

  const entitiesQuery = useQuery({
    queryKey: ["memory", "entities"],
    queryFn: fetchMemoryEntities,
    refetchInterval: 30_000,
  });

  const entityItems = entitiesQuery.data?.items;

  useEffect(() => {
    const nextEntities = entityItems ?? [];
    if (nextEntities.length === 0) {
      setSelectedEntityId(null);
      return;
    }
    if (
      !selectedEntityId ||
      !nextEntities.some((entity) => entity.entity_id === selectedEntityId)
    ) {
      setSelectedEntityId(nextEntities[0]?.entity_id ?? null);
    }
  }, [entityItems, selectedEntityId]);

  const slotsQuery = useQuery({
    queryKey: ["memory", "slots", selectedEntityId],
    queryFn: () => fetchMemorySlots(selectedEntityId ?? ""),
    enabled: selectedEntityId !== null,
    refetchInterval: 30_000,
  });

  const slotItems = slotsQuery.data?.items;

  useEffect(() => {
    const nextSlots = slotItems ?? [];
    if (nextSlots.length === 0) {
      setSelectedSlotKey(null);
      setNewValue("");
      return;
    }
    if (!selectedSlotKey || !nextSlots.some((slot) => slot.slot_key === selectedSlotKey)) {
      const nextSlot = nextSlots[0]?.slot_key ?? null;
      setSelectedSlotKey(nextSlot);
      setNewValue(nextSlots[0]?.value ?? "");
    }
  }, [slotItems, selectedSlotKey]);

  const selectedSlot = useMemo(
    () => (slotItems ?? []).find((slot) => slot.slot_key === selectedSlotKey) ?? null,
    [slotItems, selectedSlotKey],
  );

  const refreshMemory = () => {
    queryClient.invalidateQueries({ queryKey: ["memory"] });
  };

  const correctMutation = useMutation({
    mutationFn: () => {
      if (!selectedEntityId || !selectedSlot) {
        throw new Error("Select a slot first.");
      }
      return correctMemorySlot({
        entity_id: selectedEntityId,
        slot_key: selectedSlot.slot_key,
        old_value: selectedSlot.value,
        new_value: newValue,
        reason,
      });
    },
    onSuccess: () => {
      setFeedback(t("Memory corrected"));
      setReason("");
      refreshMemory();
    },
    onError: (error) => {
      setFeedback(error instanceof Error ? error.message : t("Correction failed"));
    },
  });

  const forgetMutation = useMutation({
    mutationFn: (mode: "soft" | "tombstone") => {
      if (!selectedEntityId || !selectedSlot) {
        throw new Error("Select a slot first.");
      }
      return forgetMemorySlot({
        entity_id: selectedEntityId,
        slot_key: selectedSlot.slot_key,
        reason: reason || t("Operator review"),
        mode,
      });
    },
    onSuccess: () => {
      setFeedback(t("Memory action recorded"));
      setReason("");
      refreshMemory();
    },
    onError: (error) => {
      setFeedback(error instanceof Error ? error.message : t("Forget failed"));
    },
  });

  if (entitiesQuery.isLoading) {
    return <SkeletonLoader />;
  }

  if (entitiesQuery.isError) {
    return (
      <ErrorState
        title={t("Failed to load memory entities")}
        message={
          entitiesQuery.error instanceof Error
            ? entitiesQuery.error.message
            : t("Could not reach the daemon.")
        }
        onRetry={() => entitiesQuery.refetch()}
      />
    );
  }

  return (
    <PageShell accent="var(--section-overview)">
      <PageHeader
        eyebrow={t("Memory ledger")}
        title={t("Memory")}
        description={t(
          "Inspect what the runtime remembers, then correct or forget it with evidence nearby.",
        )}
        actions={
          <>
            <StatPill label={t("Entities")} value={String((entityItems ?? []).length)} />
            <StatPill
              label={t("Backend")}
              value={entitiesQuery.data?.backend ?? t("unknown")}
              tone="var(--accent-strong)"
            />
            {slotsQuery.data ? (
              <StatPill
                label={t("Slots")}
                value={String(slotsQuery.data.count)}
                hint={t("{count} events", { count: slotsQuery.data.event_count })}
              />
            ) : null}
          </>
        }
      />

      {feedback ? (
        <Panel variant="stage" className="px-4 py-3">
          <p className="text-sm" style={{ color: "var(--accent-strong)" }}>
            {feedback}
          </p>
        </Panel>
      ) : null}

      {(entityItems ?? []).length === 0 ? (
        <EmptyState
          title={t("No memory entities yet")}
          description={t(
            "Once the companion has stored user or room memory, review rows will appear here.",
          )}
        />
      ) : (
        <div className="grid gap-4 xl:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]">
          <Panel strong variant="stage" className="px-5 py-5">
            <SectionLead
              title={t("Entity list")}
              description={t(
                "Pick a user, room, or runtime entity to inspect its remembered slots.",
              )}
            />
            <div className="mt-4 space-y-2">
              {(entityItems ?? []).map((entity) => {
                const selected = entity.entity_id === selectedEntityId;
                return (
                  <button
                    key={entity.entity_id}
                    type="button"
                    onClick={() => setSelectedEntityId(entity.entity_id)}
                    className="ui-ledger-link w-full px-4 py-3 text-left"
                    data-active={selected}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="min-w-0">
                        <p
                          className="truncate text-sm text-[var(--fg)]"
                          style={{ fontWeight: 600 }}
                        >
                          {humanizeEntityId(entity.entity_id)}
                        </p>
                        <p className="mt-1 break-all text-xs text-[var(--fg-muted)]">
                          {entity.entity_id}
                        </p>
                        <p className="mt-1 text-xs text-[var(--fg-muted)]">
                          {t("{count} slots", { count: entity.slot_count })}
                        </p>
                      </div>
                      <StatusBadge
                        variant={selected ? "ok" : "neutral"}
                        label={selected ? t("selected") : t("ready")}
                      />
                    </div>
                  </button>
                );
              })}
            </div>
          </Panel>

          <Panel variant="stage" className="px-5 py-5">
            <SectionLead
              title={
                selectedEntityId
                  ? t("Slots for {entity}", { entity: selectedEntityId })
                  : t("Slots")
              }
              description={t(
                "Select a row to review values, provenance hints, and correction actions.",
              )}
            />
            {slotsQuery.isLoading ? (
              <SkeletonLoader />
            ) : slotsQuery.isError ? (
              <ErrorState
                title={t("Failed to load slots")}
                message={
                  slotsQuery.error instanceof Error
                    ? slotsQuery.error.message
                    : t("Could not load entity slots.")
                }
                onRetry={() => slotsQuery.refetch()}
              />
            ) : (slotItems ?? []).length === 0 ? (
              <EmptyState
                title={t("No slots found")}
                description={t("This entity has no active remembered slots yet.")}
              />
            ) : (
              <div className="space-y-4">
                <div className="space-y-2">
                  {(slotItems ?? []).map((slot) => {
                    const selected = slot.slot_key === selectedSlotKey;
                    return (
                      <button
                        key={slot.slot_key}
                        type="button"
                        onClick={() => {
                          setSelectedSlotKey(slot.slot_key);
                          setNewValue(slot.value);
                        }}
                        className="ui-ledger-link w-full px-4 py-3 text-left"
                        data-active={selected}
                      >
                        <div className="flex items-start justify-between gap-3">
                          <div className="min-w-0 flex-1">
                            <p
                              className="truncate text-sm text-[var(--fg)]"
                              style={{ fontWeight: 600 }}
                            >
                              {slot.slot_key}
                            </p>
                            <p className="mt-1 line-clamp-2 text-xs text-[var(--fg-soft)]">
                              {slot.value}
                            </p>
                          </div>
                          <StatusBadge
                            variant={slot.privacy_level === "public" ? "ok" : "neutral"}
                            label={slot.privacy_level}
                          />
                        </div>
                      </button>
                    );
                  })}
                </div>

                {selectedSlot ? (
                  <Panel variant="panel" className="px-4 py-4">
                    <SectionLead title={t("Selected slot")} description={selectedSlot.slot_key} />
                    <div className="ui-rule-list mt-3">
                      <MemoryFactRow
                        label={t("Meaning")}
                        value={humanizeSlotKey(selectedSlot.slot_key)}
                      />
                      <MemoryFactRow label={t("Source")} value={selectedSlot.source} />
                      <MemoryFactRow
                        label={t("Confidence")}
                        value={selectedSlot.confidence.toFixed(2)}
                      />
                      <MemoryFactRow
                        label={t("Importance")}
                        value={selectedSlot.importance.toFixed(2)}
                      />
                      <MemoryFactRow
                        label={t("Updated")}
                        value={formatDate(selectedSlot.updated_at)}
                      />
                      {selectedSlot.provenance ? (
                        <>
                          <MemoryFactRow
                            label={t("Provenance source")}
                            value={selectedSlot.provenance.source_class}
                          />
                          <MemoryFactRow
                            label={t("Reference")}
                            value={selectedSlot.provenance.reference}
                          />
                          {selectedSlot.provenance.evidence_uri ? (
                            <MemoryFactRow
                              label={t("Evidence URI")}
                              value={selectedSlot.provenance.evidence_uri}
                            />
                          ) : null}
                        </>
                      ) : null}
                    </div>
                    <label className="mt-4 block text-xs font-semibold uppercase tracking-[0.14em] text-[var(--fg-muted)]">
                      {t("Corrected value")}
                    </label>
                    <textarea
                      value={newValue}
                      onChange={(event) => setNewValue(event.target.value)}
                      className="ui-field mt-2 min-h-[124px] w-full"
                    />
                    <label className="mt-4 block text-xs font-semibold uppercase tracking-[0.14em] text-[var(--fg-muted)]">
                      {t("Reason")}
                    </label>
                    <input
                      value={reason}
                      onChange={(event) => setReason(event.target.value)}
                      className="ui-field mt-2 w-full"
                      placeholder={t("Why are you changing this memory?")}
                    />
                    <div className="mt-4 flex flex-wrap gap-2">
                      <button
                        type="button"
                        onClick={() => correctMutation.mutate()}
                        disabled={!reason.trim() || !newValue.trim() || correctMutation.isPending}
                        className="ui-button ui-button-accent-fill px-4 py-2 text-xs font-bold uppercase"
                      >
                        {correctMutation.isPending ? t("Saving...") : t("Save correction")}
                      </button>
                      <button
                        type="button"
                        onClick={() => forgetMutation.mutate("soft")}
                        disabled={forgetMutation.isPending}
                        className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase"
                      >
                        {forgetMutation.isPending ? t("Applying...") : t("Soft forget")}
                      </button>
                      <button
                        type="button"
                        onClick={() => forgetMutation.mutate("tombstone")}
                        disabled={forgetMutation.isPending}
                        className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase"
                        style={{ color: "var(--warn)" }}
                      >
                        {t("Tombstone")}
                      </button>
                    </div>
                  </Panel>
                ) : null}
              </div>
            )}
          </Panel>
        </div>
      )}
    </PageShell>
  );
}

function MemoryFactRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{label}</p>
      <p className="ui-rule-value">{value}</p>
    </div>
  );
}

function humanizeEntityId(entityId: string): string {
  const [scope, ...rest] = entityId.split(":");
  if (rest.length === 0) return entityId;
  return `${scope} · ${rest.join(":").replace(/[._-]/g, " ")}`;
}

function humanizeSlotKey(slotKey: string): string {
  return slotKey.replace(/[./_-]/g, " ");
}
