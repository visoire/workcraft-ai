//! Shared HTTP behaviour of all adapters: retries with backoff before a stream starts, a stall guard
//! while it runs, and error messages a person can act on.
//!
//! A request is only retried while nothing has been streamed back yet, so a retry can never repeat text
//! or a tool call the person already saw.

use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc::Sender;
use workcraft_core::{Delta, Error, Result};

/// Attempts per request, the first one included.
pub const MAX_ATTEMPTS: u32 = 4;
/// Longest wait between two attempts, also the cap for a server's `Retry-After`.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// How long a hosted model may go silent in the middle of an answer.
pub const CLOUD_IDLE: Duration = Duration::from_secs(120);
/// Local models load, think and print slowly: give them longer before calling it a stall.
pub const LOCAL_IDLE: Duration = Duration::from_secs(300);

/// Whether a failed answer is worth another try.
#[must_use]
pub fn is_retryable(status: u16, body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    // A spent quota or an empty balance does not come back by waiting.
    if status == 429
        && (b.contains("insufficient_quota")
            || b.contains("billing")
            || b.contains("credit")
            || b.contains("balance"))
    {
        return false;
    }
    matches!(status, 408 | 425 | 429 | 500 | 502 | 503 | 504 | 529)
}

/// Delay before attempt number `attempt` (1 is the first retry): 1 s, 2 s, 4 s, ... with a little jitter.
#[must_use]
pub fn backoff(attempt: u32) -> Duration {
    let base = 1000u64 << attempt.saturating_sub(1).min(5);
    let jitter = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::from(d.subsec_nanos()) % (base / 4 + 1));
    Duration::from_millis(base + jitter).min(MAX_WAIT)
}

/// `Retry-After` in seconds (the HTTP-date form is not used by model APIs).
#[must_use]
pub fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let secs = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()?;
    if secs.is_finite() && secs >= 0.0 {
        Some(Duration::from_secs_f64(secs).min(MAX_WAIT))
    } else {
        None
    }
}

/// The message inside a provider error body, if it has one.
#[must_use]
pub fn error_detail(body: &str) -> String {
    let from_json = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        v.pointer("/error/message")
            .or_else(|| v.get("message"))
            .or_else(|| v.get("error"))
            .or_else(|| v.pointer("/detail"))
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let text = from_json.unwrap_or_else(|| body.trim().to_string());
    text.chars().take(300).collect()
}

/// A readable message for a failed request: what happened and what to do about it. Keys are never echoed.
#[must_use]
pub fn friendly_status(label: &str, status: u16, body: &str) -> String {
    let detail = error_detail(body);
    let d = detail.to_ascii_lowercase();
    let what = match status {
        400 | 422 if d.contains("reasoning_content") => {
            "the model needs its earlier reasoning sent back with the tool results. Start a new chat, or switch thinking off for this model".to_string()
        }
        400 | 413 | 422
            if d.contains("context length")
                || d.contains("context window")
                || d.contains("maximum context")
                || d.contains("too many tokens")
                || d.contains("prompt is too long")
                || d.contains("too long") =>
        {
            "the conversation is too long for this model's context window. Start a new chat, or pick a model with a larger window".to_string()
        }
        400 | 422 => "the request was rejected".to_string(),
        401 | 403 => "the API key was not accepted. Check it in Settings, Providers".to_string(),
        402 => "no credit left on this account. Add balance with the provider".to_string(),
        404 => "the model or the address was not found. Check the model name and the base URL".to_string(),
        408 | 504 => "the service took too long to answer. Try again".to_string(),
        413 => "the request is too large for this service".to_string(),
        429 if d.contains("insufficient_quota") || d.contains("billing") || d.contains("credit") || d.contains("balance") => {
            "the quota or balance of this account is used up. Check the plan with the provider".to_string()
        }
        429 => "too many requests. Wait a moment, or use another model".to_string(),
        529 => "the service is overloaded right now. Try again in a minute".to_string(),
        500..=599 => "the service is having trouble. Try again soon, or use another provider".to_string(),
        _ => "the service answered with an error".to_string(),
    };
    if detail.is_empty() {
        format!("{label}: {what} (HTTP {status})")
    } else {
        format!("{label}: {what} (HTTP {status}: {detail})")
    }
}

fn reason_for_notice(status: u16) -> &'static str {
    match status {
        429 => "rate limited",
        529 => "overloaded",
        408 | 504 => "timed out",
        _ => "server error",
    }
}

/// Send a request, retrying while the answer is a transient failure. `build` makes a fresh request for each
/// attempt. Retries are announced on `tx` as [`Delta::Notice`]. On success the response is returned unread.
pub async fn send_retrying(
    label: &str,
    tx: &Sender<Delta>,
    build: impl Fn() -> reqwest::RequestBuilder,
) -> Result<reqwest::Response> {
    let mut attempt = 1u32;
    loop {
        match build().send().await {
            Ok(resp) if resp.status().is_success() => return Ok(resp),
            Ok(resp) => {
                let status = resp.status().as_u16();
                let wait_hint = retry_after(resp.headers());
                let body = resp.text().await.unwrap_or_default();
                if attempt < MAX_ATTEMPTS && is_retryable(status, &body) {
                    let wait = wait_hint.unwrap_or_else(|| backoff(attempt));
                    announce(tx, label, reason_for_notice(status), wait, attempt).await?;
                    attempt += 1;
                    continue;
                }
                return Err(Error::Provider(friendly_status(label, status, &body)));
            }
            Err(e) => {
                let transient = e.is_connect() || e.is_timeout() || e.is_request();
                if transient && attempt < MAX_ATTEMPTS {
                    announce(tx, label, "cannot connect", backoff(attempt), attempt).await?;
                    attempt += 1;
                    continue;
                }
                return Err(Error::Http(format!(
                    "{label}: cannot reach the service ({}). Check the internet connection and the base URL",
                    short_error(&e)
                )));
            }
        }
    }
}

fn short_error(e: &reqwest::Error) -> String {
    // reqwest includes the full URL, which can carry a key in a query string: show the cause only.
    let mut msg = String::new();
    let mut src: Option<&(dyn std::error::Error + 'static)> = std::error::Error::source(e);
    while let Some(s) = src {
        msg = s.to_string();
        src = s.source();
    }
    if msg.is_empty() {
        if e.is_timeout() {
            "timed out".to_string()
        } else if e.is_connect() {
            "connection failed".to_string()
        } else {
            "request failed".to_string()
        }
    } else {
        msg.chars().take(160).collect()
    }
}

async fn announce(
    tx: &Sender<Delta>,
    label: &str,
    reason: &str,
    wait: Duration,
    attempt: u32,
) -> Result<()> {
    let note = format!(
        "{label}: {reason}, retrying in {} s (attempt {} of {MAX_ATTEMPTS})",
        wait.as_secs().max(1),
        attempt + 1
    );
    // A closed receiver means the turn was cancelled: stop quietly instead of sleeping.
    if tx.send(Delta::Notice(note)).await.is_err() {
        return Err(Error::Cancelled);
    }
    tokio::time::sleep(wait).await;
    Ok(())
}

/// Next chunk of a response body, or an error if the service goes silent for `idle`.
pub async fn next_chunk<S>(stream: &mut S, idle: Duration, label: &str) -> Result<Option<Bytes>>
where
    S: Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Unpin,
{
    match tokio::time::timeout(idle, stream.next()).await {
        Err(_) => Err(Error::Timeout(format!(
            "{label}: no data for {} s, the answer stopped. Try again",
            idle.as_secs()
        ))),
        Ok(None) => Ok(None),
        Ok(Some(Ok(b))) => Ok(Some(b)),
        Ok(Some(Err(e))) => Err(Error::Http(format!(
            "{label}: the connection broke while the answer was streaming ({})",
            short_error(&e)
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[test]
    fn retry_rules() {
        for s in [408u16, 429, 500, 502, 503, 504, 529] {
            assert!(is_retryable(s, ""), "{s}");
        }
        for s in [400u16, 401, 402, 403, 404, 422] {
            assert!(!is_retryable(s, ""), "{s}");
        }
        assert!(!is_retryable(
            429,
            r#"{"error":{"code":"insufficient_quota"}}"#
        ));
        assert!(!is_retryable(429, "Your credit balance is too low"));
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert!(backoff(1) >= Duration::from_secs(1) && backoff(1) < Duration::from_millis(1400));
        assert!(backoff(2) >= Duration::from_secs(2));
        assert!(backoff(3) >= Duration::from_secs(4));
        assert!(backoff(30) <= MAX_WAIT);
    }

    #[test]
    fn retry_after_is_read_and_capped() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(reqwest::header::RETRY_AFTER, "3".parse().unwrap());
        assert_eq!(retry_after(&h), Some(Duration::from_secs(3)));
        h.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
        assert_eq!(retry_after(&h), Some(MAX_WAIT));
        h.insert(reqwest::header::RETRY_AFTER, "soon".parse().unwrap());
        assert_eq!(retry_after(&h), None);
    }

    #[test]
    fn errors_say_what_to_do() {
        let m = friendly_status(
            "DeepSeek",
            401,
            r#"{"error":{"message":"Authentication Fails"}}"#,
        );
        assert!(
            m.contains("API key was not accepted") && m.contains("Authentication Fails"),
            "{m}"
        );
        assert!(friendly_status("X", 402, "").contains("no credit left"));
        assert!(
            friendly_status("X", 429, r#"{"error":{"code":"insufficient_quota"}}"#)
                .contains("used up")
        );
        assert!(friendly_status("X", 529, "").contains("overloaded"));
        let long = friendly_status(
            "X",
            400,
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#,
        );
        assert!(long.contains("too long for this model"), "{long}");
        let rc = friendly_status(
            "X",
            400,
            "The `reasoning_content` in the thinking mode must be passed back",
        );
        assert!(rc.contains("earlier reasoning"), "{rc}");
        // The body is cut, never dumped whole.
        let huge = friendly_status("X", 500, &"a".repeat(5000));
        assert!(huge.len() < 500);
    }

    /// A one-connection-at-a-time HTTP server that answers with the given raw responses in order.
    async fn serve(responses: Vec<&'static str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for r in responses {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 8192];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(r.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn a_503_is_retried_and_announced_then_succeeds() {
        let url = serve(vec![
            "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
        ])
        .await;
        let http = reqwest::Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let resp = send_retrying("Test", &tx, || http.post(&url))
            .await
            .unwrap();
        assert_eq!(resp.text().await.unwrap(), "ok");
        let note = rx.recv().await.unwrap();
        assert!(
            matches!(&note, Delta::Notice(n) if n.contains("retrying") && n.contains("server error")),
            "{note:?}"
        );
    }

    #[tokio::test]
    async fn a_401_is_not_retried() {
        let url = serve(vec![
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ])
        .await;
        let http = reqwest::Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let err = send_retrying("Test", &tx, || http.post(&url))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("API key was not accepted"),
            "{err}"
        );
        assert!(rx.try_recv().is_err(), "no retry notice");
    }

    #[tokio::test]
    async fn a_dead_port_gives_up_with_a_clear_message() {
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let http = reqwest::Client::new();
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let url = format!("http://127.0.0.1:{port}/x");
        let h = tokio::spawn(async move { send_retrying("Test", &tx, || http.post(&url)).await });
        // Close the receiver after the first notice: the next one finds nobody listening and the call ends.
        let mut seen = 0;
        while let Some(n) = rx.recv().await {
            assert!(matches!(n, Delta::Notice(_)));
            seen += 1;
            if seen == 1 {
                break;
            }
        }
        drop(rx);
        let r = h.await.unwrap();
        assert!(matches!(r, Err(Error::Cancelled)) || r.is_err());
    }

    #[tokio::test]
    async fn a_silent_stream_times_out() {
        let mut s = futures_util::stream::pending::<std::result::Result<Bytes, reqwest::Error>>();
        let err = next_chunk(&mut s, Duration::from_millis(50), "Test")
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Timeout(_)), "{err}");
    }
}
