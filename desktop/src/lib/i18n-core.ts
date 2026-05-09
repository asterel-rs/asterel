import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "@/locales/en.json";
import ja from "@/locales/ja.json";

export type Locale = "en" | "ja";

const DEFAULT_LOCALE: Locale = "en";
const LOCALE_STORAGE_KEY = "asterel-locale";

function hasWindow(): boolean {
  return typeof window !== "undefined";
}

export function normalizeLocale(raw: string | null | undefined): Locale {
  if (!raw) {
    return DEFAULT_LOCALE;
  }

  const normalized = raw.toLowerCase();
  if (normalized.startsWith("ja")) {
    return "ja";
  }
  return "en";
}

export function getStoredLocale(): Locale {
  if (!hasWindow()) {
    return DEFAULT_LOCALE;
  }

  try {
    return normalizeLocale(window.localStorage.getItem(LOCALE_STORAGE_KEY));
  } catch {
    return DEFAULT_LOCALE;
  }
}

export function detectInitialLocale(): Locale {
  if (!hasWindow()) {
    return DEFAULT_LOCALE;
  }

  const stored = getStoredLocale();
  if (stored !== DEFAULT_LOCALE) {
    return stored;
  }

  return normalizeLocale(window.navigator.language);
}

export function persistLocale(locale: Locale) {
  if (!hasWindow()) {
    return;
  }

  try {
    window.localStorage.setItem(LOCALE_STORAGE_KEY, locale);
  } catch {
    // Ignore storage failures and keep the active locale in memory.
  }
}

export function localeTag(locale: Locale): string {
  return locale === "ja" ? "ja-JP" : "en-US";
}

export function replaceParams(template: string, params?: Record<string, string | number>): string {
  if (!params) {
    return template;
  }

  return template.replace(/\{(\w+)\}/g, (_match, key: string) => {
    const value = params[key];
    return value == null ? `{${key}}` : String(value);
  });
}

function normalizeInterpolationSyntax(template: string): string {
  return template.replace(/\{\{(\w+)\}\}/g, "{$1}");
}

if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources: {
      en: { translation: en },
      ja: { translation: ja },
    },
    lng: detectInitialLocale(),
    fallbackLng: "en",
    supportedLngs: ["en", "ja"],
    defaultNS: "translation",
    ns: ["translation"],
    interpolation: {
      escapeValue: false,
    },
    keySeparator: false,
    returnEmptyString: false,
  });
}

export function getCurrentLocale(): Locale {
  return normalizeLocale(i18n.resolvedLanguage ?? i18n.language ?? getStoredLocale());
}

export async function setLocale(locale: Locale) {
  persistLocale(locale);
  await i18n.changeLanguage(locale);
}

export function translate(
  locale: Locale,
  source: string,
  params?: Record<string, string | number>,
): string {
  const normalizedSource = normalizeInterpolationSyntax(source);
  const fixedT = i18n.getFixedT(locale);
  const translated = params ? fixedT(normalizedSource, params) : fixedT(normalizedSource);
  return replaceParams(normalizeInterpolationSyntax(translated), params);
}

export function translateNow(
  source: string,
  params?: Record<string, string | number>,
  locale = getCurrentLocale(),
): string {
  return translate(locale, source, params);
}

export function formatNumber(value: number, locale = getStoredLocale()): string {
  return new Intl.NumberFormat(localeTag(locale)).format(value);
}

export function formatDateTime(
  value: string | number | Date,
  options: Intl.DateTimeFormatOptions,
  locale = getCurrentLocale(),
): string {
  try {
    return new Intl.DateTimeFormat(localeTag(locale), options).format(new Date(value));
  } catch {
    return String(value);
  }
}

export default i18n;
