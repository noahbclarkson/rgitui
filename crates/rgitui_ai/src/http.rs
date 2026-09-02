//! Shared HTTP plumbing for every provider call.
//!
//! Three things live here because getting any one of them wrong hangs the app
//! or exhausts memory, and they must not be re-derived per provider:
//!
//! - a **deadline** on every request. `ReqwestClient::new()` sets a connect
//!   timeout only, so a provider that accepts the TCP connection and then
//!   stalls used to leave the spinner running until the app was restarted.
//! - a **cap** on the response body. An arbitrary endpoint — which
//!   `base_url_override` now makes reachable — is exactly the case where an
//!   unbounded `read_to_end` becomes a real OOM.
//! - **retry with backoff** that honours `Retry-After`, so a 429 or a
//!   transient 503 is not a hard failure.

use anyhow::{Context as _, Result};
use futures::AsyncReadExt;
use gpui::http_client::{AsyncBody, Response};
use std::time::Duration;

/// Deadline for a generation request, including its response body. Tool-
/// calling generations issue several of these in sequence, so this is a
/// per-request budget rather than a whole-generation one.
pub(crate) const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Deadline for a catalogue fetch. Shorter than a generation: nothing the user
/// is waiting on blocks behind it, and the cached list is already on screen.
pub(crate) const CATALOG_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Hard cap on a response body. Well beyond any legitimate completion or
/// catalogue (the largest real payload measured is OpenRouter's ~700 KB dump).
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// How many times a retryable status is retried before giving up.
pub(crate) const MAX_RETRIES: u32 = 2;

/// Read a response body, refusing to grow past [`MAX_RESPONSE_BYTES`].
pub(crate) async fn read_response_body(response: &mut Response<AsyncBody>) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .await
        .context("Failed to read the response body")?;
    if body.len() > MAX_RESPONSE_BYTES {
        anyhow::bail!(
            "The response was larger than {} MB and was refused. If you set a custom \
             base URL, check that it points at an OpenAI-compatible API.",
            MAX_RESPONSE_BYTES / (1024 * 1024)
        );
    }
    Ok(body)
}

/// Whether a status is worth retrying: rate limits and transient server
/// errors, never a 4xx the request itself caused.
pub(crate) fn is_retryable(status: u16) -> bool {
    status == 429 || status == 408 || (500..=599).contains(&status)
}

/// How long to wait before retry `attempt` (0-based), honouring the server's
/// `Retry-After` when it sent one.
///
/// Pure so the backoff schedule is testable without sleeping.
pub(crate) fn retry_delay(attempt: u32, retry_after: Option<Duration>) -> Duration {
    let backoff = Duration::from_millis(500u64 << attempt.min(6));
    match retry_after {
        // Cap what a server can ask us to wait: a header asking for ten
        // minutes should surface as a failure the user can act on, not a
        // spinner that appears hung.
        Some(after) => after.min(Duration::from_secs(30)).max(backoff),
        None => backoff,
    }
}

/// Parse a `Retry-After` header. Only the delta-seconds form is honoured; the
/// HTTP-date form is rare in practice and a wrong parse would be worse than
/// falling back to plain backoff.
pub(crate) fn parse_retry_after(value: Option<&str>) -> Option<Duration> {
    let seconds: u64 = value?.trim().parse().ok()?;
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limits_and_server_errors_retry_but_client_errors_do_not() {
        assert!(is_retryable(429));
        assert!(is_retryable(408));
        assert!(is_retryable(503));
        assert!(is_retryable(500));
        assert!(!is_retryable(400));
        assert!(!is_retryable(401));
        assert!(!is_retryable(404));
        assert!(!is_retryable(200));
    }

    #[test]
    fn backoff_grows_exponentially_when_the_server_says_nothing() {
        assert_eq!(retry_delay(0, None), Duration::from_millis(500));
        assert_eq!(retry_delay(1, None), Duration::from_millis(1000));
        assert_eq!(retry_delay(2, None), Duration::from_millis(2000));
    }

    #[test]
    fn retry_after_wins_when_it_asks_for_longer_than_the_backoff() {
        assert_eq!(
            retry_delay(0, Some(Duration::from_secs(5))),
            Duration::from_secs(5)
        );
    }

    #[test]
    fn a_retry_after_shorter_than_the_backoff_does_not_shorten_it() {
        assert_eq!(
            retry_delay(2, Some(Duration::from_millis(100))),
            Duration::from_millis(2000)
        );
    }

    #[test]
    fn an_unreasonable_retry_after_is_capped() {
        assert_eq!(
            retry_delay(0, Some(Duration::from_secs(600))),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn retry_after_parses_only_the_delta_seconds_form() {
        assert_eq!(parse_retry_after(Some("7")), Some(Duration::from_secs(7)));
        assert_eq!(
            parse_retry_after(Some(" 12 ")),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            parse_retry_after(Some("Wed, 21 Oct 2026 07:28:00 GMT")),
            None
        );
        assert_eq!(parse_retry_after(None), None);
    }
}
