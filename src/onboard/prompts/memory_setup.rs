//! Interactive CLI prompts for memory backend selection.
//!
//! Lets the user choose between `Postgres`, Markdown, or no-memory
//! backends and configure auto-save preferences.

use anyhow::Result;

use super::super::view::print_bullet;
use crate::config::MemoryConfig;
use crate::ui::style as ui;

/// # Errors
///
/// Returns an error when interactive prompt input fails.
pub(crate) fn setup_memory() -> Result<MemoryConfig> {
    print_bullet(&t!("onboard.memory.intro"));
    print_bullet(&t!("onboard.memory.later_hint"));
    println!();

    let choice: usize = cliclack::select(format!("  {}", t!("onboard.memory.select_prompt")))
        .item(0usize, t!("onboard.memory.postgres").to_string(), "")
        .item(1usize, t!("onboard.memory.markdown").to_string(), "")
        .item(2usize, t!("onboard.memory.none").to_string(), "")
        .initial_value(0usize)
        .interact()?;

    let backend = match choice {
        1 => crate::config::MemoryBackend::Markdown,
        2 => crate::config::MemoryBackend::None,
        _ => crate::config::MemoryBackend::Postgres,
    };

    let auto_save = if backend == crate::config::MemoryBackend::None {
        false
    } else {
        cliclack::confirm(format!("  {}", t!("onboard.memory.auto_save_prompt")))
            .initial_value(true)
            .interact()?
    };

    println!(
        "  {} {}",
        ui::success("✓"),
        t!(
            "onboard.memory.confirm",
            backend = ui::value(backend),
            auto_save = if auto_save { "on" } else { "off" }
        )
    );

    Ok(MemoryConfig {
        backend,
        auto_save,
        ..MemoryConfig::default()
    })
}
