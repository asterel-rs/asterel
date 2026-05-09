//! Locale detection from `ASTEREL_LANG`, config, or system
//! `LANG`/`LC_MESSAGES`, with normalisation (e.g. `ja_JP.UTF-8`
//! to `ja`).

use super::Config;

fn detect_system_locale() -> Option<String> {
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .ok()
        .map(|lang| lang.trim().to_lowercase())
        .filter(|lang| !lang.is_empty())
}

/// Detect locale: `ASTEREL_LANG` env -> config value -> system `LANG` -> `"en"`.
fn detect_locale(config_locale: &str) -> String {
    if let Ok(lang) = std::env::var("ASTEREL_LANG") {
        let lang = lang.trim().to_lowercase();
        if !lang.is_empty() {
            return normalise_locale(&lang);
        }
    }

    if config_locale != "en" && !config_locale.is_empty() {
        return normalise_locale(config_locale);
    }

    if let Some(system_locale) = detect_system_locale() {
        return normalise_locale(&system_locale);
    }

    "en".into()
}

/// Normalise `"ja_JP.UTF-8"` -> `"ja"`, `"en_US"` -> `"en"`, passthrough `"ja"`.
fn normalise_locale(raw: &str) -> String {
    let base = raw.split('.').next().unwrap_or(raw);
    let lang = base.split('_').next().unwrap_or(base);
    lang.to_string()
}

impl Config {
    /// Detect locale from env, config, or system, then apply it
    /// via `rust_i18n::set_locale`.
    pub fn apply_locale(&self) {
        let locale = detect_locale(&self.locale);
        rust_i18n::set_locale(&locale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::core::test_env::{ENV_LOCK, EnvVarGuard};

    #[test]
    fn detect_locale_uses_expected_priority_order() {
        let _lock = ENV_LOCK.lock().unwrap();

        let _lang = EnvVarGuard::set("LANG", "pt_BR.UTF-8");
        let _lc_messages = EnvVarGuard::set("LC_MESSAGES", "es_ES.UTF-8");

        let asterel_lang_guard = EnvVarGuard::set("ASTEREL_LANG", "ja_JP.UTF-8");
        assert_eq!(detect_locale("fr_FR"), "ja");
        drop(asterel_lang_guard);

        assert_eq!(detect_locale("fr_FR"), "fr");
        assert_eq!(detect_locale("en"), "pt");

        let _lang_unset = EnvVarGuard::unset("LANG");
        assert_eq!(detect_locale("en"), "es");

        let _lc_messages_unset = EnvVarGuard::unset("LC_MESSAGES");
        assert_eq!(detect_locale("en"), "en");
    }

    #[test]
    fn normalise_locale_handles_common_formats() {
        assert_eq!(normalise_locale("ja_JP.UTF-8"), "ja");
        assert_eq!(normalise_locale("en_US"), "en");
        assert_eq!(normalise_locale("en"), "en");
        assert_eq!(normalise_locale(""), "");
    }
}
