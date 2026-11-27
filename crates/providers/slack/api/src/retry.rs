//! Retry policy: jittered exponential backoff, at most 3 retries, `Retry-After` wins.

use std::time::Duration;

use backon::{BackoffBuilder, ExponentialBuilder};

use crate::ApiError;

pub(crate) const MAX_RETRIES: usize = 3;

/// Rate limits, server errors and transport failures are transient.
/// Auth errors, configuration errors and other Slack errors are final.
pub(crate) fn is_retryable(e: &ApiError) -> bool {
    match e {
        ApiError::RateLimited { .. } | ApiError::Transport(_) => true,
        ApiError::Http { status } => *status >= 500,
        _ => false,
    }
}

pub(crate) fn backoff(base: Duration) -> impl backon::Backoff {
    ExponentialBuilder::default()
        .with_min_delay(base)
        .with_jitter()
        .with_max_times(MAX_RETRIES)
        .build()
}

/// A `Retry-After` from the server replaces the computed delay.
pub(crate) fn delay_for(e: &ApiError, computed: Option<Duration>) -> Option<Duration> {
    // `None` means the retries are used up; never extend them.
    let computed = computed?;
    match e {
        ApiError::RateLimited { retry_after } => Some(*retry_after),
        _ => Some(computed),
    }
}

/// The longest wait a server can ask for; a larger `Retry-After` is clamped to it.
pub(crate) const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

/// Parse a `Retry-After` header: delta-seconds or an HTTP date. Missing or unreadable
/// means 1s; the result never exceeds [`MAX_RETRY_AFTER`].
pub(crate) fn parse_retry_after(value: Option<&str>) -> Duration {
    let Some(v) = value.map(str::trim) else {
        return Duration::from_secs(1);
    };
    let wait = match v.parse::<u64>() {
        Ok(secs) => Duration::from_secs(secs),
        Err(_) => match httpdate::parse_http_date(v) {
            Ok(at) => at
                .duration_since(std::time::SystemTime::now())
                .unwrap_or(Duration::ZERO),
            Err(_) => Duration::from_secs(1),
        },
    };
    wait.min(MAX_RETRY_AFTER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_are_parsed_and_capped() {
        assert_eq!(parse_retry_after(Some("3")), Duration::from_secs(3));
        assert_eq!(parse_retry_after(Some("0")), Duration::ZERO);
        assert_eq!(parse_retry_after(Some("86400")), MAX_RETRY_AFTER);
    }

    #[test]
    fn missing_or_garbage_is_one_second() {
        assert_eq!(parse_retry_after(None), Duration::from_secs(1));
        assert_eq!(parse_retry_after(Some("soon")), Duration::from_secs(1));
    }

    #[test]
    fn http_date_is_a_delay_from_now() {
        let at = std::time::SystemTime::now() + Duration::from_secs(30);
        let d = parse_retry_after(Some(&httpdate::fmt_http_date(at)));
        assert!(
            d > Duration::from_secs(25) && d <= Duration::from_secs(30),
            "{d:?}"
        );
        let past = std::time::SystemTime::now() - Duration::from_secs(30);
        assert_eq!(
            parse_retry_after(Some(&httpdate::fmt_http_date(past))),
            Duration::ZERO
        );
        let far = std::time::SystemTime::now() + Duration::from_secs(86_400);
        assert_eq!(
            parse_retry_after(Some(&httpdate::fmt_http_date(far))),
            MAX_RETRY_AFTER
        );
    }
}
