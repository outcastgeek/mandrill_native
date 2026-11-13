//! Resource freshness classification.
//!
//! The classification keeps the source's local-date comparison and its seven-day and
//! thirty-day boundaries. A created instant is interpreted in the supplied `now`
//! offset, exactly as the source converts into the current location before comparing
//! dates and durations.

use chrono::{DateTime, Duration, FixedOffset, Local, TimeZone, Utc};

/// Classify the freshness of a created instant against the current local time.
#[must_use]
pub fn resource_freshness_kind(created_at: i64) -> String {
    resource_freshness_kind_at(created_at, &Local::now().fixed_offset())
}

/// Classify the freshness of a created instant against a supplied instant.
#[must_use]
pub fn resource_freshness_kind_at(created_at: i64, now: &DateTime<FixedOffset>) -> String {
    let created = Utc
        .timestamp_opt(created_at, 0)
        .single()
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
        .with_timezone(now.offset());
    if now.date_naive() == created.date_naive() {
        return "today".to_owned();
    }
    let elapsed = now.signed_duration_since(created);
    if elapsed < Duration::days(7) {
        return "week".to_owned();
    }
    if elapsed < Duration::days(30) {
        return "month".to_owned();
    }
    String::new()
}
