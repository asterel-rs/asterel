use asterel::security::policy::SecurityPolicy;

use crate::support;

#[test]
fn fuzz_is_path_allowed() {
    support::for_each_fuzz_input(10_000, 4096, |data| {
        let Ok(path) = std::str::from_utf8(data) else {
            return;
        };
        let policy = SecurityPolicy::default();
        let allowed = policy.is_path_allowed(path);

        // Null bytes must always be rejected.
        if path.contains('\0') {
            assert!(!allowed, "Null byte in path must be denied: {path:?}");
        }

        // Path traversal via ".." component must always be rejected.
        if path.split('/').any(|seg| seg == "..") {
            assert!(!allowed, "Path traversal (..) must be denied: {path:?}");
        }

        // URL-encoded traversal (%2e%2e or %2E%2E, ..%2f, %2f..) must be rejected.
        let lower = path.to_lowercase();
        if lower.contains("%2e%2e") || lower.contains("..%2f") || lower.contains("%2f..") {
            assert!(!allowed, "URL-encoded traversal must be denied: {path:?}");
        }
    });
}
