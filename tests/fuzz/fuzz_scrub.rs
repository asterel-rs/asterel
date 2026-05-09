use asterel::security::scrub::scrub_secrets;

use crate::support;

#[test]
fn fuzz_scrub_secret_patterns() {
    support::for_each_fuzz_input(10_000, 4096, |data| {
        let Ok(input) = std::str::from_utf8(data) else {
            return;
        };
        let scrubbed = scrub_secrets(input);

        // Idempotency: scrub(scrub(x)) == scrub(x).
        let double_scrubbed = scrub_secrets(&scrubbed);
        assert_eq!(
            *scrubbed, *double_scrubbed,
            "scrub must be idempotent for input: {input:?}"
        );

        // Output must be valid UTF-8 (guaranteed by Cow<str>, but verify).
        assert!(
            std::str::from_utf8(scrubbed.as_bytes()).is_ok(),
            "scrub output must be valid UTF-8"
        );
    });
}
