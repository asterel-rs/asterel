//! Generic recall-and-deserialize helper for memory backends.

use anyhow::Result;
use serde::de::DeserializeOwned;

use super::{Memory, RecallQuery};

/// Recall items by slot prefix, deserialize each value as `T`.
///
/// Items whose slot key does not start with `slot_prefix` are skipped. Values
/// that match the prefix but fail to deserialize are skipped with a warning so
/// typed-memory schema drift is operator-visible in logs.
///
/// # Errors
///
/// Propagates errors from `recall_scoped`.
pub(crate) async fn recall_typed<T: DeserializeOwned>(
    mem: &dyn Memory,
    entity_id: &str,
    slot_prefix: &str,
    limit: usize,
) -> Result<Vec<T>> {
    let query = RecallQuery::new(entity_id, slot_prefix, limit);
    let items = mem.recall_scoped(query).await.unwrap_or_else(|error| {
        tracing::warn!(%error, slot_prefix, "recall_scoped failed, returning empty");
        Vec::new()
    });
    let mut typed = Vec::new();
    for item in items {
        if !item.slot_key.as_str().starts_with(slot_prefix) {
            continue;
        }
        match serde_json::from_str::<T>(&item.value) {
            Ok(value) => typed.push(value),
            Err(error) => tracing::warn!(
                %error,
                entity_id = %item.entity_id,
                slot_key = %item.slot_key,
                slot_prefix,
                "typed memory recall skipped unreadable JSON value"
            ),
        }
    }
    Ok(typed)
}
