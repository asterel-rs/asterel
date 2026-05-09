use std::path::Path;

use asterel::core::eval::run_harness_ablation;

#[test]
fn companion_harness_ablation_fixture_shows_reduced_public_failures() {
    let report = run_harness_ablation(Path::new("tests/fixtures/harness"))
        .expect("run harness ablation fixtures");

    assert_eq!(report.off.fixtures, 5);
    assert_eq!(report.on.fixtures, 5);
    assert!(
        report.on.total_constraint_violations < report.off.total_constraint_violations,
        "expected harness-on violations to be lower than harness-off: off={} on={}",
        report.off.total_constraint_violations,
        report.on.total_constraint_violations
    );
    assert_eq!(report.on.total_privacy_exposure_findings, 0);
    assert!(report.off.total_privacy_exposure_findings > 0);
    assert!(report.on.total_template_findings < report.off.total_template_findings);
}
