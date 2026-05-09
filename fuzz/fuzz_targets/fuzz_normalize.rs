#![no_main]
use asterel::security::external_content::normalize::normalize_detection;
use libfuzzer_sys::fuzz_target;

fn is_safe_idempotency_input(input: &str) -> bool {
    input
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '!' | '#' | '^' | '&' | '(' | ')'))
}

fuzz_target!(|input: &str| {
    let normalized = normalize_detection(input);

    // Output must be lowercase (ASCII portion).
    for ch in normalized.chars() {
        if ch.is_ascii_alphabetic() {
            assert!(ch.is_ascii_lowercase(), "output must be ASCII lowercase");
        }
    }

    // Idempotency oracle for the subset that the production property test
    // defines as stable. Arbitrary inputs can legitimately create new
    // percent/leetspeak/formatting opportunities after the first pass.
    if is_safe_idempotency_input(input) {
        let double = normalize_detection(&normalized);
        assert_eq!(
            normalized, double,
            "normalize_detection must be idempotent for safe ASCII input"
        );
    }

    // Known Unicode invisible characters must be stripped (matches bolero target).
    for ch in normalized.chars() {
        let is_stripped_invisible = matches!(
            ch,
            '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{FEFF}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{00AD}'
        );
        assert!(
            !is_stripped_invisible,
            "normalize output contains invisible character U+{:04X}",
            ch as u32
        );
    }
});
