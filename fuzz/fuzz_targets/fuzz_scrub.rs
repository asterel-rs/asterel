#![no_main]
use asterel::security::scrub::scrub_secrets;
use libfuzzer_sys::fuzz_target;

fn is_secret_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '+' | '/' | '=')
}

fn token_end(input: &str, from: usize) -> usize {
    let mut end = from;
    for (i, c) in input[from..].char_indices() {
        if is_secret_char(c) {
            end = from + i + c.len_utf8();
        } else {
            break;
        }
    }
    end
}

fuzz_target!(|input: &str| {
    let scrubbed = scrub_secrets(input);

    // Idempotency oracle: scrubbing twice must produce the same result.
    let double = scrub_secrets(&scrubbed);
    assert_eq!(*scrubbed, *double, "scrub must be idempotent");

    // UTF-8 validity oracle (matches bolero target).
    assert!(
        std::str::from_utf8(scrubbed.as_bytes()).is_ok(),
        "scrub output must be valid UTF-8"
    );

    // Known-prefix detection oracle: if input contains a known secret prefix
    // followed by token characters, the output must contain [REDACTED].
    for prefix in [
        "sk-", "ghp_", "AKIA", "ASIA", "xoxb-", "xoxp-", "glpat-", "hf_", "gho_", "ghu_",
        "ghs_",
    ] {
        if let Some(start) = input.find(prefix) {
            let content_start = start + prefix.len();
            let end = token_end(input, content_start);
            if end > content_start {
                let token = &input[start..end];
                assert!(
                    !scrubbed.contains(token),
                    "prefix '{prefix}' token must not survive scrubbing, got: {scrubbed:?}"
                );
            }
        }
    }

    // Known-marker detection oracle: Authorization headers must be redacted.
    for marker in [
        "Authorization: Bearer ",
        "authorization: bearer ",
        "api_key=",
        "access_token=",
    ] {
        if let Some(start) = input.find(marker) {
            let content_start = start + marker.len();
            let end = token_end(input, content_start);
            if end > content_start {
                let token = &input[start..end];
                assert!(
                    !scrubbed.contains(token),
                    "marker '{marker}' token must not survive scrubbing, got: {scrubbed:?}"
                );
            }
        }
    }
});
