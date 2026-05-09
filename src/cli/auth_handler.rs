//! CLI `auth` subcommand handler.
//!
//! Manages API key storage, OAuth login flows, and auth profile
//! lifecycle (add, remove, list, set-default, import).

use std::io::IsTerminal;
use std::path::Path;

use anyhow::{Context, Result, bail};
use dialoguer::Password;

use crate::config::Config;
use crate::security::SecurityPolicy;
use crate::security::auth::oauth::{
    OAuthProvider, claude_auth_status, codex_login_status, import_claude_setup_token,
    import_codex_oauth, load_codex_auth_file,
};
use crate::security::auth::{
    AuthBroker, AuthProfile, AuthProfileStore, auth_profiles_path, auth_target_key,
    canonical_auth_route, canonical_provider_name, has_secret, unix_now,
};
use crate::ui::style as ui;

#[derive(Debug)]
struct OAuthLoginOptions {
    profile: Option<String>,
    label: Option<String>,
    no_default: bool,
    skip_cli_login: bool,
    setup_token: Option<String>,
}

fn print_auth_store_overview(config: &Config, path: &Path) {
    println!();
    println!("  {}", ui::section("Auth Profiles"));
    println!("{}", ui::field_line("Store", path.display()));
    println!(
        "{}",
        ui::field_line(
            "Encryption",
            if config.secrets.encrypt {
                ui::ok_badge("enabled")
            } else {
                ui::warn_badge("disabled")
            }
        )
    );
}

fn sorted_auth_profiles(store: &AuthProfileStore) -> Vec<&AuthProfile> {
    let mut profiles: Vec<&AuthProfile> = store.profiles.iter().collect();
    profiles.sort_by(|a, b| {
        canonical_provider_name(&a.provider)
            .cmp(&canonical_provider_name(&b.provider))
            .then_with(|| a.id.cmp(&b.id))
    });
    profiles
}

fn print_auth_profile_row(store: &AuthProfileStore, profile: &AuthProfile, now_ts: i64) {
    let provider = canonical_provider_name(&profile.provider);
    let auth_route = canonical_auth_route(
        &profile.provider,
        profile.auth_route.as_deref(),
        profile.auth_scheme.as_deref(),
        profile.oauth_source.as_deref(),
    );
    let target_key = auth_target_key(&provider, auth_route.as_deref());
    let is_default = store
        .defaults
        .get(&target_key)
        .is_some_and(|default_id| default_id == &profile.id);
    let default_marker = if is_default {
        ui::ok_badge("default")
    } else {
        ui::muted_badge("profile")
    };
    let status = if profile.is_disabled {
        ui::warn_badge("disabled")
    } else {
        ui::ok_badge("active")
    };
    let key_state = if has_secret(profile.api_key.as_deref()) {
        ui::ok_badge("set")
    } else {
        ui::warn_badge("missing")
    };
    let label = profile
        .label
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("-");
    let usage = store.usage_stats.get(&profile.id);
    let cooldown_state = usage
        .and_then(|value| value.cooldown_until)
        .filter(|until| *until > now_ts)
        .map_or_else(
            || "ready".to_string(),
            |until| format!("cooldown-until-{until}"),
        );
    let last_used = usage
        .and_then(|value| value.last_used_at)
        .map_or_else(|| "-".to_string(), |value| value.to_string());
    let error_count = usage.map_or(0, |value| value.error_count);
    let disabled_reason = usage
        .and_then(|value| value.disabled_reason.as_deref())
        .unwrap_or("-");

    println!();
    println!(
        "  {} {} {}",
        default_marker,
        ui::header(&profile.id),
        ui::dim(format!("({provider})"))
    );
    println!(
        "{}",
        ui::field_line("Route", auth_route.as_deref().unwrap_or("default"))
    );
    println!(
        "{}",
        ui::field_line("Auth", profile.auth_scheme.as_deref().unwrap_or("api_key"))
    );
    println!("{}", ui::field_line("State", status));
    println!("{}", ui::field_line("Key", key_state));
    println!("{}", ui::field_line("Label", label));
    println!("{}", ui::field_line("Cooldown", cooldown_state));
    println!("{}", ui::field_line("Errors", error_count));
    println!("{}", ui::field_line("Disabled reason", disabled_reason));
    println!("{}", ui::field_line("Last used", last_used));
}

fn stale_default_mappings(store: &AuthProfileStore) -> Vec<(&String, &String)> {
    store
        .defaults
        .iter()
        .filter(|(target_key, profile_id)| {
            !store.profiles.iter().any(|profile| {
                auth_target_key(&profile.provider, profile.auth_route.as_deref()) == **target_key
                    && profile.id == **profile_id
            })
        })
        .collect()
}

fn yes_no_badge(value: bool) -> String {
    if value {
        ui::ok_badge("yes")
    } else {
        ui::muted_badge("no")
    }
}

fn print_profile_resolution(
    store: &AuthProfileStore,
    active_profile: Option<&AuthProfile>,
    uses_config_key: bool,
) {
    match active_profile {
        Some(profile) => {
            println!("{}", ui::field_line("Source", ui::ok_badge("profile")));
            println!("{}", ui::field_line("Profile id", &profile.id));
            println!(
                "{}",
                ui::field_line("Profile label", profile.label.as_deref().unwrap_or("-"))
            );
            println!(
                "{}",
                ui::field_line(
                    "Profile key",
                    if has_secret(profile.api_key.as_deref()) {
                        ui::ok_badge("set")
                    } else {
                        ui::warn_badge("missing")
                    }
                )
            );
            println!(
                "{}",
                ui::field_line(
                    "Profile disabled",
                    if profile.is_disabled {
                        ui::warn_badge("yes")
                    } else {
                        ui::ok_badge("no")
                    }
                )
            );
            if let Some(reason) = store
                .usage_stats
                .get(&profile.id)
                .and_then(|stats| stats.disabled_reason.as_deref())
            {
                println!("{}", ui::field_line("Disabled reason", reason));
            }
            println!(
                "{}",
                ui::field_line(
                    "Auth scheme",
                    profile.auth_scheme.as_deref().unwrap_or("api_key")
                )
            );
            println!(
                "{}",
                ui::field_line(
                    "Auth route",
                    canonical_auth_route(
                        &profile.provider,
                        profile.auth_route.as_deref(),
                        profile.auth_scheme.as_deref(),
                        profile.oauth_source.as_deref(),
                    )
                    .as_deref()
                    .unwrap_or("default")
                )
            );
            println!(
                "{}",
                ui::field_line(
                    "OAuth source",
                    profile.oauth_source.as_deref().unwrap_or("-")
                )
            );
        }
        None if uses_config_key => {
            println!(
                "{}",
                ui::field_line("Source", ui::warn_badge("config.api_key"))
            );
            println!("{}", ui::field_line("Profile id", "-"));
        }
        None => {
            println!("{}", ui::field_line("Source", ui::error_badge("none")));
            println!("{}", ui::field_line("Profile id", "-"));
        }
    }
}

fn print_memory_embedding_status(config: &Config, broker: &AuthBroker) {
    let memory_key_resolved = broker.resolve_memory_api_key(&config.memory).is_some();
    println!();
    println!("  {}", ui::subsection("Memory Embeddings"));
    println!(
        "{}",
        ui::field_line("Provider", &config.memory.embedding_provider)
    );
    println!(
        "{}",
        ui::field_line(
            "Key resolved",
            if memory_key_resolved {
                ui::ok_badge("yes")
            } else {
                ui::warn_badge("no")
            }
        )
    );
}

fn print_codex_oauth_status(store: &AuthProfileStore, security: &SecurityPolicy) {
    println!();
    println!("  {}", ui::subsection("OpenAI / Codex"));

    match codex_login_status(security) {
        Ok(status) => println!("{}", ui::field_line("CLI status", status)),
        Err(err) => println!(
            "{}",
            ui::field_line("CLI status", ui::warn_badge(format!("unavailable ({err})")))
        ),
    }

    match load_codex_auth_file() {
        Ok(parsed) => {
            let has_access = parsed
                .tokens
                .as_ref()
                .and_then(|t| t.access_token.as_deref())
                .is_some_and(|t| !t.trim().is_empty());
            let has_refresh = parsed
                .tokens
                .as_ref()
                .and_then(|t| t.refresh_token.as_deref())
                .is_some_and(|t| !t.trim().is_empty());
            println!(
                "{}",
                ui::field_line(
                    "Local token cache",
                    if has_access {
                        ui::ok_badge("present")
                    } else {
                        ui::warn_badge("missing")
                    }
                )
            );
            println!(
                "{}",
                ui::field_line(
                    "Refresh token cache",
                    if has_refresh {
                        ui::ok_badge("present")
                    } else {
                        ui::warn_badge("missing")
                    }
                )
            );
        }
        Err(err) => println!(
            "{}",
            ui::field_line(
                "Local token cache",
                ui::warn_badge(format!("unavailable ({err})"))
            )
        ),
    }

    let has_profile = store.profiles.iter().any(|p| {
        canonical_provider_name(&p.provider) == "openai"
            && canonical_auth_route(
                &p.provider,
                p.auth_route.as_deref(),
                p.auth_scheme.as_deref(),
                p.oauth_source.as_deref(),
            )
            .as_deref()
                == Some("codex")
            && p.auth_scheme.as_deref() == Some("oauth")
            && !p.is_disabled
            && has_secret(p.api_key.as_deref())
    });
    println!(
        "{}",
        ui::field_line("Stored profile", yes_no_badge(has_profile))
    );
}

fn print_claude_oauth_status(store: &AuthProfileStore, security: &SecurityPolicy) {
    println!();
    println!("  {}", ui::subsection("Claude / Anthropic"));

    match claude_auth_status(security) {
        Ok(status) => {
            println!(
                "{}",
                ui::field_line("CLI logged in", yes_no_badge(status.logged_in))
            );
            println!(
                "{}",
                ui::field_line(
                    "CLI auth method",
                    status.auth_method.as_deref().unwrap_or("unknown")
                )
            );
        }
        Err(err) => println!(
            "{}",
            ui::field_line("CLI status", ui::warn_badge(format!("unavailable ({err})")))
        ),
    }

    let has_profile = store.profiles.iter().any(|p| {
        canonical_provider_name(&p.provider) == "anthropic"
            && p.auth_scheme.as_deref() == Some("oauth")
            && !p.is_disabled
            && has_secret(p.api_key.as_deref())
    });

    println!(
        "{}",
        ui::field_line("Stored profile", yes_no_badge(has_profile))
    );
    println!(
        "{}",
        ui::note_line("Anthropic OAuth uses a setup token (sk-ant-oat01-...).")
    );
    println!(
        "{}",
        ui::command_line("asterel auth oauth-login --provider claude")
    );
}

fn auth_helper_security_policy(base: &SecurityPolicy) -> SecurityPolicy {
    let mut scoped = base.clone();
    allow_auth_helper_if_missing(&mut scoped.allowed_commands, "codex");
    allow_auth_helper_if_missing(&mut scoped.allowed_commands, "claude");
    scoped
}

fn allow_auth_helper_if_missing(commands: &mut Vec<String>, command: &str) {
    if commands.iter().any(|existing| existing == command) {
        return;
    }
    commands.push(command.to_string());
}

/// # Errors
///
/// Returns an error when auth command processing, persistence, or OAuth helper
/// flows fail.
#[allow(clippy::needless_pass_by_value)] // Clap enum dispatched by value at CLI boundary
pub fn handle_command(command: crate::AuthCommands, config: &Config) -> Result<()> {
    let security = SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    let auth_helper_security = auth_helper_security_policy(&security);
    match command {
        crate::AuthCommands::List => handle_list(config),
        crate::AuthCommands::Status { provider } => handle_status(config, provider.as_deref()),
        crate::AuthCommands::Login {
            provider,
            profile,
            label,
            api_key,
            no_default,
        } => handle_login(
            config,
            provider.as_str(),
            profile.as_deref(),
            label,
            api_key,
            no_default,
        ),
        crate::AuthCommands::OAuthLogin {
            provider,
            profile,
            label,
            no_default,
            skip_cli_login,
            setup_token,
        } => handle_oauth_login(
            config,
            &auth_helper_security,
            provider.as_str(),
            OAuthLoginOptions {
                profile,
                label,
                no_default,
                skip_cli_login,
                setup_token,
            },
        ),
        crate::AuthCommands::OAuthStatus { provider } => {
            handle_oauth_status(config, provider.as_deref(), &auth_helper_security)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{auth_helper_security_policy, sorted_auth_profiles, stale_default_mappings};
    use crate::security::SecurityPolicy;
    use crate::security::auth::{AuthProfile, AuthProfileStore};

    #[test]
    fn auth_helper_security_policy_adds_codex_and_claude_without_duplication() {
        let base = SecurityPolicy {
            allowed_commands: vec!["git".to_string(), "codex".to_string()],
            ..SecurityPolicy::default()
        };

        let scoped = auth_helper_security_policy(&base);

        assert!(scoped.allowed_commands.contains(&"git".to_string()));
        assert!(scoped.allowed_commands.contains(&"codex".to_string()));
        assert!(scoped.allowed_commands.contains(&"claude".to_string()));
        assert_eq!(
            scoped
                .allowed_commands
                .iter()
                .filter(|command| command.as_str() == "codex")
                .count(),
            1
        );
    }

    #[test]
    fn sorted_auth_profiles_orders_by_canonical_provider_then_id() {
        let store = AuthProfileStore {
            profiles: vec![
                AuthProfile {
                    id: "b".to_string(),
                    provider: "OpenAI".to_string(),
                    auth_route: None,
                    label: None,
                    api_key: None,
                    refresh_token: None,
                    auth_scheme: None,
                    oauth_source: None,
                    is_disabled: false,
                },
                AuthProfile {
                    id: "a".to_string(),
                    provider: "openai".to_string(),
                    auth_route: None,
                    label: None,
                    api_key: None,
                    refresh_token: None,
                    auth_scheme: None,
                    oauth_source: None,
                    is_disabled: false,
                },
                AuthProfile {
                    id: "z".to_string(),
                    provider: "anthropic".to_string(),
                    auth_route: None,
                    label: None,
                    api_key: None,
                    refresh_token: None,
                    auth_scheme: None,
                    oauth_source: None,
                    is_disabled: false,
                },
            ],
            ..AuthProfileStore::default()
        };

        let ids = sorted_auth_profiles(&store)
            .into_iter()
            .map(|profile| profile.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["z", "a", "b"]);
    }

    #[test]
    fn stale_default_mappings_reports_missing_profile_reference() {
        let mut store = AuthProfileStore::default();
        store
            .defaults
            .insert("openai@api".to_string(), "missing".to_string());
        store.profiles.push(AuthProfile {
            id: "present".to_string(),
            provider: "openai".to_string(),
            auth_route: None,
            label: None,
            api_key: None,
            refresh_token: None,
            auth_scheme: None,
            oauth_source: None,
            is_disabled: false,
        });

        let stale = stale_default_mappings(&store);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].0.as_str(), "openai@api");
        assert_eq!(stale[0].1.as_str(), "missing");
    }
}

fn handle_list(config: &Config) -> Result<()> {
    let store = AuthProfileStore::load_or_init_cfg(config)?;
    let path = auth_profiles_path(config);

    print_auth_store_overview(config, path.as_path());

    if store.profiles.is_empty() {
        println!();
        println!("{}", ui::note_line("No auth profiles yet."));
        println!("{}", ui::note_line("Create one with:"));
        println!(
            "{}",
            ui::command_line(format!(
                "asterel auth login --provider {}",
                config
                    .default_provider
                    .as_deref()
                    .unwrap_or(crate::config::DEFAULT_PROVIDER)
            ))
        );
        return Ok(());
    }

    println!();
    let now_ts = unix_now();
    for profile in sorted_auth_profiles(&store) {
        print_auth_profile_row(&store, profile, now_ts);
    }

    let stale_defaults = stale_default_mappings(&store);

    if !stale_defaults.is_empty() {
        println!();
        println!("  {}", ui::subsection("Stale default mappings"));
        for (target_key, profile_id) in stale_defaults {
            println!(
                "{}",
                ui::field_line(format!("target {target_key}"), ui::error_badge(profile_id))
            );
        }
    }

    println!();
    println!(
        "{}",
        ui::note_line("Green badge marks the target default profile.")
    );
    Ok(())
}

fn handle_status(config: &Config, provider: Option<&str>) -> Result<()> {
    let broker = AuthBroker::load_or_init(config)?;
    let store = AuthProfileStore::load_or_init_cfg(config)?;

    let requested_provider = provider
        .or(config.default_provider.as_deref())
        .unwrap_or(crate::config::DEFAULT_PROVIDER);
    let canonical_provider = canonical_provider_name(requested_provider);
    let requested_route = crate::security::auth::requested_auth_route(requested_provider);
    let target_key = store.effective_target_key_for_provider(requested_provider);

    let active_profile = store.active_profile_for_provider(requested_provider);
    let default_profile_id = store.default_profile_id_for_provider(requested_provider);
    let has_resolved_key = broker.resolve_provider_key(requested_provider).is_some();
    let uses_config_key = active_profile.is_none() && has_secret(config.api_key.as_deref());

    println!();
    println!("  {}", ui::section("Auth Status"));
    println!("{}", ui::field_line("Provider", canonical_provider));
    println!(
        "{}",
        ui::field_line("Route", requested_route.unwrap_or("auto"))
    );
    println!(
        "{}",
        ui::field_line(
            "Resolved key",
            if has_resolved_key {
                ui::ok_badge("yes")
            } else {
                ui::warn_badge("no")
            }
        )
    );

    print_profile_resolution(&store, active_profile, uses_config_key);

    println!(
        "{}",
        ui::field_line("Default mapping", default_profile_id.unwrap_or("(none)"))
    );
    println!("{}", ui::field_line("Effective target", target_key));
    println!(
        "{}",
        ui::field_line(
            "Config api_key",
            if has_secret(config.api_key.as_deref()) {
                ui::ok_badge("set")
            } else {
                ui::warn_badge("missing")
            }
        )
    );

    print_memory_embedding_status(config, &broker);

    Ok(())
}

fn handle_login(
    config: &Config,
    provider: &str,
    profile: Option<&str>,
    label: Option<String>,
    api_key: Option<String>,
    no_default: bool,
) -> Result<()> {
    let canonical_provider = canonical_provider_name(provider);
    if canonical_provider.is_empty() {
        bail!("Provider cannot be empty");
    }

    let mut store = AuthProfileStore::load_or_init_cfg(config)?;
    let profile_id = profile
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(
            || format!("{canonical_provider}-default"),
            ToOwned::to_owned,
        );

    let api_key_value = if let Some(key) = api_key {
        key
    } else {
        if !std::io::stdin().is_terminal() {
            bail!("--api-key is required in non-interactive mode");
        }
        Password::new()
            .with_prompt(format!(
                "API key for provider '{canonical_provider}' (input hidden)"
            ))
            .allow_empty_password(false)
            .interact()
            .context("Failed to read API key from terminal")?
    };

    let created = store.upsert_profile(
        AuthProfile {
            id: profile_id.clone(),
            provider: canonical_provider.clone(),
            auth_route: None,
            label,
            api_key: Some(api_key_value),
            refresh_token: None,
            auth_scheme: Some("api_key".into()),
            oauth_source: None,
            is_disabled: false,
        },
        !no_default,
    )?;

    store.mark_profile_used(&profile_id);

    store.save_for_config(config)?;

    println!();
    println!("  {}", ui::section("Auth Login"));
    println!(
        "{}",
        ui::field_line(
            "Result",
            if created {
                ui::ok_badge("created")
            } else {
                ui::ok_badge("updated")
            }
        )
    );
    println!("{}", ui::field_line("Provider", canonical_provider));
    println!("{}", ui::field_line("Profile", profile_id));
    println!(
        "{}",
        ui::field_line(
            "Default mapping",
            if no_default {
                ui::muted_badge("unchanged")
            } else {
                ui::ok_badge("set")
            }
        )
    );
    println!(
        "{}",
        ui::field_line("Store", auth_profiles_path(config).display())
    );
    println!(
        "{}",
        ui::field_line(
            "Encryption",
            if config.secrets.encrypt {
                ui::ok_badge("enabled")
            } else {
                ui::warn_badge("disabled")
            }
        )
    );

    Ok(())
}

fn handle_oauth_login(
    config: &Config,
    security: &SecurityPolicy,
    provider: &str,
    options: OAuthLoginOptions,
) -> Result<()> {
    let OAuthLoginOptions {
        profile,
        label,
        no_default,
        skip_cli_login,
        setup_token,
    } = options;

    let oauth_provider = OAuthProvider::parse(provider)?;
    let mut store = AuthProfileStore::load_or_init_cfg(config)?;

    let imported = match oauth_provider {
        OAuthProvider::Codex => import_codex_oauth(skip_cli_login, security)?,
        OAuthProvider::Claude => import_claude_setup_token(setup_token)?,
    };

    let profile_id = profile
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map_or_else(
            || imported.default_profile_id.to_string(),
            ToOwned::to_owned,
        );

    let final_label = label.or_else(|| Some(imported.default_label.to_string()));

    let created = store.upsert_profile(
        AuthProfile {
            id: profile_id.clone(),
            provider: imported.target_provider.to_string(),
            auth_route: None,
            label: final_label,
            api_key: Some(imported.access_token),
            refresh_token: imported.refresh_token,
            auth_scheme: Some("oauth".into()),
            oauth_source: Some(imported.source_name.to_string()),
            is_disabled: false,
        },
        !no_default,
    )?;

    store.mark_profile_used(&profile_id);

    store.save_for_config(config)?;

    println!();
    println!("  {}", ui::section("OAuth Login"));
    println!(
        "{}",
        ui::field_line(
            "Result",
            if created {
                ui::ok_badge("created")
            } else {
                ui::ok_badge("updated")
            }
        )
    );
    println!("{}", ui::field_line("Provider", imported.target_provider));
    println!("{}", ui::field_line("Profile", profile_id));
    println!("{}", ui::field_line("OAuth source", imported.source_name));
    println!(
        "{}",
        ui::field_line(
            "Default mapping",
            if no_default {
                ui::muted_badge("unchanged")
            } else {
                ui::ok_badge("set")
            }
        )
    );
    println!(
        "{}",
        ui::field_line("Store", auth_profiles_path(config).display())
    );
    println!(
        "{}",
        ui::field_line(
            "Encryption",
            if config.secrets.encrypt {
                ui::ok_badge("enabled")
            } else {
                ui::warn_badge("disabled")
            }
        )
    );

    Ok(())
}

fn handle_oauth_status(
    config: &Config,
    provider: Option<&str>,
    security: &SecurityPolicy,
) -> Result<()> {
    let filter = provider.map(OAuthProvider::parse).transpose()?;
    let store = AuthProfileStore::load_or_init_cfg(config)?;

    println!();
    println!("  {}", ui::section("OAuth Sources"));

    if filter.is_none() || filter == Some(OAuthProvider::Codex) {
        print_codex_oauth_status(&store, security);
    }

    if filter.is_none() || filter == Some(OAuthProvider::Claude) {
        print_claude_oauth_status(&store, security);
    }

    Ok(())
}
