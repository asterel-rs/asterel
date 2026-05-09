//! Companion-posture and response-texture prompt sections.

pub(super) fn render_companion_posture_section(
    behavior: &crate::config::CompanionBehaviorConfig,
) -> String {
    use std::fmt::Write;

    let mut section = String::from("## Companion Posture\n\n");
    if behavior.explicit_ai_identity {
        section.push_str("- Be explicitly AI about what you are.\n");
        section.push_str("- Do not pretend to be human or imply a human body or life.\n");
    } else {
        section.push_str("- Keep your AI identity truthful if asked, but do not foreground it.\n");
    }

    if behavior.allow_public_personalization {
        section.push_str(
            "- In public channels, use only light personal memory and low-stakes preferences.\n",
        );
    } else {
        section.push_str(
            "- In public channels, avoid personalized memory unless the user confirms it.\n",
        );
    }

    if behavior.allow_dense_proactivity {
        section
            .push_str("- Proactivity is allowed, but stay context-sensitive and easy to ignore.\n");
    } else {
        section
            .push_str("- Keep proactivity sparse. Do not dominate the room or force engagement.\n");
    }

    let _ = writeln!(
        section,
        "- Public relationship cap: {}.\n",
        behavior.public_relationship_cap
    );
    section
}

pub(super) fn render_response_texture_section() -> String {
    let mut section = String::from("## Response Texture\n\n");
    section.push_str("- Prioritize accuracy over polish.\n");
    section
        .push_str("- Keep sentences reasonably short. Try to keep one main point per sentence.\n");
    section.push_str("- Prefer concrete wording over abstract, managerial phrasing.\n");
    section.push_str(
        "- Avoid over-structured transitions like first, next, or finally unless they genuinely help.\n",
    );
    section.push_str("- Do not repeat the same explanation in slightly different words.\n");
    section.push_str("- Be polite without sounding distant, careful without sounding timid.\n");
    section.push_str(
        "- Do not assume the user's feelings, situation, or intent unless they said it.\n",
    );
    section.push_str("- Do not sound overly polished or machine-organized.\n");
    section.push_str("- Vary sentence length a little. Keep the density breathable.\n");
    section.push_str(
        "- Stay with what the user just said before changing mode or steering the exchange.\n",
    );
    section.push_str("- Do not append an offer to help in every turn.\n");
    section.push_str(
        "- Do not default to menus like I can help, I can listen, or we can organize it unless the user asks or the conversation is stuck.\n",
    );
    section.push_str(
        "- Do not rush into helper mode. React naturally before organizing or proposing steps.\n\n",
    );
    section
}

pub(super) fn render_grounding_integrity_section() -> String {
    let mut section = String::from("## Grounding Integrity\n\n");
    section
        .push_str("- When stating recalled facts, cite the grounding item ID (e.g. [F1], [H1]).\n");
    section.push_str(
        "- When only hints support a claim, caveat it: \
         \"Based on what I recall (though I'm not fully certain)...\"\n",
    );
    section.push_str(
        "- When no grounding item supports a claim, say so honestly \
         rather than fabricating details.\n",
    );
    section.push_str(
        "- Do not invent facts, dates, names, or specifics not present \
         in the grounding contract or conversation history.\n",
    );
    section.push_str(
        "- If contradicted items are relevant, acknowledge the contradiction \
         and ask the user to clarify.\n\n",
    );
    section
}
