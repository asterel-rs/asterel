//! Wizard integration tests and re-exports.

#[cfg(test)]
mod tests {
    use crate::onboard::{run_channels_repair_wizard, run_quick_setup, run_wizard};

    type RunQuickSetupFn = for<'a, 'b, 'c, 'd> fn(
        Option<&'a str>,
        Option<&'b str>,
        Option<&'c str>,
        Option<&'d str>,
        bool,
    )
        -> anyhow::Result<(crate::config::Config, bool)>;

    #[test]
    fn resume_after_interruption_wizard_exports_repair_and_interactive_entrypoints() {
        let repair_ptr = run_channels_repair_wizard as *const () as usize;
        let wizard_ptr = run_wizard as *const () as usize;
        assert_ne!(repair_ptr, 0);
        assert_ne!(wizard_ptr, 0);
        assert_ne!(repair_ptr, wizard_ptr);
    }

    #[test]
    fn resume_after_interruption_quick_setup_export_uses_expected_signature() {
        let _: RunQuickSetupFn = run_quick_setup;
        let quick_setup_ptr = run_quick_setup as RunQuickSetupFn as usize;
        assert_ne!(quick_setup_ptr, 0);
    }
}
