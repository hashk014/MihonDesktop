//! Shared HTTP stack.
//!
//! One `reqwest` client is reused everywhere (connection pooling matters when a
//! chapter pulls thirty images at once), wrapped in a per-host rate limiter so
//! sources that ask for one are not hammered — the equivalent of Mihon's
//! `RateLimitInterceptor`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, USER_AGENT};
use tokio::time::Instant;

pub const DEFAULT_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
     Chrome/128.0.0.0 Safari/537.36";

/// Sliding-window limiter: at most `permits` requests per `period`.
struct RateLimiter {
    permits: usize,
    period: Duration,
    hits: Mutex<Vec<Instant>>,
}

impl RateLimiter {
    fn new(permits: usize, period: Duration) -> Self {
        Self {
            permits,
            period,
            hits: Mutex::new(Vec::new()),
        }
    }

    async fn acquire(&self) {
        loop {
            let wait = {
                let mut hits = self.hits.lock();
                let now = Instant::now();
                hits.retain(|t| now.duration_since(*t) < self.period);
                if hits.len() < self.permits {
                    hits.push(now);
                    return;
                }
                // Sleep until the oldest hit leaves the window.
                self.period - now.duration_since(hits[0])
            };
            tokio::time::sleep(wait).await;
        }
    }
}

pub struct HttpClient {
    client: reqwest::Client,
    limiters: Mutex<HashMap<String, Arc<RateLimiter>>>,
}

impl HttpClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(DEFAULT_UA)
            .timeout(Duration::from_secs(45))
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(8)
            .cookie_store(true)
            .build()
            .context("building the HTTP client")?;
        Ok(Self {
            client,
            limiters: Mutex::new(HashMap::new()),
        })
    }

    /// Registers a limit for one host. Sources call this at construction time.
    pub fn set_rate_limit(&self, host: &str, permits: usize, period: Duration) {
        self.limiters.insert_limit(host, permits, period);
    }

    fn limiter_for(&self, url: &str) -> Option<Arc<RateLimiter>> {
        let host = host_of(url)?;
        let limiters = self.limiters.lock();
        limiters.get(&host).cloned()
    }

    async fn throttle(&self, url: &str) {
        if let Some(limiter) = self.limiter_for(url) {
            limiter.acquire().await;
        }
    }

    pub async fn get_bytes(&self, url: &str, headers: &[(String, String)]) -> Result<Vec<u8>> {
        self.throttle(url).await;
        let response = self
            .client
            .get(url)
            .headers(build_headers(headers)?)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let status = response.status();
        if !status.is_success() {
            bail!("GET {url} returned HTTP {}", status.as_u16());
        }
        Ok(response.bytes().await?.to_vec())
    }

    pub async fn get_text(&self, url: &str, headers: &[(String, String)]) -> Result<String> {
        self.throttle(url).await;
        let response = self
            .client
            .get(url)
            .headers(build_headers(headers)?)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;

        let status = response.status();
        if !status.is_success() {
            bail!("GET {url} returned HTTP {}", status.as_u16());
        }
        Ok(response.text().await?)
    }

    pub async fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<serde_json::Value> {
        let text = self.get_text(url, headers).await?;
        serde_json::from_str(&text).with_context(|| format!("decoding JSON from {url}"))
    }
}

/// Small extension so `set_rate_limit` reads cleanly at the call site.
trait InsertLimit {
    fn insert_limit(&self, host: &str, permits: usize, period: Duration);
}

impl InsertLimit for Mutex<HashMap<String, Arc<RateLimiter>>> {
    fn insert_limit(&self, host: &str, permits: usize, period: Duration) {
        self.lock().insert(
            host.to_ascii_lowercase(),
            Arc::new(RateLimiter::new(permits, period)),
        );
    }
}

fn build_headers(extra: &[(String, String)]) -> Result<HeaderMap> {
    let mut map = HeaderMap::new();
    map.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_UA));
    for (name, value) in extra {
        let name = HeaderName::from_bytes(name.as_bytes())
            .with_context(|| format!("invalid header name {name:?}"))?;
        let value = HeaderValue::from_str(value)
            .with_context(|| format!("invalid header value {value:?}"))?;
        map.insert(name, value);
    }
    Ok(map)
}

pub fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?;
    let host = host.split(':').next()?;
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Resolves a possibly-relative URL against a base, without pulling in a URL crate.
pub fn absolute_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let base = base.trim_end_matches('/');
    if let Some(rest) = href.strip_prefix("//") {
        let scheme = base.split_once("://").map(|(s, _)| s).unwrap_or("https");
        return format!("{scheme}://{rest}");
    }
    if href.starts_with('/') {
        // Keep only scheme + host from the base.
        let origin = match base.split_once("://") {
            Some((scheme, rest)) => {
                let host = rest.split('/').next().unwrap_or(rest);
                format!("{scheme}://{host}")
            }
            None => base.to_string(),
        };
        return format!("{origin}{href}");
    }
    format!("{base}/{href}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_extraction() {
        assert_eq!(
            host_of("https://api.mangadex.org/manga?x=1").as_deref(),
            Some("api.mangadex.org")
        );
        assert_eq!(
            host_of("http://example.com:8080/a").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("not a url").as_deref(), Some("not a url"));
    }

    #[test]
    fn url_resolution() {
        assert_eq!(absolute_url("https://a.com/x/", "/y"), "https://a.com/y");
        assert_eq!(absolute_url("https://a.com/x", "y"), "https://a.com/x/y");
        assert_eq!(
            absolute_url("https://a.com", "//cdn.b.com/i.png"),
            "https://cdn.b.com/i.png"
        );
        assert_eq!(
            absolute_url("https://a.com", "https://c.com/z"),
            "https://c.com/z"
        );
    }
}
