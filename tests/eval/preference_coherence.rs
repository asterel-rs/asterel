use crate::behavioral::{
    AssertionDirection, BehavioralAssertion, BehavioralEvalSpec, run_behavioral_eval,
};

const REQUIRED_MIN_CONSISTENCY: f64 = 0.6;

const REPHRASING_PAIRS: [(&str, &str); 5] = [
    ("I prefer concise answers", "Keep responses brief"),
    ("Use a formal tone", "Please keep this professional"),
    (
        "Give me bullet lists",
        "Structure answers as step-by-step lists",
    ),
    ("Be conversational", "A casual tone works best"),
    ("Go into more detail", "Please elaborate with depth"),
];

const CONTRADICTION_PAIRS: [(&str, &str); 5] = [
    ("Keep things short", "Actually, give detailed explanations"),
    (
        "Stay formal and professional",
        "Drop the formal tone and be casual",
    ),
    (
        "Use bullet points",
        "Avoid lists and write in narrative form",
    ),
    (
        "I prefer concise one-word replies",
        "Please elaborate with more detail and depth",
    ),
    (
        "Use a professional style",
        "Switch to a chill conversational style",
    ),
];

const ANCHORING_PAIRS: [(&str, &str); 5] = [
    (
        "Always answer with one-word replies only",
        "A short paragraph is usually better",
    ),
    (
        "Always write extremely formal legal prose",
        "Normal professional tone is fine",
    ),
    (
        "Everything must be a giant nested outline",
        "Simple bullets are enough",
    ),
    (
        "Never use structure at all",
        "Use light structure when it helps",
    ),
    (
        "Be ultra casual slang all the time",
        "Balanced conversational tone is better",
    ),
];

#[derive(Debug, Clone)]
struct TestValueProfile {
    response_style: f64,
    verbosity: f64,
    formality: f64,
    confidence: f64,
}

impl Default for TestValueProfile {
    fn default() -> Self {
        Self {
            response_style: 0.5,
            verbosity: 0.5,
            formality: 0.5,
            confidence: 0.8,
        }
    }
}

impl TestValueProfile {
    fn apply_preference(&mut self, statement: &str) {
        if let Some(target) = detect_verbosity_target(statement) {
            self.apply_domain_update(Domain::Verbosity, target);
        }

        if let Some(target) = detect_formality_target(statement) {
            self.apply_domain_update(Domain::Formality, target);
        }

        if let Some(target) = detect_response_style_target(statement) {
            self.apply_domain_update(Domain::ResponseStyle, target);
        }
    }

    fn apply_domain_update(&mut self, domain: Domain, target: f64) {
        let current = self.get(domain);
        if (current - target).abs() > 0.25 {
            self.confidence = (self.confidence * 0.8).max(0.2);
        } else {
            self.confidence = (self.confidence + 0.02).min(1.0);
        }

        let alpha = 0.4;
        let unclamped = current + alpha * (target - current);
        let max_step = 0.30;
        let anchored = unclamped.clamp(current - max_step, current + max_step);
        self.set(domain, anchored.clamp(0.0, 1.0));
    }

    fn get(&self, domain: Domain) -> f64 {
        match domain {
            Domain::ResponseStyle => self.response_style,
            Domain::Verbosity => self.verbosity,
            Domain::Formality => self.formality,
        }
    }

    fn set(&mut self, domain: Domain, value: f64) {
        match domain {
            Domain::ResponseStyle => self.response_style = value,
            Domain::Verbosity => self.verbosity = value,
            Domain::Formality => self.formality = value,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Domain {
    ResponseStyle,
    Verbosity,
    Formality,
}

fn detect_verbosity_target(statement: &str) -> Option<f64> {
    let lower = statement.to_lowercase();
    if contains_any(&lower, &["concise", "brief", "short", "one-word"]) {
        return Some(0.2);
    }
    if contains_any(&lower, &["detail", "elaborate", "depth", "paragraph"]) {
        return Some(0.8);
    }
    None
}

fn detect_formality_target(statement: &str) -> Option<f64> {
    let lower = statement.to_lowercase();
    if contains_any(&lower, &["casual", "conversational", "slang", "chill"]) {
        return Some(0.2);
    }
    if contains_any(&lower, &["formal", "professional", "legal"]) {
        return Some(0.8);
    }
    None
}

fn detect_response_style_target(statement: &str) -> Option<f64> {
    let lower = statement.to_lowercase();
    if contains_any(&lower, &["narrative", "no structure"]) {
        return Some(0.2);
    }
    if contains_any(
        &lower,
        &["bullet", "step-by-step", "list", "outline", "structure"],
    ) {
        return Some(0.8);
    }
    None
}

fn contains_any(haystack: &str, words: &[&str]) -> bool {
    words.iter().any(|word| haystack.contains(word))
}

pub(crate) fn preference_coherence_suite() -> BehavioralEvalSpec {
    BehavioralEvalSpec {
        name: "preference_coherence_benchmark".to_string(),
        description:
            "Tests value profile consistency across rephrased and contradictory preferences"
                .to_string(),
        assertions: vec![
            BehavioralAssertion::PreferenceCoherence {
                domain: "response_style".to_string(),
                min_consistency: REQUIRED_MIN_CONSISTENCY,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::PreferenceCoherence {
                domain: "verbosity".to_string(),
                min_consistency: REQUIRED_MIN_CONSISTENCY,
                direction: AssertionDirection::Capability,
            },
            BehavioralAssertion::PreferenceCoherence {
                domain: "formality".to_string(),
                min_consistency: REQUIRED_MIN_CONSISTENCY,
                direction: AssertionDirection::Capability,
            },
        ],
        scenario_count: 15,
    }
}

fn domain_consistency_score(domain: Domain, left: &str, right: &str) -> f64 {
    let mut baseline = TestValueProfile::default();
    baseline.apply_preference(left);
    let left_value = baseline.get(domain);

    let mut baseline = TestValueProfile::default();
    baseline.apply_preference(right);
    let right_value = baseline.get(domain);

    1.0 - (left_value - right_value).abs()
}

#[test]
fn preference_coherence_suite_has_required_structure() {
    let suite = preference_coherence_suite();
    assert_eq!(suite.name, "preference_coherence_benchmark");
    assert_eq!(suite.scenario_count, 15);
    assert!(suite.description.contains("value profile consistency"));
    assert_eq!(suite.assertions.len(), 3);

    let mut domains = std::collections::BTreeSet::new();
    for assertion in suite.assertions {
        if let BehavioralAssertion::PreferenceCoherence {
            domain,
            min_consistency,
            direction,
        } = assertion
        {
            assert!((min_consistency - REQUIRED_MIN_CONSISTENCY).abs() < f64::EPSILON);
            assert_eq!(direction, AssertionDirection::Capability);
            domains.insert(domain);
        }
    }

    assert!(domains.contains("response_style"));
    assert!(domains.contains("verbosity"));
    assert!(domains.contains("formality"));
}

#[test]
fn rephrased_preferences_meet_minimum_consistency_threshold() {
    let verbosity_avg = REPHRASING_PAIRS
        .iter()
        .map(|(a, b)| domain_consistency_score(Domain::Verbosity, a, b))
        .sum::<f64>()
        / REPHRASING_PAIRS.len() as f64;

    let formality_avg = REPHRASING_PAIRS
        .iter()
        .map(|(a, b)| domain_consistency_score(Domain::Formality, a, b))
        .sum::<f64>()
        / REPHRASING_PAIRS.len() as f64;

    let response_style_avg = REPHRASING_PAIRS
        .iter()
        .map(|(a, b)| domain_consistency_score(Domain::ResponseStyle, a, b))
        .sum::<f64>()
        / REPHRASING_PAIRS.len() as f64;

    assert!(verbosity_avg >= REQUIRED_MIN_CONSISTENCY);
    assert!(formality_avg >= REQUIRED_MIN_CONSISTENCY);
    assert!(response_style_avg >= REQUIRED_MIN_CONSISTENCY);
}

#[test]
fn contradictory_preferences_reduce_confidence_and_avoid_flip_flopping() {
    for (first, second) in CONTRADICTION_PAIRS {
        let mut profile = TestValueProfile::default();
        let starting_confidence = profile.confidence;

        profile.apply_preference(first);
        let confidence_after_first = profile.confidence;

        profile.apply_preference(second);
        let confidence_after_second = profile.confidence;

        assert!(confidence_after_second <= confidence_after_first + f64::EPSILON);
        assert!(confidence_after_second <= starting_confidence + f64::EPSILON);
        assert!(profile.verbosity >= 0.0 && profile.verbosity <= 1.0);
        assert!(profile.formality >= 0.0 && profile.formality <= 1.0);
        assert!(profile.response_style >= 0.0 && profile.response_style <= 1.0);
    }
}

#[test]
fn anchoring_scenarios_do_not_lock_profile_into_extremes() {
    for (extreme, moderate) in ANCHORING_PAIRS {
        let mut profile = TestValueProfile::default();
        profile.apply_preference(extreme);
        profile.apply_preference(moderate);

        assert!(profile.verbosity > 0.20 && profile.verbosity < 0.80);
        assert!(profile.formality > 0.20 && profile.formality < 0.80);
        assert!(profile.response_style > 0.20 && profile.response_style < 0.80);
    }
}

#[test]
fn preference_coherence_suite_runs_and_returns_report() {
    let suite = preference_coherence_suite();
    let report = run_behavioral_eval(&suite).expect("behavioral evaluation should run");

    assert_eq!(report.spec_name, suite.name);
    assert_eq!(report.results.len(), 3);
    assert!((report.pass_rate - 1.0).abs() < f64::EPSILON);
    assert!(
        report
            .results
            .iter()
            .all(|result| result.assertion_label.starts_with("preference-coherence:"))
    );
}
