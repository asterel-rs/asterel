use asterel::security::policy::{AutonomyLevel, SecurityPolicy};

use crate::support;

#[test]
fn fuzz_is_command_allowed() {
    support::for_each_fuzz_input(10_000, 4096, |data| {
        let Ok(command) = std::str::from_utf8(data) else {
            return;
        };
        for autonomy in [
            AutonomyLevel::ReadOnly,
            AutonomyLevel::Supervised,
            AutonomyLevel::Full,
        ] {
            let policy = SecurityPolicy {
                autonomy,
                ..SecurityPolicy::default()
            };
            let allowed = policy.is_command_allowed(command);
            // ReadOnly must deny all commands unconditionally.
            if autonomy == AutonomyLevel::ReadOnly {
                assert!(
                    !allowed,
                    "ReadOnly must deny all commands, got allow for: {command:?}"
                );
            }
        }
        // Empty command must always be denied regardless of policy.
        let policy = SecurityPolicy::default();
        assert!(
            !policy.is_command_allowed(""),
            "Empty command must be denied"
        );
    });
}
