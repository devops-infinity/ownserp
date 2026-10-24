use rmcp::ServiceExt;
use rmcp::transport::stdio;

use super::{OwnserpHandler, shutdown_signal};
use crate::config::Settings;
use crate::error::{Error, Result};

pub async fn serve_stdio(settings: Settings) -> Result<()> {
    let handler = OwnserpHandler::new(settings)?;
    let service = handler
        .serve(stdio())
        .await
        .map_err(|error| Error::Config(format!("stdio handshake failed: {error}")))?;
    let token = service.cancellation_token();
    let mut joined = tokio::spawn(service.waiting());
    tokio::select! {
        result = &mut joined => match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(Error::Config(format!("stdio session ended: {error}"))),
            Err(error) => Err(Error::Config(format!("stdio task ended: {error}"))),
        },
        () = shutdown_signal() => {
            token.cancel();
            let _ = joined.await;
            Ok(())
        }
    }
}
