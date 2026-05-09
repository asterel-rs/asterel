import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { fetchProviders, fetchSettings, patchProviders, patchSettings } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import { useSound } from "@/lib/use-sound";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface SettingsItem {
  label: string;
  value: string;
  highlight?: boolean;
  fieldKey?: string;
  fieldType?: "text" | "boolean" | "number";
}

interface SettingsWorkbenchSection {
  id: string;
  title: string;
  description: string;
  summary: string;
  note: string;
  items: SettingsItem[];
}

// ---------------------------------------------------------------------------
// Config tab
// ---------------------------------------------------------------------------

export function ConfigTab() {
  const { t } = useI18n();
  const queryClient = useQueryClient();
  const [selectedSectionId, setSelectedSectionId] = useState<string | null>(null);
  const [editMode, setEditMode] = useState(false);
  const [editDraft, setEditDraft] = useState<Record<string, unknown>>({});

  const providersQuery = useQuery({
    queryKey: ["providers"],
    queryFn: fetchProviders,
  });

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: fetchSettings,
  });

  const isLoading = providersQuery.isLoading || settingsQuery.isLoading;
  const isError = providersQuery.isError || settingsQuery.isError;
  const errorMsg =
    providersQuery.error instanceof Error
      ? providersQuery.error.message
      : settingsQuery.error instanceof Error
        ? settingsQuery.error.message
        : t("Failed to load settings");

  const handleRetry = () => {
    void providersQuery.refetch();
    void settingsQuery.refetch();
  };

  const settingsMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) => patchSettings(body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      setEditMode(false);
      setEditDraft({});
    },
  });

  const providersMutation = useMutation({
    mutationFn: (body: Record<string, unknown>) => patchProviders(body),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["providers"] });
      setEditMode(false);
      setEditDraft({});
    },
  });

  const providers = providersQuery.data;
  const settings = settingsQuery.data;
  const sections = buildSettingsSections(providers, settings, t);
  const selectedSection =
    sections.find((section) => section.id === selectedSectionId) ?? sections[0] ?? null;

  useEffect(() => {
    if (sections.length === 0) {
      if (selectedSectionId !== null) {
        setSelectedSectionId(null);
      }
      return;
    }

    if (!selectedSectionId || !sections.some((section) => section.id === selectedSectionId)) {
      setSelectedSectionId(sections[0]?.id ?? null);
    }
  }, [sections, selectedSectionId]);

  useEffect(() => {
    if (editMode && selectedSection) {
      setEditDraft(initDraft(selectedSection));
    }
  }, [selectedSection, editMode]);

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <ErrorState title={t("Failed to load settings")} message={errorMsg} onRetry={handleRetry} />
    );
  }

  return (
    <div className="space-y-6">
      <SoundSection />

      <div className="flex items-center justify-between gap-4">
        <div className="flex flex-wrap gap-2">
          <StatPill label={t("Provider")} value={providers?.active_provider ?? "—"} />
          <StatPill label={t("Model")} value={providers?.active_model ?? t("—")} />
          <StatPill
            label={t("Temp")}
            value={providers?.temperature != null ? String(providers.temperature) : "—"}
          />
        </div>
        <div className="flex items-center gap-2">
          {editMode ? (
            <>
              <button
                type="button"
                onClick={() => {
                  const section = selectedSection;
                  if (!section) return;

                  let body: Record<string, unknown> = {};

                  if (section.id === "providers") {
                    body = {
                      active_provider: editDraft.active_provider,
                      active_model: editDraft.active_model,
                      temperature: Number(editDraft.temperature),
                    };
                    providersMutation.mutate(body);
                  } else if (section.id === "workspace") {
                    body = {
                      workspace_dir: editDraft.workspace_dir,
                    };
                    settingsMutation.mutate(body);
                  } else if (section.id === "memory") {
                    body = {
                      memory: {
                        backend: editDraft.backend,
                        auto_save: editDraft.auto_save,
                      },
                    };
                    settingsMutation.mutate(body);
                  } else if (section.id === "autonomy") {
                    body = {
                      autonomy: {
                        max_tool_loop: Number(editDraft.max_tool_loop),
                      },
                    };
                    settingsMutation.mutate(body);
                  } else if (section.id === "gateway") {
                    const corsOrigins = String(editDraft.cors_origins || "")
                      .split(",")
                      .map((s) => s.trim())
                      .filter(Boolean);
                    body = {
                      gateway: {
                        pairing_enabled: editDraft.pairing_enabled,
                        defense_mode: editDraft.defense_mode,
                        cors_origins: corsOrigins,
                      },
                    };
                    settingsMutation.mutate(body);
                  }
                }}
                disabled={settingsMutation.isPending || providersMutation.isPending}
                className="ui-button ui-button-accent-fill px-4 py-2 text-xs font-bold uppercase"
              >
                {settingsMutation.isPending || providersMutation.isPending
                  ? t("Saving...")
                  : t("Save")}
              </button>
              <button
                type="button"
                onClick={() => {
                  setEditMode(false);
                  setEditDraft({});
                }}
                className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-fg"
              >
                {t("Cancel")}
              </button>
            </>
          ) : (
            <button
              type="button"
              onClick={() => {
                setEditMode(true);
                if (selectedSection) {
                  setEditDraft(initDraft(selectedSection));
                }
              }}
              className="ui-button ui-button-accent-soft px-4 py-2 text-xs font-bold uppercase"
            >
              {t("Edit config")}
            </button>
          )}
          <button
            type="button"
            onClick={handleRetry}
            className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-fg"
          >
            {t("Refresh config")}
          </button>
        </div>
      </div>

      <Panel variant="stage" className="px-5 py-5">
        <SectionLead title={t("Control strip")} description={t("Key guardrails at a glance.")} />
        <div className="mt-4 grid gap-3 md:grid-cols-4">
          <SignalTile
            label={t("Tool loop")}
            value={String(settings?.autonomy.max_tool_loop ?? "-")}
            tone="var(--accent)"
          />
          <SignalTile
            label={t("Pairing")}
            value={settings?.gateway.pairing_enabled ? "enabled" : "disabled"}
            tone={settings?.gateway.pairing_enabled ? "var(--info)" : "var(--fg)"}
          />
          <SignalTile
            label={t("Memory auto save")}
            value={settings?.memory.auto_save ? "enabled" : "disabled"}
            tone={settings?.memory.auto_save ? "var(--accent)" : "var(--fg)"}
          />
          <SignalTile
            label={t("CORS origins")}
            value={String(settings?.gateway.cors_origins?.length ?? 0)}
            tone="var(--warn)"
          />
        </div>
      </Panel>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)_300px]">
        <Panel strong variant="stage" className="flex min-h-0 flex-col px-5 py-5">
          <SectionLead title={t("Config rail")} description={t("Select a section to inspect.")} />
          <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
            {sections.map((section, index) => (
              <SettingsRailRow
                key={section.id}
                section={section}
                index={index}
                isSelected={section.id === selectedSection?.id}
                onSelect={() => setSelectedSectionId(section.id)}
              />
            ))}
          </div>
        </Panel>

        <SelectedSettingsSurface
          section={selectedSection}
          editMode={editMode}
          editDraft={editDraft}
          setEditDraft={setEditDraft}
        />

        <Panel variant="stage" className="px-5 py-5">
          <SectionLead
            title={t("Runtime posture")}
            description={t("Active inference route and workspace.")}
          />
          <div className="ui-rule-list mt-4">
            <SettingsMetric
              label={t("Active provider")}
              value={providers?.active_provider ?? "—"}
            />
            <SettingsMetric label={t("Active model")} value={providers?.active_model ?? "—"} />
            <SettingsMetric label={t("Workspace")} value={settings?.workspace_dir ?? "—"} />
            <SettingsMetric
              label={t("Temperature")}
              value={providers?.temperature != null ? String(providers.temperature) : "—"}
            />
          </div>
        </Panel>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function SettingsRailRow({
  section,
  index,
  isSelected,
  onSelect,
}: {
  section: SettingsWorkbenchSection;
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
      aria-label={t("Select section {name}", { name: section.title })}
      className="ui-ledger-button ui-ledger-card px-4 py-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <p className="app-section-title">{section.title}</p>
          </div>
          <p className="mt-3 text-sm text-fg" style={{ lineHeight: 1.65 }}>
            {section.summary}
          </p>
        </div>
      </div>
      <p className="mt-4 text-sm" style={{ color: "var(--fg-muted)", lineHeight: 1.85 }}>
        {section.description}
      </p>
    </button>
  );
}

function SelectedSettingsSurface({
  section,
  editMode,
  editDraft,
  setEditDraft,
}: {
  section: SettingsWorkbenchSection | null;
  editMode: boolean;
  editDraft: Record<string, unknown>;
  setEditDraft: (draft: Record<string, unknown>) => void;
}) {
  const { t } = useI18n();

  if (!section) {
    return (
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Selected section")}
          description={t("Choose a settings section from the left rail to inspect it.")}
        />
        <SkeletonLoader />
      </Panel>
    );
  }

  return (
    <Panel strong variant="stage" className="px-5 py-5">
      <SectionLead title={section.title} description={section.description} />
      <div className="ui-rule-list mt-4">
        {section.items.map((item) => {
          if (editMode && item.fieldKey && item.fieldType) {
            return (
              <EditableSettingsField
                key={item.label}
                item={item}
                value={editDraft[item.fieldKey]}
                onChange={(newValue) => {
                  setEditDraft({
                    ...editDraft,
                    [item.fieldKey!]: newValue,
                  });
                }}
              />
            );
          }
          return (
            <SettingsMetric
              key={item.label}
              label={item.label}
              value={item.value}
              tone={item.highlight ? "var(--accent)" : "var(--fg-soft)"}
            />
          );
        })}
      </div>
    </Panel>
  );
}

function EditableSettingsField({
  item,
  value,
  onChange,
}: {
  item: SettingsItem;
  value: unknown;
  onChange: (newValue: string | boolean | number) => void;
}) {
  const { t } = useI18n();

  if (item.fieldType === "boolean") {
    const isEnabled = value === true;
    return (
      <div className="ui-rule-row">
        <p className="ui-rule-key">{t(item.label)}</p>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => onChange(true)}
            data-active={isEnabled}
            className="ui-button ui-button-muted px-3 py-2 text-xs font-bold uppercase"
            style={{
              color: isEnabled ? "var(--accent)" : "var(--fg-soft)",
            }}
          >
            {t("Enabled")}
          </button>
          <button
            type="button"
            onClick={() => onChange(false)}
            data-active={!isEnabled}
            className="ui-button ui-button-muted px-3 py-2 text-xs font-bold uppercase"
            style={{
              color: !isEnabled ? "var(--accent)" : "var(--fg-soft)",
            }}
          >
            {t("Disabled")}
          </button>
        </div>
      </div>
    );
  }

  if (item.fieldType === "number") {
    const fieldId = `field-${item.fieldKey ?? item.label}`;
    return (
      <div className="ui-rule-row">
        <label htmlFor={fieldId} className="ui-rule-key">
          {t(item.label)}
        </label>
        <input
          id={fieldId}
          type="number"
          value={String(value ?? "")}
          onChange={(e) => onChange(Number(e.target.value) || 0)}
          className="ui-field"
          style={{ maxWidth: "120px" }}
        />
      </div>
    );
  }

  const fieldId = `field-${item.fieldKey ?? item.label}`;
  return (
    <div className="ui-rule-row">
      <label htmlFor={fieldId} className="ui-rule-key">
        {t(item.label)}
      </label>
      <input
        id={fieldId}
        type="text"
        value={String(value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        className="ui-field"
      />
    </div>
  );
}

function SettingsMetric({ label, value, tone }: { label: string; value: string; tone?: string }) {
  const { t } = useI18n();

  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{t(label)}</p>
      <p className="ui-rule-value break-all" style={{ color: tone ?? "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function SignalTile({ label, value, tone }: { label: string; value: string; tone: string }) {
  const { t } = useI18n();

  return (
    <div className="ui-ledger-card px-4 py-4">
      <div className="flex items-center justify-between gap-3">
        <p className="app-section-title">{t(label)}</p>
        <p className="text-sm" style={{ color: tone }}>
          {t(value)}
        </p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sound settings (client-side, localStorage)
// ---------------------------------------------------------------------------

const SOUND_ENABLED_KEY = "asterel-sound-enabled";
const SOUND_VOLUME_KEY = "asterel-sound-volume";

function readSoundEnabled(): boolean {
  try {
    const stored = window.localStorage.getItem(SOUND_ENABLED_KEY);
    return stored !== "false";
  } catch {
    return true;
  }
}

function readSoundVolume(): number {
  try {
    const stored = window.localStorage.getItem(SOUND_VOLUME_KEY);
    if (stored == null) return 0.3;
    const parsed = Number(stored);
    return Number.isFinite(parsed) ? Math.min(Math.max(parsed, 0), 1) : 0.3;
  } catch {
    return 0.3;
  }
}

function SoundSection() {
  const { t } = useI18n();
  const { play } = useSound();
  const [enabled, setEnabled] = useState(readSoundEnabled);
  const [volume, setVolume] = useState(readSoundVolume);

  const handleToggle = useCallback((next: boolean) => {
    setEnabled(next);
    try {
      window.localStorage.setItem(SOUND_ENABLED_KEY, String(next));
    } catch {
      /* storage full or blocked — ignore */
    }
  }, []);

  const handleVolume = useCallback((next: number) => {
    setVolume(next);
    try {
      window.localStorage.setItem(SOUND_VOLUME_KEY, String(next));
    } catch {
      /* storage full or blocked — ignore */
    }
  }, []);

  return (
    <Panel variant="stage" className="px-5 py-5">
      <span className="app-kicker">{t("Sound")}</span>
      <p className="text-muted mt-2 text-sm leading-relaxed">
        {t("Audio feedback for interactions.")}
      </p>

      <div className="ui-rule-list mt-4">
        <div className="ui-rule-row">
          <p className="ui-rule-key">{t("Sound feedback")}</p>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => handleToggle(true)}
              data-active={enabled}
              className="ui-button ui-button-muted px-3 py-2 text-xs font-bold uppercase"
              style={{ color: enabled ? "var(--accent)" : "var(--fg-soft)" }}
            >
              {t("Enabled")}
            </button>
            <button
              type="button"
              onClick={() => handleToggle(false)}
              data-active={!enabled}
              className="ui-button ui-button-muted px-3 py-2 text-xs font-bold uppercase"
              style={{ color: !enabled ? "var(--accent)" : "var(--fg-soft)" }}
            >
              {t("Disabled")}
            </button>
          </div>
        </div>

        <div className="ui-rule-row">
          <label htmlFor="sound-volume" className="ui-rule-key">
            {t("Volume")}
          </label>
          <div className="flex items-center gap-3">
            <input
              id="sound-volume"
              type="range"
              min={0}
              max={1}
              step={0.1}
              value={volume}
              onChange={(e) => handleVolume(Number(e.target.value))}
              disabled={!enabled}
              className="ui-range"
              style={{ width: "120px" }}
            />
            <span
              className="tabular-nums text-xs"
              style={{ color: "var(--fg-muted)", minWidth: "2.5ch" }}
            >
              {volume.toFixed(1)}
            </span>
          </div>
        </div>

        <div className="ui-rule-row">
          <p className="ui-rule-key">{t("Preview")}</p>
          <button
            type="button"
            onClick={() => play("confirm")}
            disabled={!enabled}
            className="ui-button ui-button-accent-fill px-3 py-2 text-xs font-bold uppercase"
          >
            {t("Test")}
          </button>
        </div>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function initDraft(section: SettingsWorkbenchSection): Record<string, string | boolean | number> {
  const draft: Record<string, string | boolean | number> = {};
  for (const item of section.items) {
    if (item.fieldKey) {
      if (item.fieldType === "boolean") {
        draft[item.fieldKey] = item.value === "Enabled";
      } else if (item.fieldType === "number") {
        draft[item.fieldKey] = Number(item.value) || 0;
      } else {
        draft[item.fieldKey] = item.value;
      }
    }
  }
  return draft;
}

function buildSettingsSections(
  providers: Awaited<ReturnType<typeof fetchProviders>> | undefined,
  settings: Awaited<ReturnType<typeof fetchSettings>> | undefined,
  t: (key: string) => string,
): SettingsWorkbenchSection[] {
  const sections: SettingsWorkbenchSection[] = [];

  if (providers) {
    sections.push({
      id: "providers",
      title: "Provider posture",
      description: "Current inference route and request behavior.",
      summary: `${providers.active_provider} / ${providers.active_model}`,
      note: "This section shapes which provider and model the runtime reaches for by default.",
      items: [
        {
          label: "Active provider",
          value: providers.active_provider ?? "—",
          fieldKey: "active_provider",
          fieldType: "text",
        },
        {
          label: "Active model",
          value: providers.active_model ?? "—",
          fieldKey: "active_model",
          fieldType: "text",
        },
        {
          label: "Temperature",
          value: providers.temperature != null ? String(providers.temperature) : "—",
          fieldKey: "temperature",
          fieldType: "number",
        },
      ],
    });
  }

  if (settings) {
    sections.push({
      id: "workspace",
      title: "Workspace target",
      description: "Primary directory and execution environment.",
      summary: settings.workspace_dir ?? "No workspace",
      note: "This directory is the anchor for most local execution and file-system assumptions.",
      items: [
        {
          label: "Directory",
          value: settings.workspace_dir ?? "—",
          fieldKey: "workspace_dir",
          fieldType: "text",
        },
      ],
    });

    sections.push({
      id: "memory",
      title: "Memory",
      description: "Persistence behavior and backend selection.",
      summary: `${settings.memory.backend} / ${settings.memory.auto_save ? "autosave on" : "autosave off"}`,
      note: "Memory backend and auto-save together determine how much conversational state persists between loops.",
      items: [
        {
          label: "Backend",
          value: settings.memory.backend ?? "—",
          fieldKey: "backend",
          fieldType: "text",
        },
        {
          label: "Auto save",
          value: t(settings.memory.auto_save ? "Enabled" : "Disabled"),
          highlight: settings.memory.auto_save,
          fieldKey: "auto_save",
          fieldType: "boolean",
        },
      ],
    });

    sections.push({
      id: "autonomy",
      title: "Autonomy",
      description: "Tool-loop policy for the companion runtime.",
      summary: `loop ${settings.autonomy.max_tool_loop}`,
      note: "These values set how far the runtime can go before it needs a new review loop or operator intervention.",
      items: [
        {
          label: "Max tool loop",
          value:
            settings.autonomy.max_tool_loop != null ? String(settings.autonomy.max_tool_loop) : "—",
          fieldKey: "max_tool_loop",
          fieldType: "number",
        },
      ],
    });

    sections.push({
      id: "gateway",
      title: "Gateway policy",
      description: "Pairing controls, defense mode, and origin allowlist.",
      summary: `${settings.gateway.defense_mode} / ${settings.gateway.cors_origins?.length ?? 0} origins`,
      note: "Gateway policy decides how the desktop pairs, which origins can reach it, and how defensive the daemon should be at the edge.",
      items: [
        {
          label: "Pairing",
          value: t(settings.gateway.pairing_enabled ? "Enabled" : "Disabled"),
          highlight: settings.gateway.pairing_enabled,
          fieldKey: "pairing_enabled",
          fieldType: "boolean",
        },
        {
          label: "Defense mode",
          value: settings.gateway.defense_mode ?? "—",
          fieldKey: "defense_mode",
          fieldType: "text",
        },
        {
          label: "CORS origins",
          value:
            (settings.gateway.cors_origins?.length ?? 0) > 0
              ? settings.gateway.cors_origins.join(", ")
              : "None configured",
          fieldKey: "cors_origins",
          fieldType: "text",
        },
      ],
    });
  }

  return sections;
}
