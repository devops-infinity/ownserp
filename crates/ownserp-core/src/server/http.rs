use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{
    HeaderValue, StatusCode,
    header::{AUTHORIZATION, RETRY_AFTER},
};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Router, middleware};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use rmcp::transport::StreamableHttpServerConfig;
use rmcp::transport::StreamableHttpService;
use rmcp::transport::streamable_http_server::session::never::NeverSessionManager;
use secrecy::ExposeSecret;
use serde_json::json;
use tracing::Instrument;

use super::{Metrics, OwnserpHandler, shutdown_signal};
use crate::config::Settings;
use crate::error::{Error, Result};

const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_REQUEST_ID: usize = 128;
const MIN_TOKEN_CHARS: usize = 32;

type HttpLimiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

#[derive(Clone)]
struct HttpState {
    tokens: Arc<Vec<secrecy::SecretString>>,
    metrics: Arc<Metrics>,
    limiter: Option<Arc<HttpLimiter>>,
    request_seq: Arc<AtomicU64>,
    ready: bool,
    ready_message: Arc<str>,
    allowed_hosts: Arc<Vec<String>>,
}

pub async fn serve_http(settings: Settings) -> Result<()> {
    let tokens = settings.bearer_tokens().to_vec();
    let bind = settings.bind().to_owned();
    if tokens.is_empty() && !is_loopback(&bind) {
        return Err(Error::Config(
            "OWN_SERPER_BEARER_TOKENS is required when bind is not a loopback address".into(),
        ));
    }
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
    let limiter = settings.http_qps().filter(|rate| *rate > 0).map(|rate| {
        let per_second = NonZeroU32::new(rate.max(1)).unwrap_or(NonZeroU32::MIN);
        Arc::new(RateLimiter::direct(Quota::per_second(per_second)))
    });
    let (ready, ready_message): (bool, Arc<str>) = if settings.api_key().is_some() {
        (true, Arc::from("ready"))
    } else {
        (
            false,
            Arc::from("no Serper API key; search tools will refuse until one is set"),
        )
    };
    let allowed_hosts = allowed_hosts(&bind);
    let handler = OwnserpHandler::new(settings)?;
    let metrics = handler.metrics();
    let mut config = StreamableHttpServerConfig::default();
    config.json_response = true;
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
        request_seq: Arc::new(AtomicU64::new(0)),
        ready,
        ready_message,
        allowed_hosts: Arc::new(allowed_hosts.clone()),
    };
    let protected = Router::new()
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .fallback_service(service)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            limit_requests,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ))
        .layer(middleware::from_fn_with_state(state.clone(), require_host))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            assign_request_id,
        ))
        .with_state(state);
    let app = Router::new()
        .route("/healthz", get(healthz))
        .merge(protected)
        .layer(middleware::from_fn(security_headers));
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .map_err(|error| Error::Config(format!("bind {bind}: {error}")))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| Error::Config(format!("http server: {error}")))
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
    let Some(limiter) = state.limiter.as_ref() else {
        return next.run(request).await;
    };
    if limiter.check().is_err() {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(RETRY_AFTER, "1")],
            "rate limited",
        )
            .into_response();
    }
    next.run(request).await
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
    let host = request
        .headers()
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok());
    let accepted = host.is_some_and(|host| {
        let bare = host.split(':').next().unwrap_or(host);
        state.allowed_hosts.iter().any(|allowed| {
            let allowed_bare = allowed.split(':').next().unwrap_or(allowed);
            host == allowed || bare == allowed_bare
        })
    });
    if accepted {
        next.run(request).await
    } else {
        (StatusCode::FORBIDDEN, "host not allowed").into_response()
    }
}

async fn require_bearer(State(state): State<HttpState>, request: Request, next: Next) -> Response {
    if state.tokens.is_empty() {
        return next.run(request).await;
    }
    let presented = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let accepted = presented.is_some_and(|token| {
        state
            .tokens
            .iter()
            .any(|expected| constant_time_eq(token.as_bytes(), expected.expose_secret().as_bytes()))
    });
    if accepted {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0)
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
    let host = host_of(bind);
    matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1" | "[::1]")
}

fn allowed_hosts(bind: &str) -> Vec<String> {
    if let Ok(raw) = std::env::var("OWN_SERPER_ALLOWED_HOSTS") {
        let hosts: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|host| !host.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        if !hosts.is_empty() {
            return hosts;
        }
    }
    let host = host_of(bind);
    vec![host.clone(), bind.to_owned(), format!("{host}:443")]
}

fn host_of(bind: &str) -> String {
    if let Some(rest) = bind.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest).to_owned();
    }
    bind.split(':').next().unwrap_or(bind).to_owned()
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
            request_seq: Arc::new(AtomicU64::new(0)),
            ready: true,
            ready_message: Arc::from("ready"),
            allowed_hosts: Arc::new(vec!["localhost".to_owned()]),
        }
    }

    fn app(state: HttpState) -> Router {
        Router::new()
            .route("/probe", get(|| async { "ok" }))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                require_bearer,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                assign_request_id,
            ))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                limit_requests,
            ))
            .with_state(state)
    }

    fn probe() -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri("/probe")
            .header(axum::http::header::HOST, "localhost")
            .body(Body::empty())
            .expect("request")
    }

    #[tokio::test]
    async fn the_bearer_gate_rejects_then_accepts() {
        let router = app(state(&["secret"], None));
        let denied = router.clone().oneshot(probe()).await.expect("response");
        assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);

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
}
