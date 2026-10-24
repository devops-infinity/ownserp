use anyhow::Context;
use ownserp_core::{Settings, serve_http, serve_stdio};

use crate::cli::ServeArgs;

pub(crate) fn run(args: ServeArgs) -> anyhow::Result<()> {
    let bind_given = args.bind.is_some();
    let settings = Settings::load(args.into_overrides()).context("resolving settings")?;
    let http = settings.http();
    if bind_is_ignored(bind_given, http) {
        tracing::warn!(
            "--bind is ignored because the transport is stdio; pass --http to serve Streamable HTTP"
        );
    }
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
    block_on_and_stop(runtime, async move {
        if http {
            serve_http(settings).await?;
        } else {
            serve_stdio(settings).await?;
        }
        Ok(())
    })
}

fn block_on_and_stop<F>(runtime: tokio::runtime::Runtime, future: F) -> anyhow::Result<()>
where
    F: std::future::Future<Output = anyhow::Result<()>>,
{
    let outcome = runtime.block_on(future);
    runtime.shutdown_timeout(std::time::Duration::from_millis(500));
    outcome
}

const fn bind_is_ignored(bind_given: bool, http: bool) -> bool {
    bind_given && !http
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blocking_input_read_does_not_hold_the_runtime_open() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.spawn_blocking(|| std::thread::sleep(std::time::Duration::from_secs(5)));
        let started = std::time::Instant::now();
        block_on_and_stop(runtime, async { Ok(()) }).expect("shutdown");
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn the_bind_flag_is_reported_unused_only_for_stdio() {
        assert!(bind_is_ignored(true, false));
        assert!(!bind_is_ignored(true, true));
        assert!(!bind_is_ignored(false, false));
    }
}
