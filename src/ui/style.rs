//! ANSI text styling helpers for CLI output.
//!
//! Provides semantic formatting functions (success, header, dim, warning, error, accent)
//! using direct ANSI escape codes for minimal dependencies.

use std::fmt::Display;

// ANSI escape codes
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const UNDERLINE: &str = "\x1b[4m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const CYAN: &str = "\x1b[36m";
const WHITE: &str = "\x1b[37m";

/// Apply ANSI codes to text
fn styled<D: Display>(text: D, codes: &str) -> String {
    format!("{codes}{text}{RESET}")
}

/// Green bold — success checkmarks, confirmations
pub fn success<D: Display>(text: D) -> String {
    styled(text, &format!("{GREEN}{BOLD}"))
}

/// White bold — section headers, titles
pub fn header<D: Display>(text: D) -> String {
    styled(text, &format!("{WHITE}{BOLD}"))
}

/// Dim — subtitles, secondary text, decorative lines
pub fn dim<D: Display>(text: D) -> String {
    styled(text, DIM)
}

/// Yellow — shell commands, code snippets, warnings
pub fn yellow<D: Display>(text: D) -> String {
    styled(text, YELLOW)
}

/// Red bold — errors, hard failures, denied states
pub fn error<D: Display>(text: D) -> String {
    styled(text, &format!("{RED}{BOLD}"))
}

/// Green — confirmed values, paths, names
pub fn value<D: Display>(text: D) -> String {
    styled(text, GREEN)
}

/// Cyan bold — step numbers, bullet points
pub fn accent<D: Display>(text: D) -> String {
    styled(text, &format!("{CYAN}{BOLD}"))
}

/// Cyan — secondary accent, field labels
pub fn cyan<D: Display>(text: D) -> String {
    styled(text, CYAN)
}

/// Cyan underlined — URLs, links
pub fn url<D: Display>(text: D) -> String {
    styled(text, &format!("{CYAN}{UNDERLINE}"))
}

/// Green dim — secondary confirmed values
pub fn dim_value<D: Display>(text: D) -> String {
    styled(text, &format!("{GREEN}{DIM}"))
}

/// Section title line used by richer CLI surfaces.
pub fn section<D: Display>(title: D) -> String {
    format!("{} {}", accent("◆"), header(title))
}

/// Subsection bullet line used inside a CLI section.
pub fn subsection<D: Display>(title: D) -> String {
    format!("{} {}", accent("•"), header(title))
}

/// Aligned key/value line for status-style CLI output.
pub fn field_line<L: Display, V: Display>(label: L, value: V) -> String {
    format!("  {} {}", cyan(format!("{label:>18}")), value)
}

/// Indented secondary note line.
pub fn note_line<D: Display>(text: D) -> String {
    format!("  {} {}", dim("›"), text)
}

/// Indented command hint line.
pub fn command_line<D: Display>(text: D) -> String {
    format!("       {}", yellow(text))
}

/// Green status badge for ready/active states.
pub fn ok_badge<D: Display>(text: D) -> String {
    success(format!("[{text}]"))
}

/// Yellow status badge for warning/degraded states.
pub fn warn_badge<D: Display>(text: D) -> String {
    yellow(format!("[{text}]"))
}

/// Red status badge for error/blocked states.
pub fn error_badge<D: Display>(text: D) -> String {
    error(format!("[{text}]"))
}

/// Dim status badge for inactive/planned states.
pub fn muted_badge<D: Display>(text: D) -> String {
    dim(format!("[{text}]"))
}

// ── Unified markers ─────────────────────────────────────

/// Green bold checkmark — pass, success, enabled.
#[must_use]
pub fn check() -> String {
    success("✓")
}

/// Red bold cross — fail, error, denied.
#[must_use]
pub fn cross() -> String {
    error("✗")
}

/// Yellow exclamation — warning, needs attention.
#[must_use]
pub fn warn_mark() -> String {
    yellow("!")
}

/// Dim guillemet — hint, note, secondary detail.
#[must_use]
pub fn hint_mark() -> String {
    dim("›")
}

/// Inline pass/fail marker with message.
pub fn pass_line<D: Display>(text: D) -> String {
    format!("  {} {}", check(), text)
}

/// Inline fail marker with message.
pub fn fail_line<D: Display>(text: D) -> String {
    format!("  {} {}", cross(), text)
}

/// Inline warning marker with message.
pub fn warn_line<D: Display>(text: D) -> String {
    format!("  {} {}", warn_mark(), text)
}

/// Inline skip/info marker with message.
pub fn skip_line<D: Display>(text: D) -> String {
    format!("  {} {}", dim("–"), dim(text))
}

// ── Separators & dividers ───────────────────────────────

/// Thin separator line (50 chars).
#[must_use]
pub fn separator() -> String {
    dim("──────────────────────────────────────────────────")
}

/// Heavy separator line (50 chars).
#[must_use]
pub fn heavy_separator() -> String {
    dim("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
}

/// Section with separator: header + decorative line below.
pub fn section_with_rule<D: Display>(title: D) -> String {
    format!("  {}\n  {}", section(title), separator())
}

// ── Box drawing ─────────────────────────────────────────

/// Top border of a styled box.
pub fn box_top<D: Display>(title: D) -> String {
    format!(
        "  {} {} {}",
        accent("┌─"),
        header(title),
        dim("─".repeat(40))
    )
}

/// Middle divider of a styled box.
#[must_use]
pub fn box_mid() -> String {
    format!(
        "  {}",
        dim("├──────────────────────────────────────────────────")
    )
}

/// Bottom border of a styled box.
#[must_use]
pub fn box_bottom() -> String {
    format!(
        "  {}",
        dim("└──────────────────────────────────────────────────")
    )
}

/// Box content line.
pub fn box_line<D: Display>(text: D) -> String {
    format!("  {} {text}", dim("│"))
}

/// Box field line (key-value inside a box).
pub fn box_field<L: Display, V: Display>(label: L, value: V) -> String {
    format!("  {} {}  {}", dim("│"), cyan(format!("{label:>14}")), value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_and_header_preserve_text_content() {
        let input = "operation-complete";
        assert!(success(input).contains(input));
        assert!(header(input).contains(input));
    }

    #[test]
    fn styling_helpers_accept_non_string_display_types() {
        let value_text = value(1234_u32);
        let retries_text = dim_value(0.125_f32);

        assert!(value_text.contains("1234"));
        assert!(retries_text.contains("0.125"));
    }

    #[test]
    fn accent_and_url_include_original_text() {
        let label = "step-1";
        let link = "https://example.test";

        assert!(accent(label).contains(label));
        assert!(url(link).contains(link));
        assert!(cyan("field").contains("field"));
        assert!(yellow("cmd").contains("cmd"));
        assert!(dim("hint").contains("hint"));
        assert!(error("boom").contains("boom"));
    }

    #[test]
    fn cli_layout_helpers_render_original_values() {
        let section_text = section("Runtime");
        let subsection_text = subsection("Profiles");
        let field = field_line("Provider", "openai");
        let note = note_line("No daemon detected");
        let command = command_line("asterel status");

        assert!(section_text.contains("Runtime"));
        assert!(subsection_text.contains("Profiles"));
        assert!(field.contains("Provider"));
        assert!(field.contains("openai"));
        assert!(note.contains("No daemon detected"));
        assert!(command.contains("asterel status"));
        assert!(ok_badge("active").contains("active"));
        assert!(warn_badge("warn").contains("warn"));
        assert!(error_badge("failed").contains("failed"));
        assert!(muted_badge("planned").contains("planned"));
    }
}
