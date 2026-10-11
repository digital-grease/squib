//! Bounded retry for weather requests (docs/squib/05: "retry-after handling, bounded
//! exponential retry with jitter, and a total refresh deadline. Do not encode hidden
//! infinite retry loops"). Plan phase 3.1.
//!
//! Only transient failures are retried: timeouts, unexplained transport errors, 408, 429,
//! 500, 502, 503 and 504. Being offline, TLS failures, oversized bodies and other statuses
//! are reported at once. Each request gets at most [`MAX_ATTEMPTS`] tries, a retry is
//! scheduled only if it fits the refresh deadline with room for the request itself, and a
//! server's Retry-After is never shortened.

use crate::nws::{HttpResponse, TransportError};

/// Tries per request, including the first.
pub const MAX_ATTEMPTS: u32 = 3;
/// Backoff before the first retry; doubles each time up to [`BACKOFF_CAP_MS`].
pub const BACKOFF_BASE_MS: i64 = 500;
pub const BACKOFF_CAP_MS: i64 = 4_000;
/// Time left in the refresh deadline that a retried request needs after its delay.
pub const RETRY_MARGIN_MS: i64 = 3_000;

/// Whether this response is worth another try.
pub fn retryable(resp: &HttpResponse) -> bool {
    match &resp.error {
        Some(TransportError::Timeout | TransportError::Other(_)) => true,
        Some(TransportError::Offline | TransportError::Tls | TransportError::TooLarge) => false,
        None => matches!(resp.status, 408 | 429 | 500 | 502 | 503 | 504),
    }
}

/// Delay before the next try, or `None` to give up. `attempt` is the try that just
/// failed (1 for the first). `elapsed_ms` is time since the refresh started.
/// `unit_random` in [0, 1) comes from the platform, so the core stays deterministic.
pub fn next_delay_ms(resp: &HttpResponse, attempt: u32, elapsed_ms: i64, deadline_ms: i64, unit_random: f64) -> Option<u32> {
    if !retryable(resp) || attempt >= MAX_ATTEMPTS {
        return None;
    }
    // Exponential backoff with "equal jitter": half fixed, half random, so retries from
    // many phones spread out but never collapse to zero.
    let backoff = (BACKOFF_BASE_MS << (attempt - 1).min(16)).min(BACKOFF_CAP_MS);
    let jitter = (unit_random.clamp(0.0, 1.0) * (backoff / 2) as f64) as i64;
    let mut delay = backoff / 2 + jitter;
    if let Some(s) = resp.retry_after_s {
        delay = delay.max(i64::from(s) * 1000);
    }
    (elapsed_ms + delay + RETRY_MARGIN_MS <= deadline_ms).then_some(delay as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(status: u16, error: Option<TransportError>, retry_after_s: Option<u32>) -> HttpResponse {
        HttpResponse { id: 1, status, body: vec![], etag: None, max_age_s: None, retry_after_s, error, unit_random: 0.5 }
    }

    #[test]
    fn only_transient_failures_are_retried() {
        for s in [408, 429, 500, 502, 503, 504] {
            assert!(retryable(&resp(s, None, None)), "{s}");
        }
        for s in [200, 304, 400, 403, 404, 501] {
            assert!(!retryable(&resp(s, None, None)), "{s}");
        }
        assert!(retryable(&resp(0, Some(TransportError::Timeout), None)));
        assert!(retryable(&resp(0, Some(TransportError::Other("reset".into())), None)));
        for e in [TransportError::Offline, TransportError::Tls, TransportError::TooLarge] {
            assert!(!retryable(&resp(0, Some(e), None)));
        }
    }

    #[test]
    fn backoff_doubles_with_bounded_jitter_and_stops_after_the_last_attempt() {
        let r = resp(503, None, None);
        assert_eq!(next_delay_ms(&r, 1, 0, 20_000, 0.0), Some(250));
        assert_eq!(next_delay_ms(&r, 1, 0, 20_000, 0.999), Some(499));
        assert_eq!(next_delay_ms(&r, 2, 0, 20_000, 0.0), Some(500));
        assert_eq!(next_delay_ms(&r, 2, 0, 20_000, 0.999), Some(999));
        assert_eq!(next_delay_ms(&r, MAX_ATTEMPTS, 0, 20_000, 0.5), None, "bounded");
        assert_eq!(next_delay_ms(&r, 1, 0, 20_000, 7.0), Some(500), "random value clamped");
    }

    #[test]
    fn retry_after_is_respected_and_the_deadline_wins() {
        let r = resp(429, None, Some(5));
        assert_eq!(next_delay_ms(&r, 1, 0, 20_000, 0.0), Some(5_000), "never shorter than Retry-After");
        assert_eq!(next_delay_ms(&r, 1, 12_000, 20_000, 0.0), Some(5_000), "fits exactly");
        assert_eq!(next_delay_ms(&r, 1, 12_001, 20_000, 0.0), None, "would not finish before the deadline");
        assert_eq!(next_delay_ms(&resp(429, None, Some(3_600)), 1, 0, 20_000, 0.0), None);
        // A retry needs room for the request itself after its delay.
        assert_eq!(next_delay_ms(&resp(500, None, None), 1, 16_800, 20_000, 0.0), None);
        assert_eq!(next_delay_ms(&resp(500, None, None), 1, 16_700, 20_000, 0.0), Some(250));
    }
}
