// Route: are Discord and other runtime channels connected and calibrated correctly?
import { createFileRoute } from "@tanstack/react-router";
import { PageHeader, PageShell } from "@/components/page-frame";
import { useI18n } from "@/lib/i18n";
import { usePageTitle } from "@/lib/use-page-title";
import { ChannelsTab } from "./extensions/-channels";

export const Route = createFileRoute("/channels")({
  component: ChannelsPage,
});

function ChannelsPage() {
  const { t } = useI18n();

  usePageTitle("Channels");

  return (
    <PageShell accent="var(--section-operations)">
      <PageHeader
        eyebrow={t("Runtime posture")}
        title={t("Channels")}
        description={t("Inspect Discord and other channel connections, posture, and diagnostics.")}
      />
      <ChannelsTab />
    </PageShell>
  );
}
