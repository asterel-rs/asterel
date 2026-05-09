//! Identifier normalization for entity IDs and slot keys.
//!
//! Sanitizes raw strings to a safe character set, collapsing
//! disallowed characters into underscores.

/// Sanitize a raw identifier to contain only `[A-Za-z0-9._-:]` and
/// optionally `/`.  Non-allowed characters collapse to a single `_`.
/// Leading/trailing `_` are trimmed.
pub(crate) fn normalize_identifier(raw: &str, allow_slash: bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_underscore = false;
    for ch in raw.trim().chars() {
        let allowed = ch.is_ascii_alphanumeric()
            || matches!(ch, '.' | '_' | '-' | ':')
            || (allow_slash && ch == '/');
        if allowed {
            out.push(ch);
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_identifier;

    #[test]
    fn spaces_become_underscore() {
        assert_eq!(normalize_identifier("hello world", false), "hello_world");
    }

    #[test]
    fn consecutive_specials_collapse() {
        assert_eq!(normalize_identifier("a@@b", false), "a_b");
    }

    #[test]
    fn slash_allowed_when_flag_true() {
        assert_eq!(normalize_identifier("a/b", true), "a/b");
    }

    #[test]
    fn slash_becomes_underscore_when_flag_false() {
        assert_eq!(normalize_identifier("a/b", false), "a_b");
    }

    #[test]
    fn leading_trailing_underscore_trimmed() {
        assert_eq!(normalize_identifier(" @hello@ ", false), "hello");
    }

    #[test]
    fn dots_colons_dashes_preserved() {
        assert_eq!(
            normalize_identifier("foo.bar:baz-qux", false),
            "foo.bar:baz-qux"
        );
    }
}
