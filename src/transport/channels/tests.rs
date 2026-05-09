//! Unit tests for channel health classification and timeout detection.
use std::time::Duration;

use super::health::{ChannelHealthState, classify_health_result};

#[test]
fn classify_health_ok_true() {
    let state = classify_health_result(&Ok(true));
    assert_eq!(state, ChannelHealthState::Healthy);
}

#[test]
fn classify_health_ok_false() {
    let state = classify_health_result(&Ok(false));
    assert_eq!(state, ChannelHealthState::Unhealthy);
}

#[tokio::test]
async fn classify_health_timeout() {
    let result = tokio::time::timeout(Duration::from_millis(1), async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        true
    })
    .await;
    let state = classify_health_result(&result);
    assert_eq!(state, ChannelHealthState::Timeout);
}
