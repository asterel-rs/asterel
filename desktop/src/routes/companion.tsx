import { createFileRoute } from "@tanstack/react-router";
import { PageHeader, PageShell } from "@/components/page-frame";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { CompanionTab } from "./-companion-tab";

export const Route = createFileRoute("/companion")({
  component: CompanionPage,
});

function CompanionPage() {
  const { t } = useI18n();

  usePageTitle("Companion");

  return (
    <PageShell accent="var(--section-system)">
      <PageHeader
        eyebrow={t("Advanced")}
        title={t("Companion")}
        description={t("Companion scope management and context gates.")}
      />
      <CompanionTab />
    </PageShell>
  );
}
