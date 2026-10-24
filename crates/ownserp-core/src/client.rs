use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use reqwest::Response;
use reqwest::header::CONTENT_TYPE;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::config::Settings;
use crate::cost::{Endpoint, Host};
use crate::error::{Error, Result};

const MAX_UPSTREAM_BYTES: usize = 4 * 1024 * 1024;
const MAX_ATTEMPTS: u32 = 3;
const BASE_BACKOFF_MS: u64 = 250;
const MAX_ERROR_MESSAGE_CHARS: usize = 300;
const CONNECT_TIMEOUT_SECS: u64 = 5;

#[derive(Clone)]
pub struct SerperClient {
    http: reqwest::Client,
    api_key: Option<SecretString>,
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    base_override: Option<Arc<str>>,
    attach_key: bool,
}

impl SerperClient {
    pub fn new(settings: &Settings) -> Result<Self> {
        install_crypto_provider();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(settings.request_timeout_secs()))
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("ownserp/{}", crate::VERSION))
            .build()
            .map_err(|error| Error::network("building the HTTP client", error))?;
        let quota = Quota::per_second(settings.qps().get());
        Ok(Self {
            http,
            api_key: settings.api_key().cloned(),
            limiter: Arc::new(RateLimiter::direct(quota)),
            base_override: None,
            attach_key: true,
        })
    }

    pub fn with_base_url(settings: &Settings, base_url: &str) -> Result<Self> {
        let parsed =
            Url::parse(base_url).map_err(|error| Error::Config(format!("base URL: {error}")))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(Error::Config("the base URL must use http or https".into()));
        }
        if parsed.scheme() != "https" && !is_loopback_url(&parsed) {
            return Err(Error::Config(
                "the base URL must use https unless it is a loopback test server".into(),
            ));
        }
        let mut client = Self::new(settings)?;
        client.attach_key = key_allowed_for(&parsed);
        client.base_override = Some(Arc::from(base_url.trim_end_matches('/')));
        Ok(client)
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    fn base_for(&self, host: Host) -> String {
        match &self.base_override {
            Some(base) => base.to_string(),
            None => host.base_url().to_owned(),
        }
    }

    pub async fn post(
        &self,
        endpoint: Endpoint,
        body: &Value,
        cancellation: &CancellationToken,
    ) -> Result<Value> {
        self.post_many(endpoint, body, 1, cancellation).await
    }

    pub async fn post_many(
        &self,
        endpoint: Endpoint,
        body: &Value,
        permits: usize,
        cancellation: &CancellationToken,
    ) -> Result<Value> {
        let url = format!("{}{}", self.base_for(endpoint.host()), endpoint.path());
        let bytes = serde_json::to_vec(body).map_err(|error| Error::Json {
            context: "encoding the request body",
            source: error,
        })?;
        self.send_with_retry(permits, cancellation, || {
            let mut request = self
                .http
                .post(&url)
                .header(CONTENT_TYPE, "application/json")
                .body(bytes.clone());
            if self.attach_key
                && let Some(api_key) = &self.api_key
            {
                request = request.header("X-API-KEY", api_key.expose_secret());
            }
            request
        })
        .await
    }

    pub async fn get(
        &self,
        endpoint: Endpoint,
        query: &[(&str, String)],
        cancellation: &CancellationToken,
    ) -> Result<Value> {
        let raw = format!("{}{}", self.base_for(endpoint.host()), endpoint.path());
        let mut url = Url::parse(&raw)?;
        {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        let requires_key = endpoint.requires_key();
        self.send_with_retry(1, cancellation, || {
            let mut request = self.http.get(url.clone());
            if requires_key
                && self.attach_key
                && let Some(api_key) = &self.api_key
            {
                request = request.header("X-API-KEY", api_key.expose_secret());
            }
            request
        })
        .await
    }

    pub async fn health(&self) -> Option<Value> {
        let url = format!("{}/health", self.base_for(Host::Api));
        wait_for_permit(&self.limiter).await?;
        let response = self
            .http
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let (_, bytes) = read_capped(response).await.ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn account_probe(&self) -> Option<(u16, Option<u64>)> {
        let api_key = self.api_key.as_ref().filter(|_| self.attach_key)?;
        let url = format!("{}/account", self.base_for(Host::Google));
        wait_for_permit(&self.limiter).await?;
        let response = self
            .http
            .get(&url)
            .header("X-API-KEY", api_key.expose_secret())
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Some((status, None));
        }
        let (_, bytes) = read_capped(response).await.ok()?;
        let body: Value = serde_json::from_slice(&bytes).ok()?;
        let credits = body
            .get("credits")
            .or_else(|| body.get("balance"))
            .and_then(Value::as_u64);
        Some((status, credits))
    }

    pub async fn live_balance(&self) -> Option<u64> {
        self.account_probe().await.and_then(|(_, credits)| credits)
    }

    async fn send_with_retry<F>(
        &self,
        permits: usize,
        cancellation: &CancellationToken,
        build_request: F,
    ) -> Result<Value>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0u32;
        loop {
            attempt = attempt.saturating_add(1);
            match self.acquire_permits(permits, cancellation).await {
                Ok(()) => {}
                Err(_) if attempt == 1 => return Err(Error::CancelledBeforeSend),
                Err(error) => return Err(error),
            }
            let sent = tokio::select! {
                result = build_request().send() => result,
                () = cancellation.cancelled() => return Err(Error::Cancelled),
            };
            match sent {
                Ok(response) => {
                    if retryable_status(response.status().as_u16()) && attempt < MAX_ATTEMPTS {
                        pause(attempt, cancellation).await?;
                        continue;
                    }
                    return tokio::select! {
                        result = read_response(response) => result,
                        () = cancellation.cancelled() => Err(Error::Cancelled),
                    };
                }
                Err(error) => {
                    if attempt < MAX_ATTEMPTS && retryable_error(&error) {
                        pause(attempt, cancellation).await?;
                        continue;
                    }
                    if error.is_connect() {
                        return Err(Error::ConnectFailed {
                            message: "the connection was never established".to_owned(),
                        });
                    }
                    return Err(Error::network("calling Serper", error));
                }
            }
        }
    }

    async fn acquire_permits(
        &self,
        permits: usize,
        cancellation: &CancellationToken,
    ) -> Result<()> {
        for _ in 0..permits.max(1) {
            self.acquire(cancellation).await?;
        }
        Ok(())
    }

    async fn acquire(&self, cancellation: &CancellationToken) -> Result<()> {
        tokio::select! {
            () = self.limiter.until_ready() => Ok(()),
            () = cancellation.cancelled() => Err(Error::Cancelled),
        }
    }
}

fn key_allowed_for(url: &Url) -> bool {
    is_loopback_url(url) || url.host_str().is_some_and(is_serper_host)
}

fn is_loopback_url(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

fn is_serper_host(host: &str) -> bool {
    matches!(
        host,
        "google.serper.dev" | "scrape.serper.dev" | "api.serper.dev"
    )
}

fn retryable_status(status: u16) -> bool {
    (500..600).contains(&status) && status != 501
}

fn retryable_error(error: &reqwest::Error) -> bool {
    // A timeout can follow a request the upstream already billed, so only a failed connect is retried.
    error.is_connect()
}

async fn wait_for_permit(
    limiter: &RateLimiter<NotKeyed, InMemoryState, DefaultClock>,
) -> Option<()> {
    tokio::time::timeout(
        Duration::from_secs(CONNECT_TIMEOUT_SECS),
        limiter.until_ready(),
    )
    .await
    .ok()
}

fn backoff(attempt: u32) -> Duration {
    let step = BASE_BACKOFF_MS.saturating_mul(1u64 << attempt.saturating_sub(1).min(4));
    Duration::from_millis(step.saturating_add(jitter_ms()))
}

async fn pause(attempt: u32, cancellation: &CancellationToken) -> Result<()> {
    tokio::select! {
        () = tokio::time::sleep(backoff(attempt)) => Ok(()),
        () = cancellation.cancelled() => Err(Error::Cancelled),
    }
}

fn jitter_ms() -> u64 {
    let mut bytes = [0u8; 2];
    match getrandom::fill(&mut bytes) {
        Ok(()) => u64::from(u16::from_ne_bytes(bytes)) % 250,
        Err(_) => 0,
    }
}

async fn read_response(response: Response) -> Result<Value> {
    let (code, bytes) = read_capped(response).await?;
    if (200..300).contains(&code) {
        return serde_json::from_slice::<Value>(&bytes).map_err(|_| Error::Upstream {
            status: code,
            message: format!(
                "upstream returned {code} with a body of {} bytes that is not JSON",
                bytes.len()
            ),
        });
    }
    let body = serde_json::from_slice::<Value>(&bytes).unwrap_or_else(|_| json!({}));
    let raw = body
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| status_text(code));
    let (message, _) =
        crate::projection::cap(&crate::projection::sanitize(raw), MAX_ERROR_MESSAGE_CHARS);
    let exhausted = code == 400 && message.to_lowercase().contains("not enough credits");
    if code == 429 || exhausted {
        return Err(Error::RateLimited { message });
    }
    let message = if (300..400).contains(&code) {
        format!("{message} (a redirect, which is not followed)")
    } else {
        message.to_owned()
    };
    Err(Error::Upstream {
        status: code,
        message,
    })
}

async fn read_capped(mut response: Response) -> Result<(u16, Vec<u8>)> {
    let status = response.status().as_u16();
    if let Some(length) = response.content_length()
        && length > MAX_UPSTREAM_BYTES as u64
    {
        return Err(Error::Upstream {
            status,
            message: format!(
                "upstream announced {length} bytes, above the {MAX_UPSTREAM_BYTES} byte cap"
            ),
        });
    }
    let mut buffer = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| Error::network("reading the Serper response", error))?
    {
        if buffer.len().saturating_add(chunk.len()) > MAX_UPSTREAM_BYTES {
            return Err(Error::Upstream {
                status,
                message: format!("upstream body passed the {MAX_UPSTREAM_BYTES} byte cap"),
            });
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok((status, buffer))
}

fn status_text(code: u16) -> &'static str {
    match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "upstream failure",
    }
}

fn install_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        let dir = std::env::temp_dir().join(format!("ownserp-client-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let profile = dir.join("profile.toml");
        std::fs::write(&profile, "").expect("profile");
        let map = std::collections::BTreeMap::from([(
            "SERPER_API_KEY".to_owned(),
            "test-key".to_owned(),
        )]);
        Settings::load_with(
            crate::config::Overrides {
                profile: Some(profile),
                ledger_dir: Some(dir.join("ledger")),
                cache_ttl_secs: Some(0),
                ..crate::config::Overrides::default()
            },
            |name| map.get(name).cloned(),
        )
        .expect("settings")
    }

    #[test]
    fn a_base_override_must_be_an_http_url() {
        let settings = settings();
        assert!(SerperClient::with_base_url(&settings, "ftp://example.org").is_err());
        assert!(SerperClient::with_base_url(&settings, "not a url").is_err());
        assert!(SerperClient::with_base_url(&settings, "http://example.org").is_err());
        assert!(SerperClient::with_base_url(&settings, "https://example.org").is_ok());
        assert!(SerperClient::with_base_url(&settings, "http://127.0.0.1:9").is_ok());
    }

    #[test]
    fn only_a_serper_or_loopback_host_receives_the_key() {
        let settings = settings();
        let local = SerperClient::with_base_url(&settings, "http://127.0.0.1:9").expect("client");
        assert!(
            local.attach_key,
            "a local mock is inside the trust boundary"
        );
        let foreign =
            SerperClient::with_base_url(&settings, "https://example.org").expect("client");
        assert!(!foreign.attach_key, "a foreign host must not get the key");
        let serper =
            SerperClient::with_base_url(&settings, "https://google.serper.dev").expect("client");
        assert!(serper.attach_key);
        assert!(SerperClient::new(&settings).expect("client").attach_key);
    }

    #[test]
    fn only_transient_statuses_are_retried() {
        assert!(retryable_status(502));
        assert!(retryable_status(503));
        assert!(retryable_status(504));
        assert!(retryable_status(500));
        assert!(!retryable_status(501));
        assert!(!retryable_status(429));
        assert!(!retryable_status(401));
        assert!(!retryable_status(200));
    }

    #[test]
    fn backoff_grows_and_stays_bounded() {
        let first = backoff(1).as_millis();
        let third = backoff(3).as_millis();
        assert!(first >= 250);
        assert!(third >= 1000);
        assert!(backoff(10).as_millis() < 5_000);
    }

    #[tokio::test]
    async fn a_cancelled_backoff_returns_at_once() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let started = std::time::Instant::now();
        let outcome = pause(5, &cancellation).await;
        assert!(matches!(outcome, Err(Error::Cancelled)));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
