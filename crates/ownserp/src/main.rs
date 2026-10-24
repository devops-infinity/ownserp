#![allow(clippy::print_stdout, clippy::print_stderr)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::process::ExitCode;

use clap::Parser;

mod audit;
mod cli;
mod config;
mod doctor;
mod serve;

fn main() -> ExitCode {
    init_tracing();
    install_panic_hook();
    let parsed = cli::Cli::parse();
    match run(parsed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ownserp: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(parsed: cli::Cli) -> anyhow::Result<()> {
    match parsed.command {
        cli::Command::Serve(args) => serve::run(args),
        cli::Command::Doctor(args) => doctor::run(args),
        cli::Command::Config(args) => config::run(args),
        cli::Command::Audit(args) => audit::run(args),
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    if std::env::var("OWN_SERPER_LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }
}

fn install_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        tracing::error!(panic = %panic_info, "process panicked");
    }));
}
