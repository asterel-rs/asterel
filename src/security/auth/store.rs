//! Persistent auth profile store backed by JSON on disk.
//!
//! Manages multiple auth profiles per provider with encrypted
//! secrets, usage statistics, and default profile selection.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::resolution::{
    auth_profiles_path, auth_secret_store, auth_target_key, canonical_auth_route,
    canonical_provider_name, decrypt_opt_secret, encrypt_opt_secret, is_valid_profile_id,
    requested_auth_route,
};
use super::{AUTH_PROFILES_VERSION, AuthProfile};
use crate::config::Config;
use crate::security::SecretStore;

fn default_auth_profiles_version() -> u32 {
    AUTH_PROFILES_VERSION
}

/// Source-of-truth store for auth profiles.
/// Persistent store managing multiple auth profiles per provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProfileStore {
    /// Schema version for forward compatibility.
    #[serde(default = "default_auth_profiles_version")]
    pub version: u32,
    /// Default profile id per canonical provider name.
    #[serde(default)]
    pub defaults: HashMap<String, String>,
    /// Ordered profile id lists per canonical provider name.
    #[serde(default)]
    pub order: HashMap<String, Vec<String>>,
    /// Last successfully used profile id per provider.
    #[serde(default)]
    pub last_good: HashMap<String, String>,
    /// Usage and cooldown statistics keyed by profile id.
    #[serde(default)]
    pub usage_stats: HashMap<String, ProfileUsageStats>,
    /// All registered auth profiles.
    #[serde(default)]
    pub profiles: Vec<AuthProfile>,
}

/// Usage statistics and cooldown state for a single profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileUsageStats {
    /// Unix timestamp of the last successful use.
    #[serde(default)]
    pub last_used_at: Option<i64>,
    /// Unix timestamp until which the profile is on cooldown.
    #[serde(default)]
    pub cooldown_until: Option<i64>,
    /// Consecutive error count since last success.
    #[serde(default)]
    pub error_count: u32,
    /// Operator-visible reason for automatic disable/secret clear actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AuthTargetKey(String);

impl AuthTargetKey {
    fn new(provider: &str, auth_route: Option<&str>) -> Self {
        Self(auth_target_key(provider, auth_route))
    }

    fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone)]
struct ProviderTargetKeys {
    canonical_provider: String,
    requested_target_key: Option<AuthTargetKey>,
    api_target_key: AuthTargetKey,
}

impl ProviderTargetKeys {
    fn new(provider: &str) -> Self {
        let canonical_provider = canonical_provider_name(provider);
        let requested_target_key = requested_auth_route(provider)
            .map(|route| AuthTargetKey::new(&canonical_provider, Some(route)));
        let api_target_key = AuthTargetKey::new(&canonical_provider, Some("api"));

        Self {
            canonical_provider,
            requested_target_key,
            api_target_key,
        }
    }

    fn preferred_lookup_target_keys(&self) -> Vec<&AuthTargetKey> {
        let mut target_keys = Vec::with_capacity(2);
        if let Some(requested_target_key) = self.requested_target_key.as_ref() {
            target_keys.push(requested_target_key);
        }
        if !target_keys
            .iter()
            .any(|target_key| target_key.as_str() == self.api_target_key.as_str())
        {
            target_keys.push(&self.api_target_key);
        }
        target_keys
    }

    fn default_target_key(&self) -> AuthTargetKey {
        self.requested_target_key
            .clone()
            .unwrap_or_else(|| self.api_target_key.clone())
    }
}

impl Default for AuthProfileStore {
    fn default() -> Self {
        Self {
            version: AUTH_PROFILES_VERSION,
            defaults: HashMap::new(),
            order: HashMap::new(),
            last_good: HashMap::new(),
            usage_stats: HashMap::new(),
            profiles: Vec::new(),
        }
    }
}

impl AuthProfileStore {
    fn profile_target_key(profile: &AuthProfile) -> AuthTargetKey {
        let auth_route = canonical_auth_route(
            &profile.provider,
            profile.auth_route.as_deref(),
            profile.auth_scheme.as_deref(),
            profile.oauth_source.as_deref(),
        );
        AuthTargetKey::new(&profile.provider, auth_route.as_deref())
    }

    fn normalize_profile(profile: &mut AuthProfile) -> bool {
        let canonical_provider = canonical_provider_name(&profile.provider);
        let normalized_auth_scheme = profile.auth_scheme.as_deref().and_then(|value| {
            let trimmed = value.trim().to_ascii_lowercase();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let normalized_oauth_source = profile.oauth_source.as_deref().and_then(|value| {
            let trimmed = value.trim().to_ascii_lowercase();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let normalized_auth_route = canonical_auth_route(
            &profile.provider,
            profile.auth_route.as_deref(),
            normalized_auth_scheme.as_deref(),
            normalized_oauth_source.as_deref(),
        );

        let mut changed = false;
        if profile.provider != canonical_provider {
            profile.provider = canonical_provider;
            changed = true;
        }
        if profile.auth_route != normalized_auth_route {
            profile.auth_route = normalized_auth_route;
            changed = true;
        }
        if profile.auth_scheme != normalized_auth_scheme {
            profile.auth_scheme = normalized_auth_scheme;
            changed = true;
        }
        if profile.oauth_source != normalized_oauth_source {
            profile.oauth_source = normalized_oauth_source;
            changed = true;
        }

        changed
    }

    fn profile_indexes_for_target_key(&self, target_key: &AuthTargetKey) -> Vec<usize> {
        self.profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                (!profile.is_disabled
                    && Self::profile_target_key(profile).as_str() == target_key.as_str())
                .then_some(index)
            })
            .collect()
    }

    fn profile_indexes_for_backend(&self, provider: &str) -> Vec<usize> {
        let canonical = canonical_provider_name(provider);
        self.profiles
            .iter()
            .enumerate()
            .filter_map(|(index, profile)| {
                (!profile.is_disabled && canonical_provider_name(&profile.provider) == canonical)
                    .then_some(index)
            })
            .collect()
    }

    fn target_key_has_profiles(&self, target_key: &AuthTargetKey) -> bool {
        !self.profile_indexes_for_target_key(target_key).is_empty()
    }

    fn first_existing_backend_target_key(&self, canonical_provider: &str) -> Option<AuthTargetKey> {
        self.profile_indexes_for_backend(canonical_provider)
            .into_iter()
            .next()
            .map(|index| Self::profile_target_key(&self.profiles[index]))
    }

    fn cooldown_active(stats: Option<&ProfileUsageStats>, now_ts: i64) -> bool {
        stats
            .and_then(|value| value.cooldown_until)
            .is_some_and(|until| until > now_ts)
    }

    fn pick_profile_index_from_candidates(
        &self,
        target_key: &AuthTargetKey,
        candidate_indexes: Vec<usize>,
        ignore_cooldown: bool,
    ) -> Option<usize> {
        let now_ts = super::unix_now();
        if candidate_indexes.is_empty() {
            return None;
        }

        let is_candidate = |profile_id: &str| {
            candidate_indexes.iter().copied().find(|index| {
                let profile = &self.profiles[*index];
                if profile.id != profile_id {
                    return false;
                }
                if ignore_cooldown {
                    return true;
                }
                let stats = self.usage_stats.get(profile_id);
                !Self::cooldown_active(stats, now_ts)
            })
        };

        if let Some(default_id) = self.defaults.get(target_key.as_str())
            && let Some(index) = is_candidate(default_id)
        {
            return Some(index);
        }

        if let Some(order_list) = self.order.get(target_key.as_str())
            && let Some(index) = order_list
                .iter()
                .find_map(|profile_id| is_candidate(profile_id))
        {
            return Some(index);
        }

        if let Some(last_good_id) = self.last_good.get(target_key.as_str())
            && let Some(index) = is_candidate(last_good_id)
        {
            return Some(index);
        }

        candidate_indexes
            .into_iter()
            .filter(|index| {
                if ignore_cooldown {
                    return true;
                }
                let profile_id = &self.profiles[*index].id;
                let stats = self.usage_stats.get(profile_id);
                !Self::cooldown_active(stats, now_ts)
            })
            .min_by_key(|index| {
                let profile_id = &self.profiles[*index].id;
                self.usage_stats
                    .get(profile_id)
                    .and_then(|stats| stats.last_used_at)
                    .unwrap_or(0)
            })
    }

    fn pick_profile_index_for_provider(
        &self,
        provider: &str,
        ignore_cooldown: bool,
    ) -> Option<usize> {
        let target_keys = ProviderTargetKeys::new(provider);
        for target_key in target_keys.preferred_lookup_target_keys() {
            if let Some(index) = self.pick_profile_index_from_candidates(
                target_key,
                self.profile_indexes_for_target_key(target_key),
                ignore_cooldown,
            ) {
                return Some(index);
            }
        }

        self.profile_indexes_for_backend(&target_keys.canonical_provider)
            .into_iter()
            .filter(|index| {
                if ignore_cooldown {
                    return true;
                }
                let profile_id = &self.profiles[*index].id;
                let stats = self.usage_stats.get(profile_id);
                !Self::cooldown_active(stats, super::unix_now())
            })
            .min_by_key(|index| {
                let profile_id = &self.profiles[*index].id;
                self.usage_stats
                    .get(profile_id)
                    .and_then(|stats| stats.last_used_at)
                    .unwrap_or(0)
            })
    }

    fn target_key_for_profile_order(&self, provider: &str) -> AuthTargetKey {
        let target_keys = ProviderTargetKeys::new(provider);

        if let Some(target_key) = target_keys
            .preferred_lookup_target_keys()
            .into_iter()
            .find(|target_key| self.target_key_has_profiles(target_key))
        {
            return target_key.clone();
        }

        self.first_existing_backend_target_key(&target_keys.canonical_provider)
            .unwrap_or_else(|| target_keys.default_target_key())
    }

    pub(crate) fn effective_target_key_for_provider(&self, provider: &str) -> String {
        self.target_key_for_profile_order(provider).into_string()
    }

    pub(crate) fn default_profile_id_for_provider(&self, provider: &str) -> Option<&str> {
        let target_key = self.target_key_for_profile_order(provider);
        self.defaults.get(target_key.as_str()).map(String::as_str)
    }

    fn normalize_metadata(&mut self) -> bool {
        let mut changed = false;

        for profile in &mut self.profiles {
            changed |= Self::normalize_profile(profile);
        }

        let mut target_ids: HashMap<AuthTargetKey, Vec<String>> = HashMap::new();
        let mut target_by_profile_id: HashMap<String, AuthTargetKey> = HashMap::new();
        for profile in &self.profiles {
            let target_key = Self::profile_target_key(profile);
            target_by_profile_id.insert(profile.id.clone(), target_key.clone());
            target_ids
                .entry(target_key)
                .or_default()
                .push(profile.id.clone());
        }

        let mut normalized_defaults = HashMap::new();
        for profile_id in self.defaults.values() {
            if let Some(target_key) = target_by_profile_id.get(profile_id) {
                normalized_defaults.insert(target_key.as_str().to_string(), profile_id.clone());
            } else {
                changed = true;
            }
        }
        if self.defaults != normalized_defaults {
            self.defaults = normalized_defaults;
            changed = true;
        }

        let mut normalized_last_good = HashMap::new();
        for profile_id in self.last_good.values() {
            if let Some(target_key) = target_by_profile_id.get(profile_id) {
                normalized_last_good.insert(target_key.as_str().to_string(), profile_id.clone());
            } else {
                changed = true;
            }
        }
        if self.last_good != normalized_last_good {
            self.last_good = normalized_last_good;
            changed = true;
        }

        self.usage_stats.retain(|profile_id, _| {
            let keep = self
                .profiles
                .iter()
                .any(|profile| profile.id == *profile_id);
            if !keep {
                changed = true;
            }
            keep
        });

        let mut normalized_order: HashMap<String, Vec<String>> = HashMap::new();
        for ordered_ids in self.order.values() {
            for profile_id in ordered_ids {
                let Some(target_key) = target_by_profile_id.get(profile_id) else {
                    changed = true;
                    continue;
                };
                let entry = normalized_order
                    .entry(target_key.as_str().to_string())
                    .or_default();
                if !entry.iter().any(|id| id == profile_id) {
                    entry.push(profile_id.clone());
                }
            }
        }
        for (target_key, profile_ids) in &target_ids {
            let entry = normalized_order
                .entry(target_key.as_str().to_string())
                .or_default();
            for profile_id in profile_ids {
                if !entry.iter().any(|id| id == profile_id) {
                    entry.push(profile_id.clone());
                }
            }
        }
        if self.order != normalized_order {
            self.order = normalized_order;
            changed = true;
        }

        for (target_key, profile_ids) in &target_ids {
            if !self.order.contains_key(target_key.as_str()) {
                self.order
                    .insert(target_key.as_str().to_string(), profile_ids.clone());
                changed = true;
            }
        }

        changed
    }

    /// Set the priority order of profiles for a provider.
    pub fn set_profile_order(&mut self, provider: &str, ordered_profile_ids: &[String]) {
        let target_key = self.target_key_for_profile_order(provider);
        let mut filtered = Vec::new();
        for profile_id in ordered_profile_ids {
            if self.profiles.iter().any(|profile| {
                Self::profile_target_key(profile) == target_key && profile.id == *profile_id
            }) && !filtered.iter().any(|id| id == profile_id)
            {
                filtered.push(profile_id.clone());
            }
        }
        for profile in &self.profiles {
            if Self::profile_target_key(profile) != target_key {
                continue;
            }
            if !filtered.iter().any(|id| id == &profile.id) {
                filtered.push(profile.id.clone());
            }
        }
        self.order.insert(target_key.into_string(), filtered);
    }

    /// Record a successful use and clear cooldown for a profile.
    pub fn mark_profile_used(&mut self, profile_id: &str) {
        if let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.id == profile_id)
        {
            self.last_good.insert(
                Self::profile_target_key(profile).into_string(),
                profile_id.to_string(),
            );
        }
        let entry = self.usage_stats.entry(profile_id.to_string()).or_default();
        entry.last_used_at = Some(super::unix_now());
        entry.cooldown_until = None;
        entry.error_count = 0;
        entry.disabled_reason = None;
    }

    /// Record a failure and optionally set a cooldown period.
    pub fn mark_profile_failed(&mut self, profile_id: &str, cooldown_secs: Option<i64>) {
        let entry = self.usage_stats.entry(profile_id.to_string()).or_default();
        entry.error_count = entry.error_count.saturating_add(1);
        if let Some(seconds) = cooldown_secs.filter(|value| *value > 0) {
            entry.cooldown_until = Some(super::unix_now().saturating_add(seconds));
        }
    }

    /// Return the active (best-priority, non-cooldown) profile for a
    /// provider.
    pub(crate) fn active_profile_for_provider(&self, provider: &str) -> Option<&AuthProfile> {
        self.active_profile_index_for_provider(provider)
            .map(|index| &self.profiles[index])
    }

    /// Return the index of the active profile for a provider.
    pub(super) fn active_profile_index_for_provider(&self, provider: &str) -> Option<usize> {
        self.pick_profile_index_for_provider(provider, false)
            .or_else(|| self.pick_profile_index_for_provider(provider, true))
    }

    pub(super) fn disabled_oauth_profile_exists_for_provider(&self, provider: &str) -> bool {
        let target_keys = ProviderTargetKeys::new(provider);
        self.profiles.iter().any(|profile| {
            profile.is_disabled
                && profile.auth_scheme.as_deref() == Some("oauth")
                && (target_keys
                    .preferred_lookup_target_keys()
                    .iter()
                    .any(|target_key| {
                        Self::profile_target_key(profile).as_str() == target_key.as_str()
                    })
                    || canonical_provider_name(&profile.provider) == target_keys.canonical_provider)
        })
    }

    /// Return the active API key for a provider, if available.
    pub(super) fn active_api_key_for_provider(&self, provider: &str) -> Option<String> {
        self.active_profile_for_provider(provider)
            .and_then(|profile| profile.api_key.as_deref())
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(ToOwned::to_owned)
    }

    fn load_from_disk(
        path: &Path,
        store: &SecretStore,
        encrypt_enabled: bool,
    ) -> Result<(Self, bool)> {
        if !path.exists() {
            return Ok((Self::default(), false));
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read auth profile store: {}", path.display()))?;
        let mut loaded: Self = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse auth profile store: {}", path.display()))?;

        let mut needs_persist = false;
        let mut disabled_reasons: Vec<(String, String)> = Vec::new();
        for profile in &mut loaded.profiles {
            match decrypt_opt_secret(&mut profile.api_key, store, encrypt_enabled) {
                Ok(changed) => needs_persist |= changed,
                Err(e) => {
                    tracing::warn!(
                        profile_id = %profile.id,
                        provider = %profile.provider,
                        "Failed to decrypt api_key for auth profile — disabling: {e:#}"
                    );
                    disabled_reasons.push((
                        profile.id.clone(),
                        "api_key decrypt failed; profile disabled".to_string(),
                    ));
                    profile.api_key = None;
                    profile.is_disabled = true;
                    needs_persist = true;
                }
            }
            match decrypt_opt_secret(&mut profile.refresh_token, store, encrypt_enabled) {
                Ok(changed) => needs_persist |= changed,
                Err(e) => {
                    tracing::warn!(
                        profile_id = %profile.id,
                        provider = %profile.provider,
                        "Failed to decrypt refresh_token for auth profile — clearing: {e:#}"
                    );
                    disabled_reasons.push((
                        profile.id.clone(),
                        "refresh_token decrypt failed; token cleared".to_string(),
                    ));
                    profile.refresh_token = None;
                    needs_persist = true;
                }
            }
        }
        for (profile_id, reason) in disabled_reasons {
            loaded
                .usage_stats
                .entry(profile_id)
                .or_default()
                .disabled_reason = Some(reason);
        }

        Ok((loaded, needs_persist))
    }

    fn save_to_disk(&self, path: &Path, store: &SecretStore, encrypt_enabled: bool) -> Result<()> {
        let mut persisted = self.clone();

        if encrypt_enabled {
            for profile in &mut persisted.profiles {
                encrypt_opt_secret(&mut profile.api_key, store)?;
                encrypt_opt_secret(&mut profile.refresh_token, store)?;
            }
        }

        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create auth profile store parent directory: {}",
                parent.display()
            )
        })?;

        let json = serde_json::to_string_pretty(&persisted)?;

        // Atomic write: write to a temp file in the same directory, then rename.
        // This prevents corruption if the process crashes mid-write.
        let tmp_path = path.with_extension("tmp");
        fs::write(&tmp_path, &json).with_context(|| {
            format!(
                "Failed to write auth profile store temp file: {}",
                tmp_path.display()
            )
        })?;

        if let Err(error) = crate::security::private_file_permissions::restrict_private_file(
            &tmp_path,
            "auth profile store",
        ) {
            let _ = fs::remove_file(&tmp_path);
            return Err(error);
        }

        fs::rename(&tmp_path, path)
            .inspect_err(|_error| {
                let _ = fs::remove_file(&tmp_path);
            })
            .with_context(|| {
                format!(
                    "Failed to atomically rename auth profile store: {} -> {}",
                    tmp_path.display(),
                    path.display()
                )
            })?;

        Ok(())
    }

    /// # Errors
    ///
    /// Returns an error when loading, normalizing persistence state, or saving
    /// migrated profile metadata fails.
    pub(crate) fn load_or_init_at(
        auth_profiles_path: &Path,
        store: &SecretStore,
        encrypt_enabled: bool,
    ) -> Result<Self> {
        let (mut profile_store, mut needs_persist) =
            Self::load_from_disk(auth_profiles_path, store, encrypt_enabled)?;

        needs_persist |= profile_store.normalize_metadata();

        if needs_persist {
            profile_store.save_to_disk(auth_profiles_path, store, encrypt_enabled)?;
        }

        Ok(profile_store)
    }

    /// # Errors
    ///
    /// Returns an error when loading, normalizing persistence state, or saving
    /// migrated profile metadata fails.
    pub fn load_or_init_cfg(config: &Config) -> Result<Self> {
        let auth_profiles_path = auth_profiles_path(config);
        let store = auth_secret_store(config);
        Self::load_or_init_at(&auth_profiles_path, &store, config.secrets.encrypt)
    }

    /// # Errors
    ///
    /// Returns an error when encrypting or writing profile data to disk fails.
    pub fn save_for_config(&self, config: &Config) -> Result<()> {
        let auth_profiles_path = auth_profiles_path(config);
        let store = auth_secret_store(config);
        self.save_to_disk(&auth_profiles_path, &store, config.secrets.encrypt)
    }

    /// Insert or update a profile, returning `true` if a new profile
    /// was created.
    ///
    /// # Errors
    ///
    /// Returns an error if the profile id is invalid, provider is empty,
    /// or a profile id conflict is detected.
    pub(crate) fn upsert_profile(
        &mut self,
        profile: AuthProfile,
        set_default: bool,
    ) -> Result<bool> {
        let profile_id = profile.id.trim();
        if profile_id.is_empty() {
            bail!("Profile id cannot be empty");
        }
        if !is_valid_profile_id(profile_id) {
            bail!("Invalid profile id '{profile_id}'. Use letters, numbers, '-', '_', or '.'");
        }

        let canonical_provider = canonical_provider_name(&profile.provider);
        if canonical_provider.is_empty() {
            bail!("Provider cannot be empty");
        }

        let normalized_label = profile.label.and_then(|label| {
            let trimmed = label.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });

        let normalized_api_key = profile.api_key.and_then(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        let normalized_refresh_token = profile.refresh_token.and_then(|key| {
            let trimmed = key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        let normalized_auth_scheme = profile.auth_scheme.and_then(|kind| {
            let trimmed = kind.trim().to_ascii_lowercase();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let normalized_oauth_source = profile.oauth_source.and_then(|source| {
            let trimmed = source.trim().to_ascii_lowercase();
            (!trimmed.is_empty()).then_some(trimmed)
        });
        let normalized_auth_route = canonical_auth_route(
            &profile.provider,
            profile.auth_route.as_deref(),
            normalized_auth_scheme.as_deref(),
            normalized_oauth_source.as_deref(),
        );
        let target_key = AuthTargetKey::new(&canonical_provider, normalized_auth_route.as_deref());

        if let Some(existing) = self.profiles.iter_mut().find(|p| p.id == profile_id) {
            if canonical_provider_name(&existing.provider) != canonical_provider {
                bail!(
                    "Profile id '{profile_id}' already belongs to provider '{}'",
                    existing.provider
                );
            }

            existing.provider.clone_from(&canonical_provider);
            existing.label = normalized_label;
            existing.api_key = normalized_api_key;
            existing.refresh_token = normalized_refresh_token;
            existing.auth_route = normalized_auth_route;
            existing.auth_scheme = normalized_auth_scheme;
            existing.oauth_source = normalized_oauth_source;
            existing.is_disabled = false;

            // Reset stale usage stats so re-onboarding starts clean
            if let Some(stats) = self.usage_stats.get_mut(profile_id) {
                stats.cooldown_until = None;
                stats.error_count = 0;
            }

            if set_default {
                self.defaults
                    .insert(target_key.into_string(), profile_id.to_string());
            }
            self.normalize_metadata();
            return Ok(false);
        }

        let profile_id_owned = profile_id.to_string();
        self.profiles.push(AuthProfile {
            id: profile_id_owned.clone(),
            provider: canonical_provider,
            auth_route: normalized_auth_route,
            label: normalized_label,
            api_key: normalized_api_key,
            refresh_token: normalized_refresh_token,
            auth_scheme: normalized_auth_scheme,
            oauth_source: normalized_oauth_source,
            is_disabled: false,
        });

        self.order
            .entry(target_key.as_str().to_string())
            .or_default()
            .push(profile_id_owned.clone());
        self.usage_stats
            .entry(profile_id_owned.clone())
            .or_default();

        if set_default {
            self.defaults
                .insert(target_key.as_str().to_string(), profile_id_owned.clone());
            self.last_good
                .insert(target_key.into_string(), profile_id_owned);
        }

        self.normalize_metadata();
        Ok(true)
    }
}
