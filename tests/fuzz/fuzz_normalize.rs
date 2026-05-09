use asterel::security::external_content::normalize::normalize_detection;

use crate::support;

#[test]
fn fuzz_normalize_for_detection() {
    support::for_each_fuzz_input(10_000, 4096, |data| {
        let Ok(input) = std::str::from_utf8(data) else {
            return;
        };
        let normalized = normalize_detection(input);

        // Output must be ASCII lowercase (no uppercase letters).
        assert!(
            !normalized.chars().any(|c| c.is_ascii_uppercase()),
            "normalize output must not contain uppercase ASCII: {normalized:?}"
        );

        // Known Unicode invisible characters must be stripped.
        for ch in normalized.chars() {
            let is_stripped_invisible = matches!(ch,
                '\u{200B}'..='\u{200F}' |
                '\u{202A}'..='\u{202E}' |
                '\u{2060}'..='\u{2064}' |
                '\u{FEFF}' |
                '\u{FE00}'..='\u{FE0F}' |
                '\u{00AD}'
            );
            assert!(
                !is_stripped_invisible,
                "normalize output contains invisible character U+{:04X}: {normalized:?}",
                ch as u32
            );
        }
    });
}
