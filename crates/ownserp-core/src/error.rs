use std::error::Error as StdError;

use serde_json::{Value, json};

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("missing Serper API key")]
    MissingApiKey { cause: Option<String> },
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("upstream returned {status}: {message}")]
    Upstream { status: u16, message: String },
    #[error("upstream rate limit: {message}")]
    RateLimited { message: String },
    #[error("Serper credits exhausted: {message}")]
    CreditsExhausted { message: String },
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
    #[error("ledger busy: {0}")]
    LedgerBusy(String),
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
    #[error("tool {name} is disabled until {switch} is set")]
    ToolDisabled { name: String, switch: &'static str },
    #[error("request cancelled")]
    Cancelled,
    #[error("the call passed the server's deadline")]
    DeadlineExceeded,
    #[error("could not reach Serper")]
    ConnectFailed { message: String },
    #[error("{message}")]
    Refused {
        code: &'static str,
        message: String,
        remedy: &'static str,
    },
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
            Self::MissingApiKey { .. } => "serper.auth_missing",
            Self::InvalidArgs(_) => "serper.bad_request",
            Self::Upstream { status, .. } => match status {
                400 | 422 => "serper.bad_request",
                401 | 403 => "serper.auth_failed",
                429 => "serper.rate_limited",
                _ => "serper.upstream_failed",
            },
            Self::RateLimited { .. } => "serper.rate_limited",
            Self::CreditsExhausted { .. } => "serper.credits_exhausted",
            Self::BudgetRefused(_) => "serper.budget_refused",
            Self::Network { .. } => "serper.upstream_failed",
            Self::Config(_) => "serper.config_error",
            Self::Keychain { .. } => "serper.keychain_error",
            Self::Ledger(_) | Self::LedgerBusy(_) => "serper.ledger_fault",
            Self::Io { .. } => "serper.internal_error",
            Self::Json { .. } | Self::Projection(_) => "serper.internal_error",
            Self::UnknownTool { .. } => "serper.tool_not_found",
            Self::ToolDisabled { .. } => "serper.tool_disabled",
            Self::Cancelled => "serper.cancelled",
            Self::DeadlineExceeded => "serper.deadline_exceeded",
            Self::ConnectFailed { .. } => "serper.upstream_failed",
            Self::Refused { code, .. } => code,
        }
    }

    pub fn is_unbilled(&self) -> bool {
        match self {
            Self::Upstream { status, .. } => *status >= 300,
            Self::RateLimited { .. } => true,
            Self::CreditsExhausted { .. } => true,
            Self::ConnectFailed { .. } => true,
            _ => false,
        }
    }

    pub const fn remedy(&self) -> &'static str {
        match self {
            Self::MissingApiKey { cause: None } => {
                "Set SERPER_API_KEY in the server's environment, or run `ownserp config set-api-key`, then restart the server."
            }
            Self::MissingApiKey { cause: Some(_) } => {
                "The OS keychain could not be read. Set SERPER_API_KEY in the server's environment, or start a keyring service and run `ownserp config set-api-key`, then restart the server."
            }
            Self::InvalidArgs(_) => {
                "Fix the argument the message describes and call the tool again."
            }
            Self::Upstream { status, .. } => match status {
                400 | 422 => "Fix the request the message names, then try again.",
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
            Self::CreditsExhausted { .. } => {
                "Top up the account at https://serper.dev/dashboard, then call the tool again."
            }
            Self::BudgetRefused(_) => {
                "Raise the per-call ceiling or the session budget in the profile or the server's environment, then restart the server; or lower num or split the batch to fit the current limit."
            }
            Self::Network { .. } => "Check network reachability, then retry.",
            Self::Config(_) => {
                "Fix the setting the message names in the profile or the server's environment; `ownserp doctor` shows each resolved value and where it came from."
            }
            Self::Keychain { .. } => {
                "Use SERPER_API_KEY instead, or start a session bus with a keyring service."
            }
            Self::LedgerBusy(_) => {
                "Another ownserp process holds the ledger. Stop the older copy, or wait and retry."
            }
            Self::Json { .. } => {
                "The answer could not be read as JSON. Retry, and check https://serper.betteruptime.com if it repeats."
            }
            Self::Projection(_) => {
                "The answer was read but could not be shaped. Report the tool and the arguments."
            }
            Self::Ledger(_) | Self::Io { .. } => {
                "Check the ledger directory permissions and free space, then restart the server; if the chain is damaged, run `ownserp audit rotate --archive-damaged`."
            }
            Self::UnknownTool { .. } => {
                "Call tools/list and use only the names it returns for this configuration."
            }
            Self::ToolDisabled { .. } => {
                "Set the named profile key or environment variable and restart the server."
            }
            Self::Cancelled => "Call the tool again if the work is still needed.",
            Self::DeadlineExceeded => {
                "The call may already have been billed. Retry with a narrower request, or raise request_timeout_secs and restart the server."
            }
            Self::ConnectFailed { .. } => "Check network reachability, then retry.",
            Self::Refused { remedy, .. } => remedy,
        }
    }

    pub fn to_json(&self) -> Value {
        let redacted = redact_paths(&self.to_string());
        let message = crate::projection::cap(&crate::projection::sanitize(&redacted), 500).0;
        let mut body = serde_json::Map::new();
        body.insert("code".to_owned(), json!(self.code()));
        body.insert("message".to_owned(), json!(message));
        body.insert("remedy".to_owned(), json!(self.remedy()));
        if matches!(
            self,
            Self::Network { .. } | Self::MissingApiKey { cause: Some(_) }
        ) {
            let causes: Vec<String> = self
                .causes()
                .into_iter()
                .take(3)
                .map(|cause| crate::projection::cap(&crate::projection::sanitize(&cause), 300).0)
                .collect();
            if !causes.is_empty() {
                body.insert("causes".to_owned(), json!(causes));
            }
        }
        Value::Object(body)
    }

    pub fn causes(&self) -> Vec<String> {
        let mut chain = Vec::new();
        if let Self::MissingApiKey { cause: Some(cause) } = self {
            chain.push(redact_url_queries(cause));
        }
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

fn redact_paths(text: &str) -> String {
    text.split(' ')
        .map(|token| {
            if token.starts_with('/') || token.starts_with("~/") {
                let trimmed = token.trim_end_matches([':', ')', ',', '.']);
                let name = trimmed.rsplit('/').next().unwrap_or(trimmed);
                token.replace(trimmed, name)
            } else {
                token.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_url_queries(text: &str) -> String {
    let mut redacted_text = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(query) = rest.find('?') {
        if !rest.get(..query).is_some_and(|head| head.contains("://")) {
            redacted_text.push_str(rest.get(..=query).unwrap_or_default());
            rest = rest.get(query.saturating_add(1)..).unwrap_or_default();
            continue;
        }
        redacted_text.push_str(rest.get(..query).unwrap_or_default());
        redacted_text.push_str("?<redacted>");
        let tail = rest.get(query.saturating_add(1)..).unwrap_or_default();
        let end = tail
            .find(|character: char| character.is_whitespace() || character == ')')
            .unwrap_or(tail.len());
        rest = tail.get(end..).unwrap_or_default();
    }
    redacted_text.push_str(rest);
    redacted_text
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
    fn a_client_facing_message_keeps_only_the_file_name() {
        let error = Error::Ledger(
            "hash chain broken at sequence 4 in /home/someone/.local/share/ownserp/ledger/calls.jsonl"
                .to_owned(),
        );
        let body = error.to_json();
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .expect("message");
        assert!(!message.contains("/home/"), "{message}");
        assert!(message.contains("calls.jsonl"), "{message}");
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
    fn a_json_error_reports_its_cause_chain() {
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
    fn a_network_error_carries_its_sanitized_cause() {
        let error = Error::network(
            "calling Serper",
            std::io::Error::other("connection reset: https://api.serper.dev/search?q=secret"),
        );
        let body = error.to_json();
        let causes = body
            .get("causes")
            .and_then(Value::as_array)
            .expect("causes");
        let text = causes[0].as_str().expect("cause");
        assert!(text.contains("connection reset"));
        assert!(!body.to_string().contains("secret"));
    }

    #[test]
    fn a_budget_refused_error_carries_a_code_and_a_remedy() {
        let error = Error::BudgetRefused("10 credits".to_owned());
        let body = error.to_json();
        assert_eq!(
            body.get("code").and_then(Value::as_str),
            Some("serper.budget_refused")
        );
        assert!(body.get("remedy").and_then(Value::as_str).is_some());
    }

    #[test]
    fn a_busy_ledger_and_a_bad_request_carry_their_own_advice() {
        let busy = Error::LedgerBusy("held".to_owned());
        assert_eq!(busy.code(), "serper.ledger_fault");
        assert!(busy.remedy().contains("Another ownserp process"));

        let bad_request = Error::Upstream {
            status: 400,
            message: String::new(),
        };
        assert_eq!(bad_request.code(), "serper.bad_request");
        let unprocessable = Error::Upstream {
            status: 422,
            message: String::new(),
        };
        assert_eq!(unprocessable.code(), "serper.bad_request");

        let disk_remedy = Error::Ledger("broken".to_owned()).remedy();
        let json = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        assert_ne!(Error::from(json).remedy(), disk_remedy);
        assert_ne!(Error::Projection("shape".to_owned()).remedy(), disk_remedy);
    }

    #[test]
    fn a_rejected_redirected_or_rate_limited_error_is_unbilled() {
        for status in [301, 302, 400, 401, 403, 404, 422, 429] {
            let error = Error::Upstream {
                status,
                message: String::new(),
            };
            assert!(
                error.is_unbilled(),
                "status {status} must count as unbilled"
            );
        }
        let limited = Error::RateLimited {
            message: String::new(),
        };
        assert!(limited.is_unbilled(), "a rate limit must count as unbilled");
    }

    #[test]
    fn a_server_error_is_unbilled() {
        for status in [500, 502, 503, 504] {
            let error = Error::Upstream {
                status,
                message: String::new(),
            };
            assert!(
                error.is_unbilled(),
                "status {status} must count as unbilled"
            );
        }
    }

    #[test]
    fn a_timeout_cancel_or_bad_body_stays_billed() {
        let network = Error::network("calling Serper", std::io::Error::other("timed out"));
        assert!(!network.is_unbilled(), "a timeout may have been billed");
        assert!(
            !Error::Cancelled.is_unbilled(),
            "a cancel may have been billed"
        );
        let answered_with_a_bad_body = Error::Upstream {
            status: 200,
            message: String::new(),
        };
        assert!(
            !answered_with_a_bad_body.is_unbilled(),
            "a 2xx body that will not parse was still billed"
        );
        let json = serde_json::from_str::<serde_json::Value>("{").expect_err("invalid json");
        assert!(
            !Error::from(json).is_unbilled(),
            "a bad body may have been billed"
        );
        assert!(
            !Error::Projection("shape".to_owned()).is_unbilled(),
            "a projection failure follows a good response"
        );
    }
}
