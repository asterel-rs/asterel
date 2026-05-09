//! Authentication subsystem: profile management, OAuth flows,
//! and API key brokering across providers.

mod broker;
pub(crate) mod oauth;
mod profile;
mod resolution;
mod store;

#[cfg(test)]
mod tests;

use std::time::{SystemTime, UNIX_EPOCH};

pub use broker::{
    AuthBroker, OAuthRecoveryOutcome, OAuthRecoverySkipReason, recover_oauth_profile_for_provider,
    recover_oauth_profile_for_provider_with_outcome,
};
use profile::{AUTH_PROFILES_FILENAME, AUTH_PROFILES_VERSION};
pub use profile::{
    AuthProfile, import_oauth_access_token_for_provider, run_interactive_oauth_for_provider,
};
pub use resolution::auth_profiles_path;
pub(crate) use resolution::{
    auth_target_key, canonical_auth_route, canonical_provider_name, has_secret,
    requested_auth_route,
};
pub use store::AuthProfileStore;

/// Return the current UNIX timestamp in seconds.
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(i64::MAX) // Fallback to MAX to prevent cooldown bypass
}
