use rmcp::ServiceExt;
use rmcp::transport::stdio;

use super::{OwnserpHandler, shutdown_signal};
use crate::config::Settings;
use crate::error::{Error, Result};

pub async fn serve_stdio(settings: Settings) -> Result<()> {
    let deadline = super::http::call_deadline(settings.request_timeout_secs());
    let handler = OwnserpHandler::with_deadline(settings, Some(deadline))?;
    let handler_cancel = handler.cancellation_token();
    let mut shutdown = std::pin::pin!(shutdown_signal());
    let service = tokio::select! {
        result = handler.serve(stdio()) => result
            .map_err(|error| session_error("starting the stdio session", error))?,
        () = &mut shutdown => return Ok(()),
    };
    let token = service.cancellation_token();
    let mut joined = tokio::spawn(service.waiting());
    tokio::select! {
        result = &mut joined => match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(session_error("running the stdio session", error)),
            Err(error) => Err(Error::Io {
                context: "waiting for the stdio session",
                source: std::io::Error::other(error),
            }),
        },
        () = &mut shutdown => {
            handler_cancel.cancel();
            token.cancel();
            if tokio::time::timeout(super::CANCELLED_CALL_GRACE, joined).await.is_err() {
                tracing::warn!("the stdio session did not stop within the cancel grace period");
            }
            Ok(())
        }
    }
}

fn session_error(context: &'static str, error: impl std::fmt::Display) -> Error {
    Error::Io {
        context,
        source: std::io::Error::other(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_failure_is_an_io_error_not_a_config_error() {
        let error = session_error("running the stdio session", "the pipe closed");
        assert_eq!(error.code(), "serper.internal_error");
        assert!(error.to_string().contains("running the stdio session"));
    }
}
