use asterel::security::policy::TenantPolicyContext;

use crate::support;

fn normalized_component(data: &[u8], fallback: &str) -> String {
    let mut out = String::new();
    for byte in data.iter().copied().take(48) {
        let ch = char::from(byte);
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch.to_ascii_lowercase());
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

#[test]
fn fuzz_tenant_policy_scope_enforcement() {
    let disabled = TenantPolicyContext::disabled();
    assert!(disabled.enforce_recall_scope("default").is_ok());

    support::for_each_fuzz_input(10_000, 256, |data| {
        let tenant = normalized_component(data, "tenant-alpha");
        let suffix = normalized_component(data, "user");
        let context = TenantPolicyContext::enabled(&tenant);

        let in_scope = format!("{tenant}:{suffix}");
        assert!(
            context.enforce_recall_scope(&in_scope).is_ok(),
            "same-tenant scoped entity must be allowed: {in_scope}"
        );

        let out_of_scope = format!("other-{tenant}:{suffix}");
        assert!(
            context.enforce_recall_scope(&out_of_scope).is_err(),
            "cross-tenant scope must be rejected: {out_of_scope}"
        );

        assert!(
            context.enforce_recall_scope("default").is_err(),
            "default scope must be rejected when tenant mode is enabled"
        );

        let candidate = String::from_utf8_lossy(data);
        let _ = context.enforce_recall_scope(&candidate);
    });
}
