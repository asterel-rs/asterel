// Route: which secondary tools and automations are available beyond the main operator spine?
import { createFileRoute } from "@tanstack/react-router";
import { useState } from "react";
import { PageHeader, PageShell } from "@/components/page-frame";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { CronTab } from "./-cron";
import { SkillsTab } from "./-skills";

export const Route = createFileRoute("/extensions/")({
  component: ExtensionsPage,
});

type Tab = "skills" | "cron";

function ExtensionsPage() {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<Tab>("skills");

  usePageTitle("Tools");

  const tabs: { value: Tab; label: string }[] = [
    { value: "skills", label: t("Skills") },
    { value: "cron", label: t("Cron") },
  ];

  return (
    <PageShell accent="var(--section-system)">
      <PageHeader
        eyebrow={t("Secondary tools")}
        title={t("Tools")}
        description={t(
          "Skill packs and cron schedules that sit outside the primary companion console spine.",
        )}
      />

      <div className="flex flex-wrap gap-2" role="tablist" aria-label={t("Extension sections")}>
        {tabs.map((tab) => (
          <button
            key={tab.value}
            id={`tab-ext-${tab.value}`}
            type="button"
            onClick={() => setActiveTab(tab.value)}
            data-active={activeTab === tab.value}
            className="ui-segment-button"
            role="tab"
            aria-selected={activeTab === tab.value}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div
        key={activeTab}
        role="tabpanel"
        aria-labelledby={`tab-ext-${activeTab}`}
        style={{ animation: "page-in 200ms var(--ease-out) both" }}
      >
        {activeTab === "skills" && <SkillsTab />}
        {activeTab === "cron" && <CronTab />}
      </div>
    </PageShell>
  );
}
