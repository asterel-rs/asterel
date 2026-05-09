//! Cron expression parsing and next-run calculation.
//!
//! Normalizes 5-field cron expressions to 7-field format,
//! computes the next occurrence, and parses helper fields
//! like `max_attempts` and RFC 3339 timestamps.

use std::str::FromStr;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use cron::Schedule;

/// Computes the next occurrence of a cron expression after `from`.
///
/// # Errors
///
/// Returns an error if the expression is invalid or has no future
/// occurrence.
pub(crate) fn next_run_for(expression: &str, from: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let normalized = normalize_expression(expression)?;
    let schedule = Schedule::from_str(&normalized)
        .with_context(|| format!("Invalid cron expression: {expression}"))?;
    schedule
        .after(&from)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No future occurrence for expression: {expression}"))
}

fn normalize_expression(expression: &str) -> Result<String> {
    let expression = expression.trim();
    if expression.is_empty() {
        anyhow::bail!("Invalid cron expression: value cannot be empty");
    }

    if let Some(natural) = normalize_natural_expression(expression)? {
        return Ok(natural);
    }

    let field_count = expression.split_whitespace().count();

    match field_count {
        // standard crontab syntax: minute hour day month weekday
        5 => Ok(format!("0 {expression}")),
        // crate-native syntax includes seconds (+ optional year)
        6 | 7 => Ok(expression.to_string()),
        _ => anyhow::bail!(
            "Invalid cron expression: {expression} (expected 5, 6, or 7 fields, got {field_count})"
        ),
    }
}

fn normalize_natural_expression(expression: &str) -> Result<Option<String>> {
    let normalized = expression.trim().to_ascii_lowercase();

    if normalized == "hourly" {
        return Ok(Some("0 0 * * * *".to_string()));
    }
    if normalized == "daily" || normalized == "every day" {
        return Ok(Some("0 0 0 * * *".to_string()));
    }

    if let Some(raw_time) = normalized.strip_prefix("daily at ") {
        let (hour, minute) = parse_hhmm_24(raw_time)?;
        return Ok(Some(format!("0 {minute} {hour} * * *")));
    }

    if let Some(raw_time) = normalized.strip_prefix("every day at ") {
        let (hour, minute) = parse_hhmm_24(raw_time)?;
        return Ok(Some(format!("0 {minute} {hour} * * *")));
    }

    if let Some(raw_time) = normalized.strip_prefix("weekdays at ") {
        let (hour, minute) = parse_hhmm_24(raw_time)?;
        return Ok(Some(format!("0 {minute} {hour} * * MON-FRI")));
    }

    if let Some(rest) = normalized.strip_prefix("every ") {
        let parts = rest.split_whitespace().collect::<Vec<_>>();
        if parts.len() == 2 {
            let interval = parts[0].parse::<u32>().map_err(|_| {
                anyhow::anyhow!("Invalid natural schedule interval in expression: {expression}")
            })?;

            if parts[1] == "minutes" || parts[1] == "minute" {
                if !(1..=59).contains(&interval) {
                    anyhow::bail!(
                        "Invalid minute interval in natural schedule: {interval} (expected 1..=59)"
                    );
                }
                return Ok(Some(format!("0 */{interval} * * * *")));
            }

            if parts[1] == "hours" || parts[1] == "hour" {
                if !(1..=23).contains(&interval) {
                    anyhow::bail!(
                        "Invalid hour interval in natural schedule: {interval} (expected 1..=23)"
                    );
                }
                return Ok(Some(format!("0 0 */{interval} * * *")));
            }
        }
    }

    if let Some(rest) = normalized.strip_prefix("weekly on ")
        && let Some((weekday_raw, time_raw)) = rest.split_once(" at ")
    {
        let weekday = normalize_weekday(weekday_raw)
            .ok_or_else(|| anyhow::anyhow!("Invalid weekday in natural schedule: {weekday_raw}"))?;
        let (hour, minute) = parse_hhmm_24(time_raw)?;
        return Ok(Some(format!("0 {minute} {hour} * * {weekday}")));
    }

    Ok(None)
}

fn parse_hhmm_24(raw: &str) -> Result<(u32, u32)> {
    let value = raw.trim();
    let (hour_raw, minute_raw) = value.split_once(':').ok_or_else(|| {
        anyhow::anyhow!("Invalid time format in natural schedule: {value} (expected HH:MM)")
    })?;

    let hour = hour_raw.parse::<u32>().map_err(|_| {
        anyhow::anyhow!("Invalid hour in natural schedule time: {hour_raw} (expected 0..=23)")
    })?;
    let minute = minute_raw.parse::<u32>().map_err(|_| {
        anyhow::anyhow!("Invalid minute in natural schedule time: {minute_raw} (expected 0..=59)")
    })?;

    if hour > 23 || minute > 59 {
        anyhow::bail!(
            "Invalid time in natural schedule: {value} (expected hour 0..=23 and minute 0..=59)"
        );
    }

    Ok((hour, minute))
}

fn normalize_weekday(raw: &str) -> Option<&'static str> {
    match raw.trim() {
        "monday" | "mon" => Some("MON"),
        "tuesday" | "tue" | "tues" => Some("TUE"),
        "wednesday" | "wed" => Some("WED"),
        "thursday" | "thu" | "thurs" => Some("THU"),
        "friday" | "fri" => Some("FRI"),
        "saturday" | "sat" => Some("SAT"),
        "sunday" | "sun" => Some("SUN"),
        _ => None,
    }
}

/// Parses an RFC 3339 timestamp string into a UTC `DateTime`.
///
/// # Errors
///
/// Returns an error if the string is not valid RFC 3339.
pub(crate) fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("Invalid RFC3339 timestamp in cron DB: {raw}"))?;
    Ok(parsed.with_timezone(&Utc))
}

/// Parses a max-attempts value, clamping non-positive values to 1.
pub(crate) fn parse_max_attempts(raw: i64) -> u32 {
    u32::try_from(raw)
        .ok()
        .filter(|value| *value > 0)
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::normalize_expression;

    #[test]
    fn normalize_expression_supports_natural_minute_interval() {
        assert_eq!(
            normalize_expression("every 5 minutes").expect("natural expression should parse"),
            "0 */5 * * * *"
        );
    }

    #[test]
    fn normalize_expression_supports_daily_time() {
        assert_eq!(
            normalize_expression("daily at 14:30").expect("daily schedule should parse"),
            "0 30 14 * * *"
        );
    }

    #[test]
    fn normalize_expression_supports_hourly_alias() {
        assert_eq!(
            normalize_expression("hourly").expect("hourly schedule should parse"),
            "0 0 * * * *"
        );
    }

    #[test]
    fn normalize_expression_supports_daily_aliases() {
        assert_eq!(
            normalize_expression("daily").expect("daily alias should parse"),
            "0 0 0 * * *"
        );
        assert_eq!(
            normalize_expression("every day").expect("every day alias should parse"),
            "0 0 0 * * *"
        );
    }

    #[test]
    fn normalize_expression_supports_weekdays_time() {
        assert_eq!(
            normalize_expression("weekdays at 09:15").expect("weekday schedule should parse"),
            "0 15 9 * * MON-FRI"
        );
    }

    #[test]
    fn normalize_expression_supports_weekly_day_time() {
        assert_eq!(
            normalize_expression("weekly on monday at 08:00")
                .expect("weekly schedule should parse"),
            "0 0 8 * * MON"
        );
    }

    #[test]
    fn normalize_expression_rejects_invalid_natural_time() {
        let error = normalize_expression("daily at 99:00")
            .expect_err("invalid natural time should fail")
            .to_string();
        assert!(error.contains("Invalid time in natural schedule"));
    }

    #[test]
    fn normalize_expression_rejects_invalid_natural_intervals() {
        let minute_zero = normalize_expression("every 0 minutes")
            .expect_err("zero-minute interval should fail")
            .to_string();
        assert!(minute_zero.contains("expected 1..=59"));

        let minute_high = normalize_expression("every 60 minutes")
            .expect_err("60-minute interval should fail")
            .to_string();
        assert!(minute_high.contains("expected 1..=59"));

        let hour_high = normalize_expression("every 24 hours")
            .expect_err("24-hour interval should fail")
            .to_string();
        assert!(hour_high.contains("expected 1..=23"));
    }

    #[test]
    fn normalize_expression_rejects_invalid_weekday() {
        let error = normalize_expression("weekly on moonday at 08:00")
            .expect_err("invalid weekday should fail")
            .to_string();
        assert!(error.contains("Invalid weekday in natural schedule"));
    }
}
