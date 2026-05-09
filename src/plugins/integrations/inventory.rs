#![allow(clippy::missing_errors_doc)]

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::plugins::integrations::IntegrationEntry;

const REGISTRY_SOURCE: &str = include_str!("registry/catalog.rs");
const SCOPE_LOCK_BASELINE: &str = include_str!("inventory_scope_lock.json");
const CAPABILITY_MATRIX_SOURCE: &str = include_str!("integration_capability_matrix.json");

const INTEGRATION_SOURCE: &str = "integrations";
const INTEGRATION_PRIORITY: u8 = 1;
const INTEGRATION_HEADER: &str = "coming_soon_count";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopeLockInventory {
    pub coming_soon_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InventoryTodoEntry {
    pub source: String,
    pub category: String,
    pub status: String,
    pub priority: u8,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationCapabilityMatrix {
    pub schema_version: String,
    pub source_file: String,
    pub entries: Vec<IntegrationCapabilityMatrixEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IntegrationCapabilityMatrixEntry {
    pub name: String,
    pub implemented: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationCapabilityDrift {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub expectation: String,
}

impl ScopeLockInventory {
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

impl IntegrationCapabilityMatrix {
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
}

pub fn load_integration_capability_matrix() -> Result<IntegrationCapabilityMatrix> {
    parse_integration_capability_matrix(CAPABILITY_MATRIX_SOURCE)
}

pub fn parse_integration_capability_matrix(
    matrix_source: &str,
) -> Result<IntegrationCapabilityMatrix> {
    serde_json::from_str(matrix_source)
        .map_err(|error| anyhow!("invalid integration capability matrix artifact: {error}"))
}

pub fn validate_integration_status_against_matrix(
    matrix: &IntegrationCapabilityMatrix,
    entries: &[IntegrationEntry],
    config: &Config,
) -> std::result::Result<(), Vec<IntegrationCapabilityDrift>> {
    let normalized = normalize_capability_matrix_entries(matrix.entries.clone());
    let mut drifts = Vec::new();

    let mut by_name: HashMap<&str, bool> = HashMap::new();
    for entry in &normalized {
        by_name.insert(entry.name.as_str(), entry.implemented);
    }

    let registry_names: HashSet<&str> = entries.iter().map(|entry| entry.name).collect();

    for name in by_name.keys() {
        if !registry_names.contains(name) {
            drifts.push(IntegrationCapabilityDrift {
                name: (*name).to_string(),
                kind: "stale_matrix_entry".to_string(),
                status: "unknown".to_string(),
                expectation: "remove or rename artifact entry".to_string(),
            });
        }
    }

    for entry in entries {
        let status = (entry.status_fn)(config);
        let matrix_implemented = by_name.get(entry.name).copied().unwrap_or(false);

        if matches!(
            status,
            crate::plugins::integrations::IntegrationStatus::Active
                | crate::plugins::integrations::IntegrationStatus::Available
        ) && !matrix_implemented
        {
            drifts.push(IntegrationCapabilityDrift {
                name: entry.name.to_string(),
                kind: "unsupported_status_claim".to_string(),
                status: format!("{status:?}"),
                expectation: "listed as implemented in matrix".to_string(),
            });
        }
    }

    if drifts.is_empty() {
        Ok(())
    } else {
        drifts.sort_by(|a, b| a.name.cmp(&b.name).then(a.kind.cmp(&b.kind)));
        Err(drifts)
    }
}

fn normalize_capability_matrix_entries(
    mut entries: Vec<IntegrationCapabilityMatrixEntry>,
) -> Vec<IntegrationCapabilityMatrixEntry> {
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.implemented.cmp(&right.implemented).reverse())
    });
    entries.dedup_by_key(|entry| entry.name.clone());
    entries
}

pub fn build_scope_lock_inventory() -> Result<ScopeLockInventory> {
    build_scope_lock_inventory_from_sources(REGISTRY_SOURCE)
}

pub fn load_scope_lock_baseline_inventory() -> Result<ScopeLockInventory> {
    serde_json::from_str(SCOPE_LOCK_BASELINE)
        .map_err(|error| anyhow!("invalid scope-lock baseline artifact: {error}"))
}

pub fn normalize_unimplemented_inventory() -> Result<Vec<InventoryTodoEntry>> {
    normalize_unimplemented_inventory_from_sources(REGISTRY_SOURCE)
}

pub fn normalize_unimplemented_inventory_from_sources(
    registry_source: &str,
) -> Result<Vec<InventoryTodoEntry>> {
    Ok(normalize_and_deduplicate(
        parse_registry_coming_soon_entries(registry_source)?,
    ))
}

pub fn build_scope_lock_inventory_from_sources(
    registry_source: &str,
) -> Result<ScopeLockInventory> {
    Ok(ScopeLockInventory {
        coming_soon_count: parse_registry_coming_soon_count(registry_source)?,
    })
}

pub fn validate_inventory_against_sources(
    expected: &ScopeLockInventory,
    registry_source: &str,
) -> std::result::Result<(), String> {
    let actual = build_scope_lock_inventory_from_sources(registry_source)
        .map_err(|error| format!("failed to parse inventory sources: {error}"))?;

    if expected.coming_soon_count == actual.coming_soon_count {
        Ok(())
    } else {
        Err(format!(
            "{} mismatch: expected={}, actual={}",
            INTEGRATION_HEADER, expected.coming_soon_count, actual.coming_soon_count
        ))
    }
}

pub fn parse_registry_coming_soon_count(source: &str) -> Result<usize> {
    let scope = source
        .split("#[cfg(test)]")
        .next()
        .ok_or_else(|| anyhow!("registry source is empty"))?;

    Ok(scope.matches("status::coming_soon").count())
}

pub fn parse_registry_coming_soon_entries(source: &str) -> Result<Vec<InventoryTodoEntry>> {
    let scope = source
        .split("#[cfg(test)]")
        .next()
        .ok_or_else(|| anyhow!("registry source is empty"))?;

    let mut entries = Vec::new();
    let mut in_entry = false;
    let mut helper_entry = false;
    let mut name = None;
    let mut category = None;
    let mut is_coming_soon = false;

    for line in scope.lines() {
        let trimmed = line.trim();

        if trimmed == "IntegrationEntry {" || trimmed == "integration(" {
            in_entry = true;
            helper_entry = trimmed == "integration(";
            name = None;
            category = None;
            is_coming_soon = false;
            continue;
        }

        if !in_entry {
            continue;
        }

        if let Some(parsed_name) = parse_integration_field_string(trimmed, "name:") {
            name = Some(parsed_name.to_string());
        }
        if helper_entry
            && name.is_none()
            && let Some(parsed_name) = parse_helper_integration_name(trimmed)
        {
            name = Some(parsed_name.to_string());
        }

        if let Some(parsed_category) = parse_integration_category(trimmed)? {
            category = Some(parsed_category);
        }
        if helper_entry
            && category.is_none()
            && let Some(parsed_category) = parse_helper_integration_category(trimmed)?
        {
            category = Some(parsed_category);
        }

        if trimmed.contains("status::coming_soon") {
            is_coming_soon = true;
        }

        if trimmed == "}," || (helper_entry && trimmed == "),") {
            if is_coming_soon {
                if let (Some(name), Some(category)) = (name.take(), category.take()) {
                    entries.push(InventoryTodoEntry {
                        source: INTEGRATION_SOURCE.to_string(),
                        category,
                        status: "ComingSoon".to_string(),
                        priority: INTEGRATION_PRIORITY,
                        name,
                    });
                } else {
                    return Err(anyhow!(
                        "missing integration name/category in registry parser"
                    ));
                }
            }

            in_entry = false;
            helper_entry = false;
        }
    }

    Ok(entries)
}

fn parse_integration_field_string<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    if !line.starts_with(key) {
        return None;
    }

    let start = line.find('"')?;
    let end = line[start + 1..].find('"')?;
    Some(&line[start + 1..start + 1 + end])
}

fn parse_integration_category(line: &str) -> Result<Option<String>> {
    if !line.starts_with("category:") {
        return Ok(None);
    }

    let Some(raw_category) = line.split("IntegrationCategory::").nth(1) else {
        return Ok(None);
    };

    let variant = raw_category
        .split([',', ' ', '\t', '{', '}'])
        .next()
        .unwrap_or_default();

    Ok(Some(match variant {
        "Chat" => "Chat Providers".to_string(),
        "AiModel" => "AI Models".to_string(),
        "Productivity" => "Productivity".to_string(),
        "SmartHome" => "Smart Home".to_string(),
        "ToolsAutomation" => "Tools & Automation".to_string(),
        "MediaCreative" => "Media & Creative".to_string(),
        "Social" => "Social".to_string(),
        "Platform" => "Platforms".to_string(),
        _ => return Err(anyhow!("unknown integration category '{variant}'")),
    }))
}

fn parse_helper_integration_name(line: &str) -> Option<&str> {
    if !line.starts_with('"') {
        return None;
    }

    let end = line[1..].find('"')?;
    Some(&line[1..=end])
}

fn parse_helper_integration_category(line: &str) -> Result<Option<String>> {
    if !line.contains("IntegrationCategory::") {
        return Ok(None);
    }

    parse_integration_category(&format!("category: {line}"))
}

fn normalize_and_deduplicate(mut entries: Vec<InventoryTodoEntry>) -> Vec<InventoryTodoEntry> {
    entries.sort_by(|a, b| {
        (
            a.category.as_str(),
            a.name.as_str(),
            a.source.as_str(),
            a.status.as_str(),
            a.priority,
        )
            .cmp(&(
                b.category.as_str(),
                b.name.as_str(),
                b.source.as_str(),
                b.status.as_str(),
                b.priority,
            ))
    });

    entries.dedup();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChannelsConfig, Config};
    use crate::plugins::integrations::registry;

    #[test]
    fn parse_registry_coming_soon_count_matches_baseline() {
        let count = parse_registry_coming_soon_count(REGISTRY_SOURCE).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn validate_inventory_against_sources_detects_mismatches() {
        let expected = ScopeLockInventory {
            coming_soon_count: 2,
        };

        let error = validate_inventory_against_sources(&expected, REGISTRY_SOURCE)
            .expect_err("mismatch should be detected");

        assert!(error.contains("coming_soon_count mismatch"));
    }

    #[test]
    fn inventory_json_output_is_stable() {
        let first = build_scope_lock_inventory().expect("baseline inventory should build");
        let second = build_scope_lock_inventory().expect("baseline inventory should build");

        assert_eq!(first.to_json_pretty(), second.to_json_pretty());
    }

    #[test]
    fn parse_registry_coming_soon_count_matches_symbol_scan() {
        let scope = REGISTRY_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("registry source should have cfg(test) marker");
        let symbol_count = count_symbol_occurrences(scope, "status::coming_soon");
        let parsed_count = parse_registry_coming_soon_count(REGISTRY_SOURCE).unwrap();

        assert_eq!(parsed_count, symbol_count);
    }

    fn count_symbol_occurrences(text: &str, symbol: &str) -> usize {
        text.matches(symbol).count()
    }

    #[test]
    fn sort_is_stable_by_category_then_name() {
        let normalized = vec![
            InventoryTodoEntry {
                source: "integrations".to_string(),
                category: "Social".to_string(),
                status: "ComingSoon".to_string(),
                priority: 1,
                name: "Slack".to_string(),
            },
            InventoryTodoEntry {
                source: "integrations".to_string(),
                category: "AI Models".to_string(),
                status: "ComingSoon".to_string(),
                priority: 1,
                name: "Claude".to_string(),
            },
            InventoryTodoEntry {
                source: "integrations".to_string(),
                category: "AI Models".to_string(),
                status: "ComingSoon".to_string(),
                priority: 1,
                name: "Claude".to_string(),
            },
        ];

        let normalized = normalize_and_deduplicate(normalized);

        assert_eq!(
            normalized,
            vec![
                InventoryTodoEntry {
                    source: "integrations".to_string(),
                    category: "AI Models".to_string(),
                    status: "ComingSoon".to_string(),
                    priority: 1,
                    name: "Claude".to_string(),
                },
                InventoryTodoEntry {
                    source: "integrations".to_string(),
                    category: "Social".to_string(),
                    status: "ComingSoon".to_string(),
                    priority: 1,
                    name: "Slack".to_string(),
                },
            ]
        );
    }

    #[test]
    fn parse_integration_capability_matrix_matches_registry_projection() {
        let matrix = load_integration_capability_matrix().expect("matrix artifact should parse");
        let entries = registry::all_integrations();
        let config = Config::default();

        let validation = validate_integration_status_against_matrix(&matrix, &entries, &config);
        assert!(
            validation.is_ok(),
            "baseline status matrix should match registry status projection: {validation:?}"
        );
    }

    #[test]
    fn registry_projection_rejects_unbacked_active_or_available() {
        let mut matrix =
            load_integration_capability_matrix().expect("matrix artifact should parse");
        matrix.entries.retain(|entry| entry.name != "Shell");

        let entries = registry::all_integrations();
        let config = Config {
            channels_config: ChannelsConfig::default(),
            ..Config::default()
        };

        let result = validate_integration_status_against_matrix(&matrix, &entries, &config)
            .expect_err("matrix missing implemented entry should fail");

        let has_shell = result.iter().any(|item| item.name == "Shell");
        let has_status = result
            .iter()
            .any(|item| item.kind == "unsupported_status_claim");

        assert!(
            has_shell,
            "missing implemented integration should be reported: {result:#?}"
        );
        assert!(
            has_status,
            "missing implemented entry should be reported as unsupported status claim"
        );
    }
}
