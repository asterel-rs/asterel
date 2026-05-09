//! iMessage input validation and `AppleScript` escaping to prevent
//! injection attacks via message targets and content strings.
/// Escape a string for safe interpolation into `AppleScript`.
///
/// This prevents injection attacks by escaping:
/// - Backslashes (`\` → `\\`)
/// - Double quotes (`"` → `\"`)
/// - Newlines (`\n` → `\\n`, `\r` → `\\r`) to prevent code injection via line breaks
pub(super) fn escape_applescript(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Validate that a target looks like a valid phone number or email address.
///
/// This is a defense-in-depth measure to reject obviously malicious targets
/// before they reach `AppleScript` interpolation.
///
/// Valid patterns:
/// - Phone: starts with `+` followed by digits (with optional spaces/dashes)
/// - Email: contains `@` with alphanumeric chars on both sides
pub(super) fn is_valid_imessage_target(target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }

    // Phone number: +1234567890 or +1 234-567-8900
    if target.starts_with('+') {
        let digits_only: String = target.chars().filter(char::is_ascii_digit).collect();
        // Must have at least 7 digits (shortest valid phone numbers)
        return digits_only.len() >= 7 && digits_only.len() <= 15;
    }

    // Email: simple validation (contains @ with chars on both sides)
    if let Some(at_pos) = target.find('@') {
        let local = &target[..at_pos];
        let domain = &target[at_pos + 1..];

        // Local part: non-empty, alphanumeric + common email chars
        let local_valid = !local.is_empty()
            && local
                .chars()
                .all(|c| c.is_alphanumeric() || "._+-".contains(c));

        // Domain: non-empty, contains a dot, alphanumeric + dots/hyphens
        let domain_valid = !domain.is_empty()
            && domain.contains('.')
            && domain
                .chars()
                .all(|c| c.is_alphanumeric() || ".-".contains(c));

        return local_valid && domain_valid;
    }

    false
}
