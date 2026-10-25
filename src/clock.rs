//! Wall-clock time for the UI. Debug screenshots fix it (`--now <unix-ms>`), so the same keys
//! always produce the same image.
use chrono::TimeZone;
use std::sync::atomic::{AtomicI64, Ordering};

static FIXED: AtomicI64 = AtomicI64::new(0);

pub fn fix(unix_ms: i64) {
    FIXED.store(unix_ms, Ordering::Relaxed);
}

/// Milliseconds since the Unix epoch.
pub fn utc_ms() -> i64 {
    match FIXED.load(Ordering::Relaxed) {
        0 => chrono::Utc::now().timestamp_millis(),
        ms => ms,
    }
}

pub fn local() -> chrono::DateTime<chrono::Local> {
    match FIXED.load(Ordering::Relaxed) {
        0 => chrono::Local::now(),
        ms => chrono::Local.timestamp_millis_opt(ms).single().unwrap_or_else(chrono::Local::now),
    }
}
