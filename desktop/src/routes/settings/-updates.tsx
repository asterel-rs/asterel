import { useCallback, useState } from "react";
import { Panel, SectionLead, StatPill } from "@/components/page-frame";
import { StatusBadge } from "@/components/status-badge";
import { type UpdateInfo, checkForUpdates, installUpdateAndRelaunch } from "@/lib/desktop-shell";
import { useI18n } from "@/lib/i18n";

export function UpdatesTab() {
  const { t } = useI18n();
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleCheck = useCallback(async () => {
    setChecking(true);
    setError(null);
    try {
      const info = await checkForUpdates();
      setUpdateInfo(info);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("Failed to check for updates"));
    } finally {
      setChecking(false);
    }
  }, [t]);

  const handleInstall = useCallback(async () => {
    setInstalling(true);
    try {
      await installUpdateAndRelaunch();
    } catch (err) {
      setError(err instanceof Error ? err.message : t("Failed to install update"));
      setInstalling(false);
    }
  }, [t]);

  return (
    <div className="space-y-4">
      <Panel strong variant="stage" className="px-5 py-5">
        <SectionLead
          title={t("Software updates")}
          description={t("Check for new versions and install updates.")}
          action={
            <button
              type="button"
              onClick={handleCheck}
              disabled={checking || installing}
              className="ui-button ui-button-accent-hint px-4 py-2 text-xs font-bold"
            >
              {checking ? t("Checking...") : t("Check for updates")}
            </button>
          }
        />

        {updateInfo ? (
          <div className="mt-4 space-y-4">
            <div className="flex flex-wrap gap-3">
              <StatPill
                label={t("Status")}
                value={updateInfo.available ? t("Update available") : t("Up to date")}
              />
              {updateInfo.version ? (
                <StatPill
                  label={t("Version")}
                  value={updateInfo.version}
                  tone="var(--accent-strong)"
                />
              ) : null}
            </div>

            {updateInfo.available ? (
              <>
                {updateInfo.body ? (
                  <div
                    className="px-4 py-3"
                    style={{
                      borderLeft:
                        "2px solid color-mix(in oklch, var(--page-accent) 35%, var(--border))",
                      fontSize: "13px",
                      lineHeight: 1.65,
                      color: "var(--fg-soft)",
                      whiteSpace: "pre-wrap",
                    }}
                  >
                    {updateInfo.body}
                  </div>
                ) : null}
                <button
                  type="button"
                  onClick={handleInstall}
                  disabled={installing}
                  className="ui-button ui-button-accent-fill ui-button-stamp"
                >
                  {installing ? t("Installing...") : t("Install and relaunch")}
                </button>
              </>
            ) : (
              <StatusBadge variant="ok" label={t("Up to date")} />
            )}
          </div>
        ) : null}

        {error ? <p className="mt-4 text-xs text-error">{error}</p> : null}
      </Panel>
    </div>
  );
}
