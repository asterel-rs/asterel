import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { FormInput } from "@/components/form-input";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { fetchSkills, installSkill, patchSkill, removeSkill } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { Skill } from "@/lib/types";

// ---------------------------------------------------------------------------
// Skills tab
// ---------------------------------------------------------------------------

export function SkillsTab() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [installSource, setInstallSource] = useState("");
  const [showInstall, setShowInstall] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState<string | null>(null);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["skills"],
    queryFn: fetchSkills,
  });

  const invalidate = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["skills"] });
  }, [queryClient]);

  const installMutation = useMutation({
    mutationFn: (source: string) => installSkill(source),
    onSuccess: () => {
      setInstallSource("");
      setShowInstall(false);
      invalidate();
    },
  });

  const removeMutation = useMutation({
    mutationFn: (name: string) => removeSkill(name),
    onSuccess: () => {
      setConfirmRemove(null);
      invalidate();
    },
  });

  const toggleMutation = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) => patchSkill(id, { enabled }),
    onSuccess: () => invalidate(),
  });

  const skills = data?.items ?? [];
  const toolCount = skills.reduce((total, skill) => total + (skill.tools?.length ?? 0), 0);

  if (isLoading) return <SkeletonLoader />;

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load skills")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex flex-wrap gap-3">
          <StatPill label={t("Packs")} value={String(skills.length)} />
          <StatPill label={t("Tools")} value={String(toolCount)} tone="var(--accent-strong)" />
        </div>
        <button
          type="button"
          onClick={() => setShowInstall((v) => !v)}
          className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-fg"
        >
          {showInstall ? t("Close install panel") : t("Install skill")}
        </button>
      </div>

      {showInstall ? (
        <Panel variant="stage" className="px-5 py-5">
          <SectionLead
            title={t("Install source")}
            description={t("Point at a local directory containing a skill pack.")}
          />
          <div className="mt-4 space-y-3">
            <FormInput
              label={t("Source")}
              value={installSource}
              onChange={setInstallSource}
              placeholder={t("/path/to/skill")}
            />
            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={() => installMutation.mutate(installSource)}
                disabled={!installSource.trim() || installMutation.isPending}
                className="ui-button ui-button-accent-soft px-4 py-2 text-xs font-bold uppercase"
              >
                {installMutation.isPending ? t("Installing...") : t("Install")}
              </button>
              {installMutation.isError ? (
                <span className="text-xs text-error">
                  {installMutation.error instanceof Error
                    ? installMutation.error.message
                    : t("Install failed")}
                </span>
              ) : null}
            </div>
          </div>
        </Panel>
      ) : null}

      {skills.length === 0 ? (
        <EmptyState
          title={t("No skills installed")}
          description={t(
            "Install skill packs to extend agent capabilities. Use the install panel above to point at a local skill directory.",
          )}
          action={{
            label: t("Install skill"),
            onClick: () => setShowInstall(true),
          }}
        />
      ) : (
        <Panel strong variant="stage" className="px-5 py-5">
          <SectionLead
            title={t("Skill ledger")}
            description={t("Installed packs with tools and removal controls.")}
          />
          <div className="mt-4">
            {skills.map((skill) => (
              <SkillRow
                key={skill.name}
                skill={skill}
                confirmRemove={confirmRemove}
                onConfirmRemove={setConfirmRemove}
                onRemove={() => removeMutation.mutate(skill.name)}
                isRemoving={removeMutation.isPending && removeMutation.variables === skill.name}
                onToggle={(enabled) => toggleMutation.mutate({ id: skill.name, enabled })}
                isToggling={toggleMutation.isPending && toggleMutation.variables?.id === skill.name}
              />
            ))}
          </div>
        </Panel>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function SkillRow({
  skill,
  confirmRemove,
  onConfirmRemove,
  onRemove,
  isRemoving,
  onToggle,
  isToggling,
}: {
  skill: Skill;
  confirmRemove: string | null;
  onConfirmRemove: (name: string | null) => void;
  onRemove: () => void;
  isRemoving: boolean;
  onToggle: (enabled: boolean) => void;
  isToggling: boolean;
}) {
  const { t } = useI18n();
  const isConfirming = confirmRemove === skill.name;
  const isEnabled = skill.enabled !== false;
  const tags = skill.tags ?? [];
  const tools = skill.tools ?? [];

  return (
    <div className="ui-ledger-card px-4 py-4">
      <div className="flex flex-col gap-4 lg:flex-row lg:items-start lg:justify-between">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-3">
            <p
              className="font-display text-lg"
              style={{
                color: isEnabled ? "var(--fg)" : "var(--fg-muted)",
                lineHeight: 1.2,
                letterSpacing: "-0.03em",
                fontWeight: 700,
              }}
            >
              {skill.name ?? t("Untitled skill")}
            </p>
            <span className="ui-chip font-mono">v{skill.version ?? "0.0.0"}</span>
            {!isEnabled ? (
              <span className="ui-chip text-warn" style={{ borderColor: "var(--warn)" }}>
                {t("disabled")}
              </span>
            ) : null}
          </div>
          {skill.description ? (
            <p
              className="mt-2"
              style={{
                color: "var(--fg-soft)",
                fontSize: "13px",
                lineHeight: 1.85,
              }}
            >
              {skill.description}
            </p>
          ) : null}

          <div className="ui-rule-list mt-3">
            <SkillMeta label={t("Author")} value={skill.author ?? t("Unknown")} />
            <SkillMeta label={t("Tools")} value={String(tools.length)} />
            {skill.location ? (
              <SkillMeta label={t("Location")} value={skill.location} mono />
            ) : null}
          </div>

          {tools.length > 0 ? (
            <div className="mt-3">
              <p className="app-section-title">{t("Tool roster")}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {tools.slice(0, 5).map((tool) => (
                  <span key={`${skill.name}-${tool.name}`} className="ui-chip">
                    {tool.name}
                  </span>
                ))}
                {tools.length > 5 ? (
                  <span className="ui-chip">{t("+{count} more", { count: tools.length - 5 })}</span>
                ) : null}
              </div>
            </div>
          ) : null}

          {tags.length > 0 ? (
            <div className="mt-3">
              <p className="app-section-title">{t("Tags")}</p>
              <div className="mt-2 flex flex-wrap gap-2">
                {tags.map((tag) => (
                  <span key={`${skill.name}-${tag}`} className="ui-chip">
                    {tag}
                  </span>
                ))}
              </div>
            </div>
          ) : null}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onClick={() => onToggle(!isEnabled)}
            disabled={isToggling}
            className="ui-button ui-button-muted px-2.5 py-1.5 text-[10px] font-bold uppercase"
            style={{ color: isEnabled ? "var(--warn)" : "var(--accent)" }}
          >
            {isToggling ? "..." : isEnabled ? t("Disable") : t("Enable")}
          </button>
          {isConfirming ? (
            <>
              <button
                type="button"
                onClick={onRemove}
                disabled={isRemoving}
                className="ui-button ui-button-error-soft px-3 py-2 text-[10px] font-bold uppercase"
              >
                {isRemoving ? t("Removing...") : t("Confirm")}
              </button>
              <button
                type="button"
                onClick={() => onConfirmRemove(null)}
                className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase"
              >
                {t("Cancel")}
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => onConfirmRemove(skill.name ?? "")}
              className="ui-button ui-button-muted px-3 py-2 text-[10px] font-bold uppercase text-error"
            >
              {t("Remove")}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

function SkillMeta({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  const { t } = useI18n();

  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{t(label)}</p>
      <p className={mono ? "ui-rule-value font-mono break-all" : "ui-rule-value"}>{value}</p>
    </div>
  );
}
