use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::{
    HeaderValue, StatusCode,
    header::{AUTHORIZATION, RETRY_AFTER},
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::serve::{Listener, ListenerExt};
use axum::{Router, middleware};
use governor::clock::DefaultClock;
use governor::state::keyed::DefaultKeyedStateStore;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use secrecy::ExposeSecret;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tower_http::timeout::RequestBodyDeadlineLayer;
use tracing::Instrument;

use super::{Metrics, OwnserpHandler, shutdown_signal};
use crate::config::Settings;
use crate::error::{Error, Result};

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_ID: usize = 128;
const MIN_TOKEN_CHARS: usize = 32;
const MAX_CONNECTIONS: usize = 256;
const MAX_CONCURRENT_REQUESTS: usize = 128;
const BODY_DEADLINE_SECS: u64 = 30;
const SHUTDOWN_GRACE_SECS: u64 = 10;
const FAILED_AUTH_PER_SECOND: u32 = 5;
const CLIENT_ATTEMPTS: u64 = 3;
const CLIENT_BACKOFF_TOTAL_MS: u64 = 1_250;

type HttpLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;
type FailedAuthLimiter = RateLimiter<SocketAddr, DefaultKeyedStateStore<SocketAddr>, DefaultClock>;

#[derive(Clone)]
struct HttpState {
    tokens: Arc<Vec<secrecy::SecretString>>,
    metrics: Arc<Metrics>,
    limiter: Option<Arc<HttpLimiter>>,
    failed_auth: Arc<FailedAuthLimiter>,
    gate: Arc<Semaphore>,
    request_seq: Arc<AtomicU64>,
    ready: bool,
    ready_message: Arc<str>,
    allowed_hosts: Arc<Vec<String>>,
    request_deadline: Duration,
}

pub async fn serve_http(settings: Settings) -> Result<()> {
    let tokens = settings.bearer_tokens().to_vec();
    let tokens_configured = !tokens.is_empty();
    let bind = settings.bind().to_owned();
    if !tokens_configured && !is_loopback(&bind) {
        return Err(Error::Config(
            "OWN_SERPER_BEARER_TOKENS is required when bind is not a loopback address".into(),
        ));
    }
    validate_tokens(&tokens)?;
    let limiter = settings.http_qps().filter(|rate| *rate > 0).map(|rate| {
        let per_second = NonZeroU32::new(rate.max(1)).unwrap_or(NonZeroU32::MIN);
        Arc::new(RateLimiter::direct(Quota::per_second(per_second)))
    });
    let failed_auth = Arc::new(RateLimiter::keyed(Quota::per_second(
        NonZeroU32::new(FAILED_AUTH_PER_SECOND).unwrap_or(NonZeroU32::MIN),
    )));
    let (ready, ready_message): (bool, Arc<str>) = if settings.api_key().is_some() {
        (true, Arc::from("ready"))
    } else {
        (
            false,
            Arc::from("no Serper API key; search tools will refuse until one is set"),
        )
    };
    let allowed_hosts = allowed_hosts(&bind, settings.allowed_hosts());
    let request_deadline =
        call_deadline(settings.request_timeout_secs(), settings.qps().get().get());
    let handler = OwnserpHandler::with_deadline(settings, Some(request_deadline))?;
    let cancel = handler.cancellation_token();
    let metrics = handler.metrics();
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
    config.legacy_session_mode = false;
    config = config.enforce_origin_validation();
    config.max_request_body_bytes = MAX_BODY_BYTES;
    config.allowed_hosts = allowed_hosts.clone();
    let service = StreamableHttpService::new(
        move || Ok(handler.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    let state = HttpState {
        tokens: Arc::new(tokens),
        metrics,
        limiter,
        failed_auth,
        gate: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        request_seq: Arc::new(AtomicU64::new(0)),
        ready,
        ready_message,
        allowed_hosts: Arc::new(allowed_hosts.clone()),
        request_deadline,
    };
    let protected = Router::new()
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .fallback_service(service)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), gate_requests))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_requests,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), require_host))
        .layer(middleware::from_fn(reject_origin))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            assign_request_id,
        ))
        .layer(RequestBodyDeadlineLayer::new(
            std::time::Duration::from_secs(BODY_DEADLINE_SECS),
        ))
        .with_state(state);
    let app = Router::new()
        .route("/healthz", get(healthz))
        .merge(protected)
        .layer(middleware::from_fn(security_headers));
    let listener = TcpListener::bind(&bind).await.map_err(|error| Error::Io {
        context: "binding the HTTP listener",
        source: error,
    })?;
    if !tokens_configured
        && !listener
            .local_addr()
            .map(|address| address.ip().is_loopback())
            .unwrap_or(false)
    {
        return Err(Error::Config(format!(
            "{bind} resolved to a non-loopback address; set OWN_SERPER_BEARER_TOKENS before serving beyond this machine"
        )));
    }
    if !tokens_configured {
        tracing::warn!(
            bind = %bind,
            "serving HTTP without a bearer token; anything that can reach this port, including a tunnel or proxy, can spend credits"
        );
    }
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let listener = LimitedListener {
        inner: listener,
        connections: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
    }
    .tap_io(|_| {});
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        let _ = stop_rx.changed().await;
    });
    let mut handle = tokio::spawn(async move { server.await });
    tokio::select! {
        result = &mut handle => http_result(result),
        () = shutdown_signal() => {
            cancel.cancel();
            let _ = stop_tx.send(true);
            match tokio::time::timeout(
                std::time::Duration::from_secs(SHUTDOWN_GRACE_SECS),
                &mut handle,
            )
            .await
            {
                Ok(result) => http_result(result),
                Err(_) => {
                    tracing::warn!(
                        grace_secs = SHUTDOWN_GRACE_SECS,
                        "shutdown grace period elapsed with requests still open"
                    );
                    handle.abort();
                    let _ = handle.await;
                    Ok(())
                }
            }
        }
    }
}

fn http_result(
    result: std::result::Result<std::io::Result<()>, tokio::task::JoinError>,
) -> Result<()> {
    match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(Error::Io {
            context: "serving HTTP",
            source: error,
        }),
        Err(error) => Err(Error::Io {
            context: "waiting for the HTTP server task",
            source: std::io::Error::other(error),
        }),
    }
}

// One permit per socket bounds file descriptors and idle connections.
struct LimitedListener {
    inner: TcpListener,
    connections: Arc<Semaphore>,
}

struct LimitedStream {
    inner: TcpStream,
    _permit: OwnedSemaphorePermit,
}

impl Listener for LimitedListener {
    type Io = LimitedStream;
    type Addr = std::net::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let permit = match Arc::clone(&self.connections).acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
            };
            match self.inner.accept().await {
                Ok((stream, address)) => {
                    return (
                        LimitedStream {
                            inner: stream,
                            _permit: permit,
                        },
                        address,
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.inner.local_addr()
    }
}

impl AsyncRead for LimitedStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(context, buffer)
    }
}

impl AsyncWrite for LimitedStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(context, buffers)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

async fn healthz() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn readyz(State(state): State<HttpState>) -> impl IntoResponse {
    if state.ready {
        (StatusCode::OK, "ready").into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            state.ready_message.to_string(),
        )
            .into_response()
    }
}

async fn metrics_handler(State(state): State<HttpState>) -> impl IntoResponse {
    let (calls, errors, inflight) = state.metrics.snapshot();
    (
        StatusCode::OK,
        Json(json!({
            "calls": calls,
            "errors": errors,
            "inflight": inflight,
            "byTool": state.metrics.per_tool(),
        })),
    )
}

async fn limit_requests(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    if request.uri().path() == "/readyz" {
        return next.run(request).await;
    }
    if let Some(limiter) = state.limiter.as_ref()
        && limiter.check().is_err()
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, "1")],
            "rate limited",
        )
            .into_response();
    }
    next.run(request).await
}

async fn reject_origin(request: Request, next: Next) -> Response {
    if request.headers().contains_key(axum::http::header::ORIGIN) {
        return (StatusCode::FORBIDDEN, "origin not allowed").into_response();
    }
    next.run(request).await
}

async fn gate_requests(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    let Ok(_permit) = state.gate.clone().try_acquire_owned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(RETRY_AFTER, "1")],
            "server busy",
        )
            .into_response();
    };
    let backstop = state
        .request_deadline
        .saturating_add(Duration::from_secs(SHUTDOWN_GRACE_SECS));
    match tokio::time::timeout(backstop, next.run(request)).await {
        Ok(response) => response,
        Err(_) => (StatusCode::GATEWAY_TIMEOUT, "request deadline passed").into_response(),
    }
}

async fn assign_request_id(
    State(state): State<HttpState>,
    request: Request,
    next: Next,
) -> Response {
    let incoming = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| valid_request_id(value));
    let id = incoming.map_or_else(
        || {
            format!(
                "{:x}-{:x}",
                unix_nanos(),
                state.request_seq.fetch_add(1, Ordering::Relaxed)
            )
        },
        str::to_owned,
    );
    let span = tracing::info_span!(
        "http_request",
        request_id = %id,
        method = %request.method(),
        path = %request.uri().path(),
    );
    let mut response = next.run(request).instrument(span.clone()).await;
    span.in_scope(|| {
        tracing::debug!(status = %response.status().as_u16(), "http request completed");
    });
    if let Ok(value) = HeaderValue::from_str(&id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response
}

async fn require_host(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    let host = match request.headers().get(axum::http::header::HOST) {
        Some(value) => value.to_str().ok().and_then(parse_host),
        None => request
            .uri()
            .authority()
            .and_then(|authority| parse_host(authority.as_str())),
    };
    let accepted = host.is_some_and(|host| {
        state
            .allowed_hosts
            .iter()
            .filter_map(|allowed| parse_host(allowed))
            .any(|allowed| host_matches(&host, &allowed))
    });
    if accepted {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "host not allowed").into_response()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostIdentity {
    host: String,
    port: Option<u16>,
}

fn parse_host(value: &str) -> Option<HostIdentity> {
    let authority: axum::http::uri::Authority = value.parse().ok()?;
    if authority.as_str().contains('@') {
        return None;
    }
    let raw = authority.host();
    let host = raw
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(raw);
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(HostIdentity {
        host,
        port: authority.port_u16(),
    })
}

fn host_matches(request: &HostIdentity, allowed: &HostIdentity) -> bool {
    request.host == allowed.host
        && match allowed.port {
            Some(port) => request.port == Some(port),
            None => true,
        }
}

async fn require_bearer(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    if state.tokens.is_empty() {
        return next.run(request).await;
    }
    let accepted = bearer_token(&request).is_some_and(|token| {
        state
            .tokens
            .iter()
            .any(|expected| token_matches(token, expected.expose_secret()))
    });
    if accepted {
        return next.run(request).await;
    }
    if let Some(peer) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0)
        && state.failed_auth.check_key(&peer).is_err()
    {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, "1")],
            "rate limited",
        )
            .into_response();
    }
    (
        StatusCode::UNAUTHORIZED,
        [(axum::http::header::WWW_AUTHENTICATE, "Bearer")],
        "unauthorized",
    )
        .into_response()
}

fn bearer_token(request: &Request) -> Option<&str> {
    let value = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

fn validate_tokens(tokens: &[secrecy::SecretString]) -> Result<()> {
    if let Some((index, _)) = tokens
        .iter()
        .enumerate()
        .find(|(_, token)| token.expose_secret().chars().count() < MIN_TOKEN_CHARS)
    {
        return Err(Error::Config(format!(
            "bearer token {} is shorter than {MIN_TOKEN_CHARS} characters; use a random value from a password manager",
            index + 1
        )));
    }
    if let Some((index, _)) = tokens.iter().enumerate().find(|(_, token)| {
        !token
            .expose_secret()
            .chars()
            .all(|character| ('\u{21}'..='\u{7e}').contains(&character))
    }) {
        return Err(Error::Config(format!(
            "bearer token {} has a character outside the printable ASCII range 0x21 to 0x7E; a header value cannot carry it",
            index + 1
        )));
    }
    Ok(())
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0)
}

fn token_matches(presented: &str, expected: &str) -> bool {
    let left = Sha256::digest(presented.as_bytes());
    let right = Sha256::digest(expected.as_bytes());
    constant_time_eq(&left, &right)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        difference |= a ^ b;
    }
    difference == 0
}

fn is_loopback(bind: &str) -> bool {
    let host = host_from_bind(bind);
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

fn allowed_hosts(bind: &str, configured: Option<&[String]>) -> Vec<String> {
    if let Some(hosts) = configured
        && !hosts.is_empty()
    {
        return hosts.to_vec();
    }
    let host = host_from_bind(bind);
    if unspecified_bind(bind) {
        tracing::warn!(
            bind = %bind,
            "the bind address is unspecified and no allowed hosts are configured; requests will be rejected as host not allowed until OWN_SERPER_ALLOWED_HOSTS names the Host a client sends"
        );
    }
    let authority_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.clone()
    };
    let mut hosts = vec![
        authority_host.clone(),
        bind.to_owned(),
        format!("{authority_host}:443"),
    ];
    if is_loopback(bind) {
        for alias in ["localhost", "127.0.0.1", "[::1]"] {
            if !hosts.iter().any(|entry| entry == alias) {
                hosts.push(alias.to_owned());
            }
        }
    }
    hosts
}

fn host_from_bind(bind: &str) -> String {
    if let Some(rest) = bind.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest).to_owned();
    }
    bind.split(':').next().unwrap_or(bind).to_owned()
}

fn unspecified_bind(bind: &str) -> bool {
    host_from_bind(bind)
        .parse::<std::net::IpAddr>()
        .is_ok_and(|address| address.is_unspecified())
}

fn call_deadline(request_timeout_secs: u64, qps: u32) -> Duration {
    // The client makes three attempts with 250 ms and 500 ms backoff, and each attempt waits one limiter slot.
    let limiter_ms = 1000u64
        .saturating_mul(CLIENT_ATTEMPTS)
        .checked_div(u64::from(qps.max(1)))
        .unwrap_or(0);
    Duration::from_millis(
        request_timeout_secs
            .saturating_mul(CLIENT_ATTEMPTS)
            .saturating_mul(1000)
            .saturating_add(CLIENT_BACKOFF_TOTAL_MS)
            .saturating_add(limiter_ms),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request as HttpRequest;
    use tower::ServiceExt;

    fn state(tokens: &[&str], qps: Option<u32>) -> HttpState {
        let limiter = qps.filter(|rate| *rate > 0).map(|rate| {
            let per_second = NonZeroU32::new(rate).unwrap_or(NonZeroU32::MIN);
            Arc::new(RateLimiter::direct(Quota::per_second(per_second)))
        });
        HttpState {
            tokens: Arc::new(
                tokens
                    .iter()
                    .map(|token| secrecy::SecretString::from(*token))
                    .collect(),
            ),
            metrics: Arc::new(Metrics::default()),
            limiter,
            failed_auth: Arc::new(RateLimiter::keyed(Quota::per_second(
                NonZeroU32::new(FAILED_AUTH_PER_SECOND).unwrap_or(NonZeroU32::MIN),
            ))),
            gate: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            request_seq: Arc::new(AtomicU64::new(0)),
            ready: true,
            ready_message: Arc::from("ready"),
            allowed_hosts: Arc::new(vec!["localhost".to_owned()]),
            request_deadline: Duration::from_secs(30),
        }
    }

    fn app(state: HttpState) -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .route("/readyz", get(|| async { "ready" }))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                limit_requests,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                assign_request_id,
            ))
            .with_state(state)
    }

    fn host_app(state: HttpState) -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(state.clone(), require_host))
            .with_state(state)
    }

    fn probe() -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .body(Body::empty())
            .expect("request")
    }

    fn peer(mut request: HttpRequest<Body>, address: SocketAddr) -> HttpRequest<Body> {
        request.extensions_mut().insert(ConnectInfo(address));
        request
    }

    #[tokio::test]
    async fn the_bearer_gate_rejects_then_accepts() {
        let router = app(state(&["secret"], None));
        let denied = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            denied
                .headers()
                .get(axum::http::header::WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some("Bearer")
        );

        let request = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .header(AUTHORIZATION, "Bearer secret")
            .body(Body::empty())
            .expect("request");
        let allowed = router.oneshot(request).await.expect("response");
        assert_eq!(allowed.status(), StatusCode::OK);
        assert!(allowed.headers().contains_key("x-request-id"));
    }

    #[tokio::test]
    async fn an_open_router_needs_no_token() {
        let router = app(state(&[], None));
        let response = router.oneshot(probe()).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_request_id_is_echoed_when_valid() {
        let router = app(state(&[], None));
        let request = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .header("x-request-id", "client-42")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(
            response
                .headers()
                .get("x-request-id")
                .and_then(|value| value.to_str().ok()),
            Some("client-42")
        );
    }

    #[tokio::test]
    async fn the_limiter_answers_429_with_retry_after() {
        let router = app(state(&[], Some(1)));
        let first = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        let second = router.oneshot(probe()).await.expect("response");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            second
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );
    }

    #[tokio::test]
    async fn an_unauthenticated_caller_does_not_spend_the_global_rate_limit() {
        let router = app(state(&["secret"], Some(1)));
        let first = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(first.status(), StatusCode::UNAUTHORIZED);
        let second = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(
            second.status(),
            StatusCode::UNAUTHORIZED,
            "a failed token must not spend the token holder's rate limit"
        );
        let authorized = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .header(AUTHORIZATION, "Bearer secret")
            .body(Body::empty())
            .expect("request");
        let allowed = router.clone().oneshot(authorized).await.expect("response");
        assert_eq!(allowed.status(), StatusCode::OK);
        let limited = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .header(AUTHORIZATION, "Bearer secret")
            .body(Body::empty())
            .expect("request");
        let refused = router.oneshot(limited).await.expect("response");
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn the_readiness_probe_is_not_rate_limited() {
        let router = app(state(&[], Some(1)));
        for _ in 0..3 {
            let request = HttpRequest::builder()
                .uri("/readyz")
                .header(axum::http::header::HOST, "localhost")
                .body(Body::empty())
                .expect("request");
            let response = router.clone().oneshot(request).await.expect("response");
            assert_eq!(response.status(), StatusCode::OK);
        }
        let first = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        let second = router.oneshot(probe()).await.expect("response");
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn failed_tokens_are_limited_per_peer() {
        let router = app(state(&["secret"], None));
        let one = SocketAddr::from(([127, 0, 0, 1], 4000));
        let other = SocketAddr::from(([127, 0, 0, 2], 4000));
        for _ in 0..FAILED_AUTH_PER_SECOND {
            let response = router
                .clone()
                .oneshot(peer(probe(), one))
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let refused = router
            .clone()
            .oneshot(peer(probe(), one))
            .await
            .expect("response");
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        let allowed_other_peer = router
            .oneshot(peer(probe(), other))
            .await
            .expect("response");
        assert_eq!(allowed_other_peer.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_bearer_scheme_is_case_insensitive() {
        let router = app(state(&["secret"], None));
        for scheme in ["bearer", "BEARER", "BeArEr"] {
            let request = HttpRequest::builder()
                .uri("/probe")
                .header(axum::http::header::HOST, "localhost")
                .header(AUTHORIZATION, format!("{scheme} secret"))
                .body(Body::empty())
                .expect("request");
            let response = router.clone().oneshot(request).await.expect("response");
            assert_eq!(response.status(), StatusCode::OK, "scheme {scheme}");
        }
    }

    #[tokio::test]
    async fn an_origin_header_is_rejected() {
        let router = Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(middleware::from_fn(reject_origin));
        let request = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .header(axum::http::header::ORIGIN, "https://example.org")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_http2_style_authority_counts_as_the_host() {
        let mut state = state(&[], None);
        state.allowed_hosts = Arc::new(vec!["localhost".to_owned()]);
        let router = host_app(state);
        let request = HttpRequest::builder()
            .uri("http://localhost/probe")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn bearer_comparison_matches_only_equal_bytes() {
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokeN"));
        assert!(!constant_time_eq(b"token", b"tokens"));
    }

    #[test]
    fn request_id_validation_accepts_the_common_shape() {
        assert!(valid_request_id("d1f2-3a4b"));
        assert!(valid_request_id("req.123_ABC"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("has space"));
        assert!(!valid_request_id(&"a".repeat(MAX_REQUEST_ID + 1)));
    }

    #[test]
    fn a_host_entry_with_a_port_must_match_the_port() {
        let allowed = parse_host("example.com:443").expect("allowed");
        assert!(host_matches(
            &parse_host("example.com:443").expect("host"),
            &allowed
        ));
        assert!(!host_matches(
            &parse_host("example.com:8080").expect("host"),
            &allowed
        ));
    }

    #[test]
    fn host_matching_ignores_case_and_one_trailing_dot() {
        let allowed = parse_host("example.com").expect("allowed");
        assert!(host_matches(
            &parse_host("Example.COM.").expect("host"),
            &allowed
        ));
    }

    #[test]
    fn the_host_parser_keeps_ipv6_and_rejects_a_bare_colon() {
        assert_eq!(
            parse_host("[::1]:8777").expect("ipv6").host,
            "::1".to_owned()
        );
        assert_eq!(parse_host("127.0.0.1:8777").expect("ipv4").port, Some(8777));
        assert!(parse_host("::1").is_none());
        assert!(parse_host(":443").is_none());
        assert!(parse_host("has space").is_none());
    }

    #[tokio::test]
    async fn a_host_entry_with_a_port_rejects_another_port() {
        let mut state = state(&[], None);
        state.allowed_hosts = Arc::new(vec!["example.com:443".to_owned()]);
        let router = host_app(state);
        let request = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "example.com:8080")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_invalid_host_header_is_rejected_instead_of_using_the_uri() {
        let mut state = state(&[], None);
        state.allowed_hosts = Arc::new(vec!["localhost".to_owned()]);
        let router = host_app(state);
        let request = HttpRequest::builder()
            .uri("http://localhost/probe")
            .header(axum::http::header::HOST, "bad host")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn a_loopback_bind_accepts_every_loopback_spelling() {
        let hosts = allowed_hosts("127.0.0.1:8777", None);
        assert!(hosts.iter().any(|host| host == "localhost"));
        assert!(hosts.iter().any(|host| host == "127.0.0.1"));
        assert!(hosts.iter().any(|host| host == "[::1]"));
    }

    #[test]
    fn an_ipv6_bind_formats_its_entries_with_brackets() {
        let hosts = allowed_hosts("[::1]:8777", None);
        assert!(hosts.iter().any(|host| host == "[::1]:443"));
        assert!(!hosts.iter().any(|host| host == "::1:443"));
    }

    #[test]
    fn an_unspecified_bind_is_recognized() {
        assert!(unspecified_bind("0.0.0.0:8777"));
        assert!(unspecified_bind("[::]:8777"));
        assert!(!unspecified_bind("127.0.0.1:8777"));
        assert!(!unspecified_bind("localhost:8777"));
    }

    #[test]
    fn a_non_ascii_bearer_token_is_refused_at_startup() {
        let token = secrecy::SecretString::from("é".repeat(MIN_TOKEN_CHARS));
        let error = validate_tokens(&[token]).expect_err("must fail");
        assert!(error.to_string().contains("0x21"));
    }

    #[test]
    fn a_short_bearer_token_is_refused_at_startup() {
        let token = secrecy::SecretString::from("short");
        let error = validate_tokens(&[token]).expect_err("must fail");
        assert!(error.to_string().contains("shorter than"));
    }

    #[test]
    fn the_deadline_follows_the_configured_timeout_and_the_retries() {
        assert_eq!(
            call_deadline(10, 1),
            Duration::from_millis(10 * 3 * 1000 + CLIENT_BACKOFF_TOTAL_MS + 3 * 1000)
        );
    }

    #[test]
    fn a_slow_request_timeout_is_not_cut_at_the_old_120_second_deadline() {
        assert!(call_deadline(300, 10) > Duration::from_secs(120));
    }

    #[test]
    fn a_transport_failure_is_an_io_error_not_a_config_error() {
        let error = http_result(Ok(Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "address already in use",
        ))))
        .expect_err("must fail");
        assert_eq!(error.code(), "serper.internal_error");
    }

    #[tokio::test]
    async fn a_bare_colon_host_does_not_pass_the_ipv6_allowlist() {
        let mut state = state(&[], None);
        state.allowed_hosts = Arc::new(vec!["::1".to_owned()]);
        let router = host_app(state);
        let request = HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, ":443")
            .body(Body::empty())
            .expect("request");
        let response = router.oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
