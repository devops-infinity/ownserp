use anyhow::Context;
use ownserp_core::{Settings, serve_http, serve_stdio};

use crate::cli::ServeArgs;

pub(crate) fn run(args: ServeArgs) -> anyhow::Result<()> {
    let settings = Settings::load(args.into_overrides()).context("resolving settings")?;
    let http = settings.http();
    let runtime = if http {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .context("building the multithread runtime")?
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("building the current-thread runtime")?
    };
    runtime.block_on(async move {
        if http {
            serve_http(settings).await
        } else {
            serve_stdio(settings).await
        }
    })?;
    Ok(())
}
