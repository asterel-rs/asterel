//! Interactive CLI prompts for gathering user and agent context.
//!
//! Collects the user's name, timezone, agent name, and preferred
//! communication style during onboarding.

use anyhow::Result;

use super::super::view::print_bullet;
use crate::ui::style as ui;

/// User and agent context collected during onboarding.
#[derive(Debug, Clone, Default)]
pub(crate) struct ProjectContext {
    /// Display name of the user.
    pub user_name: String,
    /// IANA timezone identifier (e.g. `US/Eastern`).
    pub timezone: String,
    /// Name the user chose for the agent.
    pub agent_name: String,
    /// Free-form style directive for the agent's tone.
    pub communication_style: String,
}

/// # Errors
///
/// Returns an error when interactive prompt input fails.
pub(crate) fn setup_project_context() -> Result<ProjectContext> {
    print_bullet(&t!("onboard.context.intro"));
    print_bullet(&t!("onboard.context.defaults_hint"));
    println!();

    let user_name: String = cliclack::input(format!("  {}", t!("onboard.context.name_prompt")))
        .default_input("User")
        .interact()?;

    let tz_other = t!("onboard.context.tz_other").to_string();
    let tz_options = [
        "US/Eastern (EST/EDT)",
        "US/Central (CST/CDT)",
        "US/Mountain (MST/MDT)",
        "US/Pacific (PST/PDT)",
        "Europe/London (GMT/BST)",
        "Europe/Berlin (CET/CEST)",
        "Asia/Tokyo (JST)",
        "UTC",
        &tz_other,
    ];

    let tz_idx: usize = cliclack::select(format!("  {}", t!("onboard.context.tz_prompt")))
        .item(0usize, tz_options[0], "")
        .item(1usize, tz_options[1], "")
        .item(2usize, tz_options[2], "")
        .item(3usize, tz_options[3], "")
        .item(4usize, tz_options[4], "")
        .item(5usize, tz_options[5], "")
        .item(6usize, tz_options[6], "")
        .item(7usize, tz_options[7], "")
        .item(8usize, tz_options[8], "")
        .initial_value(0usize)
        .interact()?;

    let timezone = if tz_idx == tz_options.len() - 1 {
        cliclack::input(format!("  {}", t!("onboard.context.tz_manual_prompt")))
            .default_input("UTC")
            .interact()?
    } else {
        tz_options[tz_idx]
            .split('(')
            .next()
            .unwrap_or("UTC")
            .trim()
            .to_string()
    };

    let agent_name: String =
        cliclack::input(format!("  {}", t!("onboard.context.agent_name_prompt"))).interact()?;

    let style_options = [
        t!("onboard.context.style_direct").to_string(),
        t!("onboard.context.style_friendly").to_string(),
        t!("onboard.context.style_professional").to_string(),
        t!("onboard.context.style_expressive").to_string(),
        t!("onboard.context.style_technical").to_string(),
        t!("onboard.context.style_balanced").to_string(),
        t!("onboard.context.style_custom").to_string(),
    ];

    let style_idx: usize = cliclack::select(format!("  {}", t!("onboard.context.style_prompt")))
        .item(0usize, style_options[0].clone(), "")
        .item(1usize, style_options[1].clone(), "")
        .item(2usize, style_options[2].clone(), "")
        .item(3usize, style_options[3].clone(), "")
        .item(4usize, style_options[4].clone(), "")
        .item(5usize, style_options[5].clone(), "")
        .item(6usize, style_options[6].clone(), "")
        .initial_value(1usize)
        .interact()?;

    let communication_style = match style_idx {
        0 => "Be direct and concise. Skip pleasantries. Get to the point.".to_string(),
        1 => "Be friendly, human, and conversational. Show warmth and empathy while staying efficient. Use natural contractions.".to_string(),
        2 => "Be professional and polished. Stay calm, structured, and respectful. Use occasional tone-setting emojis only when appropriate.".to_string(),
        3 => "Be expressive and playful when appropriate. Use relevant emojis naturally (0-2 max), and keep serious topics emoji-light.".to_string(),
        4 => "Be technical and detailed. Thorough explanations, code-first.".to_string(),
        5 => "Adapt to the situation. Default to warm and clear communication; be concise when needed, thorough when it matters.".to_string(),
        _ => cliclack::input(format!("  {}", t!("onboard.context.custom_style_prompt")))
            .default_input("Be warm, natural, and clear. Use occasional relevant emojis (1-2 max) and avoid robotic phrasing.")
            .interact()?,
    };

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.context.confirm",
            name = ui::value(&user_name),
            tz = ui::value(&timezone),
            agent = ui::value(&agent_name),
            style = ui::dim_value(&communication_style)
        )
    );

    Ok(ProjectContext {
        user_name,
        timezone,
        agent_name,
        communication_style,
    })
}
