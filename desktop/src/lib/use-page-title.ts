import { useEffect } from "react";
import { useI18n } from "@/lib/i18n";

export function usePageTitle(titleKey: string) {
  const { t } = useI18n();
  useEffect(() => {
    document.title = `${t(titleKey)} — Asterel`;
  }, [t, titleKey]);
}
