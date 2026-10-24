use std::error::Error as StdError;

use serde_json::{Value, json};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("missing Serper API key")]
    MissingApiKey,
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("upstream returned {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("upstream rate limit or exhausted credits: {message}")]
    RateLimited { message: String },
    #[error("budget refused: {0}")]
    BudgetRefused(String),
    #[error("network failure while {context}")]
    Network {
        context: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    #[error("configuration: {0}")]
    Config(String),
    #[error("keychain failure while {context}")]
    Keychain {
        context: &'static str,
        #[source]
        source: Box<dyn StdError + Send + Sync + 'static>,
    },
    #[error("ledger: {0}")]
    Ledger(String),
    #[error("input or output failure while {context}")]
    Io {
        context: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("json failure while {context}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("result shaping failed: {0}")]
    Projection(String),
    #[error("tool {name} is not available in this configuration")]
    UnknownTool { name: String },
    #[error("request cancelled by the client")]
    Cancelled,
}

impl Error {
    pub fn network(context: &'static str, source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Network {
            context,
            source: Box::new(source),
        }
    }

    pub fn keychain(context: &'static str, source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Keychain {
            context,
            source: Box::new(source),
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::MissingApiKey => "serper.auth_missing",
            Self::InvalidArgs(_) => "serper.bad_request",
            Self::Upstream { status, .. } => match status {
                401 | 403 => "serper.auth_failed",
                429 => "serper.rate_limited",
                _ => "serper.upstream_failed",
            },
            Self::RateLimited { .. } => "serper.rate_limited",
            Self::BudgetRefused(_) => "serper.budget_refused",
            Self::Network { .. } => "serper.upstream_failed",
            Self::Config(_) => "serper.config_error",
            Self::Keychain { .. } => "serper.keychain_error",
            Self::Ledger(_) | Self::Io { .. } | Self::Json { .. } | Self::Projection(_) => {
                "serper.internal_error"
            }
            Self::UnknownTool { .. } => "serper.tool_not_found",
            Self::Cancelled => "serper.cancelled",
        }
    }

    pub const fn remedy(&self) -> &'static str {
        match self {
            Self::MissingApiKey => {
                "Set SERPER_API_KEY in the client's env block, or run `ownserp config set-api-key`."
            }
            Self::InvalidArgs(_) => {
                "Fix the argument named in the message and call the tool again."
            }
            Self::Upstream { status, .. } => match status {
                401 | 403 => {
                    "Check the API key at https://serper.dev/api-keys and set a valid one."
                }
                429 => {
                    "Wait for the rate window to pass, or check the credit balance on the dashboard."
                }
                _ => "Retry later; if it repeats, check https://serper.betteruptime.com.",
            },
            Self::RateLimited { .. } => {
                "Lower the call rate or wait; Serper publishes no retry-after header."
            }
            Self::BudgetRefused(_) => {
                "Raise the per-call ceiling or the session budget in settings."
            }
            Self::Network { .. } => "Check network reachability, then retry.",
            Self::Config(_) => "Run `ownserp doctor` to see every resolved setting and its source.",
            Self::Keychain { .. } => {
                "Use SERPER_API_KEY instead, or start a session bus with a keyring service."
            }
            Self::Ledger(_) | Self::Io { .. } | Self::Json { .. } | Self::Projection(_) => {
                "Check the ledger directory permissions and free space."
            }
            Self::UnknownTool { .. } => {
                "Call tools/list and use only the names it returns for this configuration."
            }
            Self::Cancelled => "Call the tool again if the work is still needed.",
        }
    }

    pub fn to_json(&self) -> Value {
        let message =
            crate::projection::cap(&crate::projection::sanitize(&self.to_string()), 500).0;
        json!({
            "code": self.code(),
            "message": message,
            "remedy": self.remedy(),
        })
    }

    pub fn causes(&self) -> Vec<String> {
        let mut chain = Vec::new();
        let mut current: Option<&(dyn StdError + 'static)> = self.source();
        while let Some(error) = current {
            chain.push(redact_url_queries(&error.to_string()));
            current = error.source();
        }
        chain
    }
}

impl From<std::io::Error> for Error {
    fn from(source: std::io::Error) -> Self {
        Self::Io {
            context: "file access",
            source,
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(source: serde_json::Error) -> Self {
        Self::Json {
            context: "serializing protocol data",
            source,
        }
    }
}

impl From<url::ParseError> for Error {
    fn from(source: url::ParseError) -> Self {
        Self::InvalidArgs(format!("url: {source}"))
    }
}

fn redact_url_queries(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(query) = rest.find('?') {
        if !rest.get(..query).is_some_and(|head| head.contains("://")) {
            result.push_str(rest.get(..=query).unwrap_or_default());
            rest = rest.get(query.saturating_add(1)..).unwrap_or_default();
            continue;
        }
        result.push_str(rest.get(..query).unwrap_or_default());
        result.push_str("?<redacted>");
        let tail = rest.get(query.saturating_add(1)..).unwrap_or_default();
        let end = tail
            .find(|ch: char| ch.is_whitespace() || ch == ')')
            .unwrap_or(tail.len());
        rest = tail.get(end..).unwrap_or_default();
    }
    result.push_str(rest);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_error_text_redacts_a_url_query() {
        let text = redact_url_queries(
            "error sending request for url (https://api.serper.dev/locations?q=dhaka&limit=3)",
        );
        assert!(text.contains("?<redacted>)"));
        assert!(!text.contains("dhaka"));
    }

    #[test]
    fn a_wrapped_error_keeps_its_cause_off_the_display_line() {
        let error = Error::network(
            "calling Serper",
            std::io::Error::other("connection reset by peer"),
        );
        assert_eq!(error.to_string(), "network failure while calling Serper");
        assert_eq!(error.causes(), vec!["connection reset by peer".to_owned()]);
        assert_eq!(error.code(), "serper.upstream_failed");
    }

    #[test]
    fn json_and_io_errors_report_a_chain() {
        let json = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        let error = Error::from(json);
        assert_eq!(
            error.to_string(),
            "json failure while serializing protocol data"
        );
        assert!(!error.causes().is_empty());
        assert_eq!(error.code(), "serper.internal_error");
    }

    #[test]
    fn every_code_and_remedy_is_present() {
        let error = Error::BudgetRefused("10 credits".to_owned());
        let body = error.to_json();
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some("serper.budget_refused")
        );
        assert!(body.get("remedy").and_then(Value::as_str).is_some());
    }
}
