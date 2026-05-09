use std::fmt::Write as _;

use super::influence::TasteGuidance;
use super::value_profile::ValueProfile;
use super::value_signals::ValueSignal;
use crate::utils::text::{sanitize_prompt_line, truncate_ellipsis};

const TASTE_CONTRACT_PATTERN_MAX_CHARS: usize = 120;

fn sanitize_contract_pattern(value: &str) -> String {
    truncate_ellipsis(
        sanitize_prompt_line(value).as_str(),
        TASTE_CONTRACT_PATTERN_MAX_CHARS,
    )
}

#[must_use]
pub(crate) fn render_taste_contract(guidance: &TasteGuidance) -> String {
    if !guidance.has_content() && guidance.render_mode == super::modes::RenderMode::ConciseProse {
        return String::new();
    }

    let mut block = String::with_capacity(128);
    block.push_str("[Taste Contract]\n");
    let _ = writeln!(block, "Format: {}", guidance.render_mode.as_instruction());

    if !guidance.preferred_patterns.is_empty() {
        block.push_str("Preferred: ");
        let mut first = true;
        for p in &guidance.preferred_patterns {
            let p = sanitize_contract_pattern(p);
            if p.is_empty() {
                continue;
            }
            if !first {
                block.push_str(", ");
            }
            block.push_str(&p);
            first = false;
        }
        block.push('\n');
    }

    if !guidance.avoid_patterns.is_empty() {
        block.push_str("Avoid: ");
        let mut first = true;
        for p in &guidance.avoid_patterns {
            let p = sanitize_contract_pattern(p);
            if p.is_empty() {
                continue;
            }
            if !first {
                block.push_str(", ");
            }
            block.push_str(&p);
            first = false;
        }
        block.push('\n');
    }

    block
}

#[must_use]
pub(crate) fn render_value_guidance(profile: &ValueProfile) -> String {
    let ordered_signals = [
        ValueSignal::PrefersBrevity,
        ValueSignal::PrefersDetail,
        ValueSignal::PrefersCaution,
        ValueSignal::PrefersAutonomy,
        ValueSignal::PrefersStructure,
        ValueSignal::PrefersInformal,
    ];
    let mut active: Vec<(ValueSignal, f64)> = ordered_signals
        .into_iter()
        .map(|signal| (signal, profile.strength(signal)))
        .filter(|(_, strength)| *strength > 0.3)
        .collect();

    if active.is_empty() {
        return String::new();
    }

    active.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut out = String::with_capacity(128);
    out.push_str("[Value Guidance]\n");
    out.push_str("Precedence: apply only within persona, safety, and style constraints; never override them.\n");
    for (signal, strength) in active.iter().take(4) {
        let label = match signal {
            ValueSignal::PrefersBrevity => "User prefers concise responses",
            ValueSignal::PrefersDetail => "User prefers detailed responses",
            ValueSignal::PrefersCaution => "User prefers cautious, confirmed actions",
            ValueSignal::PrefersAutonomy => "User prefers autonomous execution",
            ValueSignal::PrefersStructure => "User prefers structured output (lists, tables)",
            ValueSignal::PrefersInformal => "User prefers informal tone",
        };
        let _ = writeln!(out, "- [{strength:.2}] {label}");
    }
    out
}
