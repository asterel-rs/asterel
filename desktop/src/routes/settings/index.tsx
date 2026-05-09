// Route: which companion-runtime controls and trust settings are active right now?
import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { PageHeader, PageShell } from "@/components/page-frame";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { AuthTab } from "./-auth";
import { ConfigTab } from "./-config";
import { GovernanceTab } from "./-governance";
import { TenantsTab } from "./-tenants";
import { UpdatesTab } from "./-updates";

export const Route = createFileRoute("/settings/")({
  component: SettingsPage,
});

type SettingsTab = "config" | "auth" | "tenants" | "governance" | "updates";

function SettingsPage() {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<SettingsTab>("config");

  usePageTitle("Settings");

  return (
    <PageShell accent="var(--section-system)">
      <PageHeader
        eyebrow={t("Runtime controls")}
        title={t("Settings")}
        description={t(
          "Adjust runtime behavior, trust posture, auth, and tenant scope for the companion daemon.",
        )}
      />

      <div className="flex gap-1" role="tablist" aria-label={t("Settings sections")}>
        {(["config", "auth", "tenants", "governance", "updates"] as const).map((tab) => (
          <button
            key={tab}
            id={`tab-settings-${tab}`}
            type="button"
            data-active={activeTab === tab}
            onClick={() => setActiveTab(tab)}
            className="ui-segment-button"
            role="tab"
            aria-selected={activeTab === tab}
          >
            {t(
              tab === "config"
                ? "Config"
                : tab === "auth"
                  ? "Auth"
                  : tab === "tenants"
                    ? "Tenants"
                    : tab === "governance"
                      ? "Governance"
                      : "Updates",
            )}
          </button>
        ))}
      </div>

      <div
        key={activeTab}
        role="tabpanel"
        aria-labelledby={`tab-settings-${activeTab}`}
        style={{ animation: "page-in 200ms var(--ease-out) both" }}
      >
        {activeTab === "config" && <ConfigTab />}
        {activeTab === "auth" && <AuthTab />}
        {activeTab === "tenants" && <TenantsTab />}
        {activeTab === "governance" && <GovernanceTab />}
        {activeTab === "updates" && <UpdatesTab />}
      </div>
    </PageShell>
  );
}
