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

use std::io::{IsTerminal, Write};
use std::process::ExitCode;

use clap::{CommandFactory, Parser};

mod audit;
mod cli;
mod config;
mod doctor;
mod output;
mod serve;
mod update;

fn main() -> ExitCode {
    init_tracing();
    install_panic_hook();
    let parsed = cli::Cli::parse();
    match run(parsed) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error)
            if error
                .downcast_ref::<output::PrintError>()
                .is_some_and(output::PrintError::is_broken_pipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ownserp: {error:#}");
            if let Some(remedy) = error
                .downcast_ref::<ownserp_core::Error>()
                .map(ownserp_core::Error::remedy)
            {
                eprintln!("ownserp: {remedy}");
            }
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
        cli::Command::Update { json } => update::run(json),
        cli::Command::Completions { shell } => {
            let mut command = cli::Cli::command();
            let mut buffer = Vec::new();
            clap_complete::generate(shell, &mut command, "ownserp", &mut buffer);
            write_output(std::io::stdout().lock(), &buffer)
        }
        cli::Command::Man => {
            let mut buffer = Vec::new();
            render_man(&mut buffer)?;
            write_output(std::io::stdout().lock(), &buffer)
        }
    }
}

fn write_output(mut writer: impl Write, bytes: &[u8]) -> anyhow::Result<()> {
    match writer.write_all(bytes).and_then(|()| writer.flush()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {
            Err(anyhow::Error::from(output::PrintError::BrokenPipe))
        }
        Err(error) => Err(anyhow::Error::from(output::PrintError::Io(error))),
    }
}

fn render_man(mut writer: impl Write) -> anyhow::Result<()> {
    clap_mangen::Man::new(cli::Cli::command())
        .render(&mut writer)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::BrokenPipe => anyhow::Error::from(output::PrintError::BrokenPipe),
            _ => anyhow::Error::from(output::PrintError::Io(error))
                .context("rendering the manual page"),
        })
}

fn init_tracing() {
    let filter = match tracing_subscriber::EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        Err(error) => {
            if std::env::var_os("RUST_LOG").is_some() {
                eprintln!("ownserp: RUST_LOG is not a valid filter ({error}); using info");
            }
            tracing_subscriber::EnvFilter::new("info")
        }
    };
    let raw_format = std::env::var("OWN_SERPER_LOG_FORMAT").ok();
    let (format, warning) = parse_log_format(raw_format.as_deref());
    if let Some(warning) = warning {
        eprintln!("ownserp: {warning}");
    }
    if format == LogFormat::Json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .json()
            .init();
    } else {
        let ansi = std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .with_ansi(ansi)
            .init();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LogFormat {
    Json,
    Text,
}

fn parse_log_format(raw: Option<&str>) -> (LogFormat, Option<String>) {
    match raw.map(str::trim).filter(|value| !value.is_empty()) {
        None => (LogFormat::Text, None),
        Some(value) if value.eq_ignore_ascii_case("json") => (LogFormat::Json, None),
        Some(value) if value.eq_ignore_ascii_case("text") => (LogFormat::Text, None),
        Some(value) => (
            LogFormat::Text,
            Some(format!(
                "OWN_SERPER_LOG_FORMAT is {value:?}; use json or text. Using text."
            )),
        ),
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing::error!(panic = %panic_info, "process panicked");
        previous(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    static PANIC_HOOK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct ClosedPipe;

    impl Write for ClosedPipe {
        fn write(&mut self, _buffer: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "closed",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_closed_pipe_from_completions_reads_as_success() {
        let error = write_output(ClosedPipe, b"completion").expect_err("closed pipe");
        assert!(
            error
                .downcast_ref::<output::PrintError>()
                .is_some_and(output::PrintError::is_broken_pipe)
        );
    }

    #[test]
    fn a_closed_pipe_from_the_manual_page_reads_as_success() {
        let error = render_man(ClosedPipe).expect_err("closed pipe");
        assert!(
            error
                .downcast_ref::<output::PrintError>()
                .is_some_and(output::PrintError::is_broken_pipe)
        );
    }

    #[test]
    fn a_case_insensitive_json_format_is_read() {
        assert_eq!(parse_log_format(Some("JSON")).0, LogFormat::Json);
        assert_eq!(parse_log_format(Some("json")).0, LogFormat::Json);
        assert_eq!(parse_log_format(Some("text")).0, LogFormat::Text);
        assert_eq!(parse_log_format(None).0, LogFormat::Text);
    }

    #[test]
    fn an_unknown_log_format_falls_back_to_text_with_a_warning() {
        let (format, warning) = parse_log_format(Some("yaml"));
        assert_eq!(format, LogFormat::Text);
        assert!(warning.is_some_and(|warning| warning.contains("json or text")));
    }

    #[test]
    fn the_panic_hook_keeps_the_previous_one() {
        static CALLED: AtomicBool = AtomicBool::new(false);
        let _guard = PANIC_HOOK_LOCK.lock().expect("lock");
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| CALLED.store(true, Ordering::SeqCst)));
        install_panic_hook();
        let _ = std::panic::catch_unwind(|| panic!("hook test"));
        std::panic::set_hook(previous);
        assert!(CALLED.load(Ordering::SeqCst));
    }
}
