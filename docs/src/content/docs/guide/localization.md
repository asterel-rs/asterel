---
title: Localization
description: How to extend CLI/runtime localization beyond English and Japanese.
---

# Localization

Asterel currently ships English and Japanese locale files in `locales/en.yml`
and `locales/ja.yml`. Runtime locale selection is driven by `ASTEREL_LANG`, the
configured locale, then the system `LANG`, with English as the fallback.

## Add a locale

1. Copy `locales/en.yml` to `locales/<iso-639-1>.yml`.
2. Keep keys identical across locale files.
3. Prefer concise operator-facing phrasing; avoid changing command names or
   configuration keys through translation.
4. Run the config locale tests and at least one onboarding smoke test.
5. Update documentation if the new locale needs operator-specific setup notes.

Locale expansion should be treated as product work, not only string replacement:
error messages, onboarding prompts, and safety/governance copy need native review
before a locale is presented as supported.
