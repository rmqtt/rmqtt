use std::time::Duration;

use crate::types::{Timestamp, TimestampMillis};

#[inline]
pub fn timestamp() -> Duration {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_else(|_| {
        let now = chrono::Local::now();
        Duration::new(now.timestamp() as u64, now.timestamp_subsec_nanos())
    })
}

#[inline]
pub fn timestamp_secs() -> Timestamp {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_secs() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp())
}

#[inline]
pub fn timestamp_millis() -> TimestampMillis {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|t| t.as_millis() as i64)
        .unwrap_or_else(|_| chrono::Local::now().timestamp_millis())
}

#[inline]
pub fn format_timestamp(t: Timestamp) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_opt(t, 0) {
            t.format("%Y-%m-%d %H:%M:%S").to_string()
        } else {
            "".into()
        }
    }
}

#[inline]
pub fn format_timestamp_millis(t: TimestampMillis) -> String {
    if t <= 0 {
        "".into()
    } else {
        use chrono::TimeZone;
        if let chrono::LocalResult::Single(t) = chrono::Local.timestamp_millis_opt(t) {
            t.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
        } else {
            "".into()
        }
    }
}
