import { type ReactNode, useEffect } from "react";
import { I18nextProvider, useTranslation } from "react-i18next";
import {
  setLocale as changeLocale,
  getCurrentLocale,
  default as i18n,
  type Locale,
  localeTag,
  replaceParams,
} from "@/lib/i18n-core";

export function I18nProvider({ children }: { children: ReactNode }) {
  return <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
}

export function useI18n(): {
  locale: Locale;
  setLocale: (locale: Locale) => void;
  t: (source: string, params?: Record<string, string | number>) => string;
} {
  const { t } = useTranslation();
  const locale = getCurrentLocale();

  useEffect(() => {
    document.documentElement.lang = localeTag(locale);
  }, [locale]);

  return {
    locale,
    setLocale: (nextLocale) => {
      void changeLocale(nextLocale);
    },
    t: (source, params) => {
      const result = params ? t(source, params) : t(source);
      return params ? replaceParams(result, params) : result;
    },
  };
}
