//! iMessage channel adapter: macOS `AppleScript` bridge for sending replies
//! via `osascript`.
mod auth;
mod handler;

use std::sync::Arc;

use crate::security::SecurityPolicy;

/// iMessage channel using macOS `AppleScript` bridge.
/// Sends replies via `osascript`.
#[derive(Clone)]
pub struct IMessageChannel {
    allowed_contacts: Vec<String>,
    poll_interval_secs: u64,
    security: Arc<SecurityPolicy>,
}

impl IMessageChannel {
    #[must_use]
    pub fn new(allowed_contacts: Vec<String>) -> Self {
        Self::with_security(allowed_contacts, Arc::new(SecurityPolicy::default()))
    }

    #[must_use]
    pub fn with_security(allowed_contacts: Vec<String>, security: Arc<SecurityPolicy>) -> Self {
        Self {
            allowed_contacts,
            poll_interval_secs: 3,
            security,
        }
    }

    fn is_contact_allowed(&self, sender: &str) -> bool {
        if self.allowed_contacts.iter().any(|u| u == "*") {
            return true;
        }
        self.allowed_contacts
            .iter()
            .any(|u| u.eq_ignore_ascii_case(sender))
    }
}
