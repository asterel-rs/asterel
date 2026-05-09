use super::response_finalize_io::{
    NaturalnessFinalizeDecision, contains_explicit_reasoning_tags, contract_mismatch_fallback_text,
    protected_segments_match, push_reason_code, run_naturalness_gate, verifier_reason_codes,
};
use super::{
    NaturalnessFinalizationContext, PreparedNaturalnessContext, ResponseFinalizationRequest,
    ResponseFinalizationResult, ResponseFixResult, apply_deterministic_fixes, audit_response,
    audit_response_against_contract, audit_response_contextual,
};
use crate::core::agent::response_audit::ResponseFixHint;
use crate::utils::text::strip_internal_prompt_blocks;

#[cfg(test)]
pub(crate) fn finalize_response(
    request: ResponseFinalizationRequest<'_>,
) -> ResponseFinalizationResult {
    finalize_response_with_context(request, NaturalnessFinalizationContext::default())
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn finalize_response_with_context(
    request: ResponseFinalizationRequest<'_>,
    naturalness_context: NaturalnessFinalizationContext<'_>,
) -> ResponseFinalizationResult {
    // Control outputs are non-user-facing runtime/control-plane payloads. They must
    // bypass every text-shaping and contract path, including naturalness, because a
    // fallback written for users can corrupt machine/internal output.
    if request.control_output {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let stripped_raw_text = strip_internal_prompt_blocks(request.raw_text);
    let request = ResponseFinalizationRequest {
        raw_text: stripped_raw_text.as_str(),
        ..request
    };

    let naturalness_context = PreparedNaturalnessContext::from_context(naturalness_context);

    // Critical exposure checks intentionally run before contract mismatch handling.
    // Shape/mode contract fallbacks may preserve raw text, so memory/internal-state
    // exposure must get the first user-facing safety decision.
    if request.naturalness_gate_enabled
        && let NaturalnessFinalizeDecision::Blocked(fallback) =
            run_naturalness_gate(request, request.raw_text, &naturalness_context)
    {
        return ResponseFinalizationResult {
            final_text: fallback,
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: vec!["naturalness_block"],
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let contract_mismatch_reason = request.contract.and_then(|contract| {
        audit_response_against_contract(request.raw_text, request.output_mode, *contract)
            .mismatch_reason
    });
    if let Some(reason) = contract_mismatch_reason {
        return ResponseFinalizationResult {
            final_text: contract_mismatch_fallback_text(reason, request.raw_text),
            applied_actions: Vec::new(),
            contract_mismatch_reason: Some(reason),
            micro_rewrite_reason_codes: vec![reason.code()],
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let before = audit_response(request.raw_text, request.output_mode);
    // Run the critical subset again after the audit but before bypasses that preserve
    // raw text. Streaming/structured/reasoning output should skip cosmetic mutation,
    // not exposure blocking.
    if request.naturalness_gate_enabled
        && let NaturalnessFinalizeDecision::Blocked(fallback) =
            run_naturalness_gate(request, request.raw_text, &naturalness_context)
    {
        return ResponseFinalizationResult {
            final_text: fallback,
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: vec!["naturalness_block"],
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }
    if request.streaming_active
        || before.structured_risk
        || contains_explicit_reasoning_tags(request.raw_text)
    {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }

    let ResponseFixResult {
        mut text,
        mut applied_actions,
    } = apply_deterministic_fixes(request.raw_text, request.output_mode);
    let mut reason_codes = verifier_reason_codes(&before);

    if request.naturalness_gate_enabled {
        match run_naturalness_gate(request, text.as_str(), &naturalness_context) {
            NaturalnessFinalizeDecision::Unchanged => {}
            NaturalnessFinalizeDecision::Patched(patched) => {
                if protected_segments_match(request.raw_text, patched.as_str()) {
                    text = patched;
                    applied_actions.push(ResponseFixHint::NaturalnessPatch);
                    push_reason_code(&mut reason_codes, "naturalness_gate");
                }
            }
            NaturalnessFinalizeDecision::Blocked(fallback) => {
                return ResponseFinalizationResult {
                    final_text: fallback,
                    applied_actions: Vec::new(),
                    contract_mismatch_reason: None,
                    micro_rewrite_reason_codes: vec!["naturalness_block"],
                    before_score: before.total_score,
                    after_score: before.total_score,
                    preserved: true,
                };
            }
            NaturalnessFinalizeDecision::RepairNeeded => {
                // MVP behavior: structured repair is reported but not executed here.
                // Non-critical repair candidates keep the original text until a
                // dedicated repair backend can re-run protected-segment checks and the
                // naturalness gate after rewriting.
                push_reason_code(&mut reason_codes, "naturalness_repair_needed");
            }
        }
    }

    if text == request.raw_text {
        return ResponseFinalizationResult {
            final_text: text,
            applied_actions,
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: reason_codes,
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }

    if !protected_segments_match(request.raw_text, text.as_str()) {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: false,
        };
    }

    let after = audit_response(text.as_str(), request.output_mode);

    ResponseFinalizationResult {
        final_text: text,
        applied_actions,
        contract_mismatch_reason: None,
        micro_rewrite_reason_codes: reason_codes,
        before_score: before.total_score,
        after_score: after.total_score,
        preserved: true,
    }
}

/// Extended finalization that includes H-D conversational quality checks.
///
/// Same as `finalize_response` but uses the contextual audit to detect
/// lecture drift and disconnection in addition to the baseline checks.
#[must_use]
#[allow(clippy::too_many_lines)]
#[cfg(test)]
pub(crate) fn finalize_response_contextual(
    request: ResponseFinalizationRequest<'_>,
    user_message: &str,
) -> ResponseFinalizationResult {
    finalize_response_contextual_with_context(
        request,
        user_message,
        NaturalnessFinalizationContext::default(),
    )
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub(crate) fn finalize_response_contextual_with_context(
    request: ResponseFinalizationRequest<'_>,
    user_message: &str,
    naturalness_context: NaturalnessFinalizationContext<'_>,
) -> ResponseFinalizationResult {
    // Keep this ordering aligned with `finalize_response`: control output is not
    // user-facing text and must not be rewritten or contract-fallbacked.
    if request.control_output {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let stripped_raw_text = strip_internal_prompt_blocks(request.raw_text);
    let request = ResponseFinalizationRequest {
        raw_text: stripped_raw_text.as_str(),
        ..request
    };

    let naturalness_context = PreparedNaturalnessContext::from_context(naturalness_context);

    // Critical naturalness exposure must precede contract fallback for the same
    // reason as the non-contextual path: some contract mismatches preserve raw text.
    if request.naturalness_gate_enabled
        && let NaturalnessFinalizeDecision::Blocked(fallback) =
            run_naturalness_gate(request, request.raw_text, &naturalness_context)
    {
        return ResponseFinalizationResult {
            final_text: fallback,
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: vec!["naturalness_block"],
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let contract_mismatch_reason = request.contract.and_then(|contract| {
        audit_response_against_contract(request.raw_text, request.output_mode, *contract)
            .mismatch_reason
    });
    if let Some(reason) = contract_mismatch_reason {
        return ResponseFinalizationResult {
            final_text: contract_mismatch_fallback_text(reason, request.raw_text),
            applied_actions: Vec::new(),
            contract_mismatch_reason: Some(reason),
            micro_rewrite_reason_codes: vec![reason.code()],
            before_score: 0,
            after_score: 0,
            preserved: true,
        };
    }

    let before = audit_response_contextual(request.raw_text, request.output_mode, user_message);
    // Contextual audit may detect conversation quality issues, but critical exposure
    // remains a pre-bypass safety decision even for streamed or structured text.
    if request.naturalness_gate_enabled
        && let NaturalnessFinalizeDecision::Blocked(fallback) =
            run_naturalness_gate(request, request.raw_text, &naturalness_context)
    {
        return ResponseFinalizationResult {
            final_text: fallback,
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: vec!["naturalness_block"],
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }
    if request.streaming_active
        || before.structured_risk
        || contains_explicit_reasoning_tags(request.raw_text)
    {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }

    let ResponseFixResult {
        mut text,
        mut applied_actions,
    } = apply_deterministic_fixes(request.raw_text, request.output_mode);
    let mut reason_codes = verifier_reason_codes(&before);

    if request.naturalness_gate_enabled {
        match run_naturalness_gate(request, text.as_str(), &naturalness_context) {
            NaturalnessFinalizeDecision::Unchanged => {}
            NaturalnessFinalizeDecision::Patched(patched) => {
                if protected_segments_match(request.raw_text, patched.as_str()) {
                    text = patched;
                    applied_actions.push(ResponseFixHint::NaturalnessPatch);
                    push_reason_code(&mut reason_codes, "naturalness_gate");
                }
            }
            NaturalnessFinalizeDecision::Blocked(fallback) => {
                return ResponseFinalizationResult {
                    final_text: fallback,
                    applied_actions: Vec::new(),
                    contract_mismatch_reason: None,
                    micro_rewrite_reason_codes: vec!["naturalness_block"],
                    before_score: before.total_score,
                    after_score: before.total_score,
                    preserved: true,
                };
            }
            NaturalnessFinalizeDecision::RepairNeeded => {
                // See the non-contextual path: repair is telemetry-only in this MVP.
                push_reason_code(&mut reason_codes, "naturalness_repair_needed");
            }
        }
    }

    if text == request.raw_text {
        return ResponseFinalizationResult {
            final_text: text,
            applied_actions,
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: reason_codes,
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: true,
        };
    }

    if !protected_segments_match(request.raw_text, text.as_str()) {
        return ResponseFinalizationResult {
            final_text: request.raw_text.to_string(),
            applied_actions: Vec::new(),
            contract_mismatch_reason: None,
            micro_rewrite_reason_codes: Vec::new(),
            before_score: before.total_score,
            after_score: before.total_score,
            preserved: false,
        };
    }

    let after = audit_response_contextual(text.as_str(), request.output_mode, user_message);

    ResponseFinalizationResult {
        final_text: text,
        applied_actions,
        contract_mismatch_reason: None,
        micro_rewrite_reason_codes: reason_codes,
        before_score: before.total_score,
        after_score: after.total_score,
        preserved: true,
    }
}
