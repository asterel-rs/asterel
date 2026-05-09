#![no_main]
use asterel::security::policy::SecurityPolicy;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|path: &str| {
    let policy = SecurityPolicy::default();

    // Must not panic on any input.
    let result = policy.is_path_allowed(path);

    // Determinism oracle.
    let result2 = policy.is_path_allowed(path);
    assert_eq!(
        result, result2,
        "is_path_allowed must be deterministic for '{path}'"
    );

    // Null bytes must always be rejected.
    if path.contains('\0') {
        assert!(!result, "null byte in path must be denied: {path:?}");
    }

    // Path traversal via ".." component must always be rejected.
    if path.split('/').any(|seg| seg == "..") {
        assert!(!result, "path traversal (..) must be denied: {path:?}");
    }

    // Sensitive paths must never be allowed.
    if path == "/etc/shadow" || path == "/etc/passwd" {
        assert!(!result, "sensitive system path '{path}' must be blocked");
    }

    // URL-encoded traversal must be rejected (matches bolero target).
    let lower = path.to_lowercase();
    if lower.contains("%2e%2e") || lower.contains("..%2f") || lower.contains("%2f..") {
        assert!(!result, "URL-encoded traversal must be denied: {path:?}");
    }
});
