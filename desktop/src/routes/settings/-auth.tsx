import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/empty-state";
import { ErrorState } from "@/components/error-state";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { SkeletonLoader } from "@/components/skeleton-loader";
import { StatusBadge } from "@/components/status-badge";
import { fetchAuthProfiles, patchAuthProfile } from "@/lib/api";
import { useI18n } from "@/lib/i18n";
import type { AuthProfile } from "@/lib/types";

// ---------------------------------------------------------------------------
// Auth tab
// ---------------------------------------------------------------------------

export function AuthTab() {
  const { t } = useI18n();
  const [selectedProfileId, setSelectedProfileId] = useState<string | null>(null);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["auth-profiles"],
    queryFn: fetchAuthProfiles,
  });

  const profileItems = data?.items;
  const profiles = profileItems ?? [];
  const defaults = data?.defaults ?? {};
  const defaultProfileId = defaults.default_profile_id ?? null;
  const selectedProfile =
    profiles.find((profile) => profile.id === selectedProfileId) ??
    profiles.find((profile) => profile.id === defaultProfileId) ??
    null;
  const providerMix = buildProviderMix(profiles);

  useEffect(() => {
    const nextProfiles = profileItems ?? [];
    const nextSelected =
      nextProfiles.find((profile) => profile.id === defaultProfileId)?.id ??
      nextProfiles[0]?.id ??
      null;

    if (nextProfiles.length === 0) {
      if (selectedProfileId !== null) {
        setSelectedProfileId(null);
      }
      return;
    }

    if (!selectedProfileId || !nextProfiles.some((profile) => profile.id === selectedProfileId)) {
      setSelectedProfileId(nextSelected);
    }
  }, [defaultProfileId, profileItems, selectedProfileId]);

  if (isLoading) {
    return <SkeletonLoader />;
  }

  if (isError) {
    return (
      <ErrorState
        title={t("Failed to load auth profiles")}
        message={error instanceof Error ? error.message : t("Could not reach the daemon.")}
        onRetry={() => refetch()}
      />
    );
  }

  if (profiles.length === 0) {
    return (
      <EmptyState
        title={t("No auth profiles")}
        description={t("Add profiles via the daemon CLI or API to configure provider credentials.")}
      />
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between gap-4">
        <div className="flex flex-wrap gap-2">
          <StatPill label={t("Profiles")} value={String(profiles.length)} />
          <StatPill
            label={t("Enabled")}
            value={String(profiles.filter((p) => !p.disabled).length)}
            tone="var(--accent)"
          />
          <StatPill label={t("Default")} value={defaultProfileId ?? t("none")} />
        </div>
        <button
          type="button"
          onClick={() => refetch()}
          className="ui-button ui-button-muted px-4 py-2 text-xs font-bold uppercase text-fg"
        >
          {t("Refresh auth")}
        </button>
      </div>

      <div className="grid gap-4 xl:grid-cols-[minmax(0,0.82fr)_minmax(0,1.18fr)]">
        <Panel strong variant="stage" className="flex min-h-0 flex-col px-5 py-5">
          <SectionLead title={t("Profile rail")} description={t("Select a profile to inspect.")} />
          <div className="mt-4 min-h-0 flex-1 overflow-y-auto">
            {profiles.map((profile, index) => (
              <ProfileRailRow
                key={profile.id}
                profile={profile}
                index={index}
                isSelected={profile.id === selectedProfile?.id}
                isDefault={profile.id === defaultProfileId}
                onSelect={() => setSelectedProfileId(profile.id)}
              />
            ))}
          </div>
        </Panel>

        <SelectedProfileSurface
          profile={selectedProfile}
          isDefault={selectedProfile?.id === defaultProfileId}
          providerMix={providerMix}
        />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function ProfileRailRow({
  profile,
  index,
  isSelected,
  isDefault,
  onSelect,
}: {
  profile: AuthProfile;
  index: number;
  isSelected: boolean;
  isDefault: boolean;
  onSelect: () => void;
}) {
  const { t } = useI18n();

  return (
    <button
      type="button"
      onClick={onSelect}
      data-selected={isSelected}
      aria-label={t("Select profile {name}", { name: profile.label || profile.id })}
      className="ui-ledger-button ui-ledger-card px-4 py-4"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="ui-chip">{String(index + 1).padStart(2, "0")}</span>
            <p className="app-section-title">
              {profile.label || profile.id || t("Unnamed profile")}
            </p>
            {isDefault ? <span className="ui-chip">{t("default")}</span> : null}
          </div>
          <p className="text-fg mt-3 text-sm" style={{ lineHeight: 1.65 }}>
            {profile.provider}
          </p>
          <p className="mt-2 break-all text-xs" style={{ color: "var(--fg-muted)" }}>
            {profile.id}
          </p>
        </div>
        <StatusBadge
          variant={profile.disabled ? "neutral" : "ok"}
          label={profile.disabled ? t("disabled") : t("active")}
        />
      </div>

      <div className="mt-4 flex flex-wrap gap-2">
        <StatusBadge
          variant={profile.has_api_key ? "ok" : "neutral"}
          label={profile.has_api_key ? t("key set") : t("no key")}
        />
        {profile.auth_scheme ? <StatusBadge variant="info" label={profile.auth_scheme} /> : null}
        {profile.oauth_source ? <StatusBadge variant="degraded" label={t("oauth")} /> : null}
      </div>
    </button>
  );
}

function SelectedProfileSurface({
  profile,
  isDefault,
  providerMix,
}: {
  profile: AuthProfile | null;
  isDefault: boolean;
  providerMix: Array<[string, number]>;
}) {
  const { t } = useI18n();

  if (!profile) {
    return (
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Selected profile")}
          description={t("Choose a profile from the rail to inspect it.")}
        />
        <EmptyState
          title={t("No profile selected")}
          description={t("Select a profile from the left rail to inspect it.")}
        />
      </Panel>
    );
  }

  return (
    <Panel strong variant="stage" className="px-5 py-5">
      <SectionLead
        title={t("Selected profile")}
        description={t(
          "Keep one identity pinned while you inspect provider, scheme, and route posture.",
        )}
        action={isDefault ? <span className="ui-chip">{t("default profile")}</span> : undefined}
      />

      <div className="ui-rule-list mt-4">
        <AuthMetric
          label={t("Label")}
          value={profile.label || profile.id || t(t("Unnamed profile"))}
        />
        <AuthMetric label={t("Provider")} value={profile.provider ?? t("—")} />
        <AuthMetric label={t("Profile ID")} value={profile.id ?? t("—")} />
        <AuthMetric label={t("Scheme")} value={profile.auth_scheme ?? t("No scheme reported")} />
        <AuthMetric
          label={t("OAuth source")}
          value={profile.oauth_source ?? t("No OAuth source")}
        />
      </div>

      <div className="mt-5 border-t border-[var(--border)] pt-4">
        <p className="app-section-title">{t("Credential posture")}</p>
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <StatusBadge
            variant={profile.disabled ? "neutral" : "ok"}
            label={profile.disabled ? t("disabled") : t("active")}
          />
          <StatusBadge
            variant={profile.has_api_key ? "ok" : "neutral"}
            label={profile.has_api_key ? t("key set") : t("no key")}
          />
          <AuthToggleButton profileId={profile.id} disabled={profile.disabled} />
        </div>
      </div>

      <div className="mt-5 border-t border-[var(--border)] pt-4">
        <p className="app-section-title">{t("Route surface")}</p>
        {profile.auth_route ? (
          <pre className="ui-code-block mt-3 max-h-55 overflow-auto">{profile.auth_route}</pre>
        ) : (
          <div className="ui-ledger-card mt-3 px-4 py-4">
            <p className="text-sm" style={{ color: "var(--fg-muted)" }}>
              {t("No explicit auth route is attached to this profile.")}
            </p>
          </div>
        )}
      </div>

      {providerMix.length > 0 && (
        <div className="mt-5 border-t border-[var(--border)] pt-4">
          <p className="app-section-title">{t("Provider mix")}</p>
          <div className="ui-rule-list mt-3">
            {providerMix.map(([provider, count]) => (
              <div key={provider} className="ui-rule-row">
                <p className="ui-rule-key">{provider}</p>
                <span className="ui-chip">{t("{count} profiles", { count })}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </Panel>
  );
}

function AuthMetric({ label, value }: { label: string; value: string }) {
  const { t } = useI18n();

  return (
    <div className="ui-rule-row">
      <p className="ui-rule-key">{t(label)}</p>
      <p className="ui-rule-value break-all" style={{ color: "var(--fg-soft)" }}>
        {value}
      </p>
    </div>
  );
}

function AuthToggleButton({ profileId, disabled }: { profileId: string; disabled: boolean }) {
  const { t } = useI18n();
  const queryClient = useQueryClient();

  const toggleMutation = useMutation({
    mutationFn: () => patchAuthProfile(profileId, { disabled: !disabled }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["auth-profiles"] });
    },
  });

  return (
    <button
      type="button"
      onClick={() => toggleMutation.mutate()}
      disabled={toggleMutation.isPending}
      className="ui-button ui-button-accent-hint px-3 py-1 text-xs font-bold uppercase"
      style={{ color: disabled ? "var(--accent)" : "var(--error)" }}
    >
      {toggleMutation.isPending ? t("Updating...") : disabled ? t("Enable") : t("Disable")}
    </button>
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function buildProviderMix(profiles: AuthProfile[]): Array<[string, number]> {
  const counts = new Map<string, number>();

  profiles.forEach((profile) => {
    counts.set(profile.provider, (counts.get(profile.provider) ?? 0) + 1);
  });

  return [...counts.entries()].sort((left, right) => right[1] - left[1]);
}
