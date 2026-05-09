#![no_main]
use asterel::security::policy::{AutonomyLevel, SecurityPolicy};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|command: &str| {
    let read_only = SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    };
    let supervised = SecurityPolicy {
        autonomy: AutonomyLevel::Supervised,
        ..SecurityPolicy::default()
    };
    let full = SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        ..SecurityPolicy::default()
    };

    let r = read_only.is_command_allowed(command);
    let s = supervised.is_command_allowed(command);
    let f = full.is_command_allowed(command);

    // Monotonicity oracle: ReadOnly <= Supervised <= Full.
    if r {
        assert!(s, "ReadOnly allows '{command}' but Supervised blocks it");
    }
    if s {
        assert!(f, "Supervised allows '{command}' but Full blocks it");
    }

    // ReadOnly must deny all commands unconditionally.
    assert!(!r, "ReadOnly must deny all commands, got allow for: {command:?}");

    // Determinism oracle.
    let r2 = read_only.is_command_allowed(command);
    assert_eq!(r, r2, "is_command_allowed must be deterministic");

    // Empty command must always be denied (matches bolero target).
    let default_policy = SecurityPolicy::default();
    assert!(
        !default_policy.is_command_allowed(""),
        "Empty command must be denied"
    );
});
