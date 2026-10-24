use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use ownserp_core::{Overrides, PackTier};

#[derive(Debug, Parser)]
#[command(
    name = "ownserp",
    version,
    about = "MCP server for the Serper.dev search API",
    long_about = "Exposes the Serper.dev search endpoints as MCP tools over stdio or Streamable HTTP, with per-call credit accounting."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    Serve(ServeArgs),
    Doctor(DoctorArgs),
    Config(ConfigArgs),
    Audit(AuditArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    #[arg(long, help = "Serve Streamable HTTP instead of stdio")]
    pub(crate) http: bool,
    #[arg(long, help = "Bind address for HTTP mode, for example 127.0.0.1:8777")]
    pub(crate) bind: Option<String>,
    #[arg(long, help = "Path to a TOML profile file")]
    pub(crate) profile: Option<PathBuf>,
    #[arg(long, help = "Directory that holds the call ledger")]
    pub(crate) ledger_dir: Option<PathBuf>,
    #[arg(long, help = "Client-side queries per second ceiling")]
    pub(crate) qps: Option<u32>,
    #[arg(long, value_parser = parse_pack, help = "Credit pack tier for dollar figures")]
    pub(crate) pack: Option<PackTier>,
    #[arg(long, help = "Enable the response cache for this many seconds")]
    pub(crate) cache_ttl: Option<u64>,
    #[arg(long, help = "Enable the batch search tool")]
    pub(crate) enable_batch: bool,
}

impl ServeArgs {
    pub(crate) fn into_overrides(self) -> Overrides {
        Overrides {
            http: self.http.then_some(true),
            bind: self.bind,
            profile: self.profile,
            ledger_dir: self.ledger_dir,
            qps: self.qps,
            pack: self.pack,
            cache_ttl_secs: self.cache_ttl,
            enable_batch: self.enable_batch.then_some(true),
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct DoctorArgs {
    #[arg(long, help = "Path to a TOML profile file")]
    pub(crate) profile: Option<PathBuf>,
    #[arg(long, help = "Directory that holds the call ledger")]
    pub(crate) ledger_dir: Option<PathBuf>,
}

impl DoctorArgs {
    pub(crate) fn into_overrides(self) -> Overrides {
        Overrides {
            profile: self.profile,
            ledger_dir: self.ledger_dir,
            ..Overrides::default()
        }
    }
}

#[derive(Debug, Args)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) action: ConfigAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigAction {
    #[command(about = "Read the API key from stdin and store it in the OS keychain")]
    SetApiKey,
    #[command(about = "Print the resolved settings with the key masked")]
    Show,
    #[command(about = "Remove the stored API key from the OS keychain")]
    DeleteApiKey,
}

#[derive(Debug, Args)]
pub(crate) struct AuditArgs {
    #[command(subcommand)]
    pub(crate) action: AuditAction,
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuditAction {
    #[command(about = "Recompute the ledger hash chain and report any break")]
    Verify {
        #[arg(
            long,
            help = "Ledger file, defaulting to calls.jsonl in the data directory"
        )]
        file: Option<PathBuf>,
    },
    #[command(
        about = "Verify the chain, archive the ledger, and leave a fresh one for the next start; stop the server first"
    )]
    Rotate {
        #[arg(
            long,
            help = "Ledger file, defaulting to calls.jsonl in the data directory"
        )]
        file: Option<PathBuf>,
    },
}

fn parse_pack(raw: &str) -> Result<PackTier, String> {
    match raw.to_ascii_lowercase().as_str() {
        "starter" => Ok(PackTier::Starter),
        "standard" => Ok(PackTier::Standard),
        "scale" => Ok(PackTier::Scale),
        "ultimate" => Ok(PackTier::Ultimate),
        "custom" => Ok(PackTier::Custom),
        other => Err(format!(
            "unknown pack {other}; use starter, standard, scale, ultimate, or custom"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_names_parse_and_unknown_names_fail() {
        assert_eq!(parse_pack("starter").expect("starter"), PackTier::Starter);
        assert_eq!(
            parse_pack("ULTIMATE").expect("case folding"),
            PackTier::Ultimate
        );
        assert!(parse_pack("enterprise").is_err());
    }

    #[test]
    fn doctor_flags_become_overrides() {
        let args = DoctorArgs {
            profile: Some(PathBuf::from("/tmp/profile.toml")),
            ledger_dir: Some(PathBuf::from("/tmp/ledger")),
        };
        let overrides = args.into_overrides();
        assert_eq!(overrides.profile, Some(PathBuf::from("/tmp/profile.toml")));
        assert_eq!(overrides.ledger_dir, Some(PathBuf::from("/tmp/ledger")));
        assert_eq!(overrides.http, None);
    }

    #[test]
    fn serve_flags_become_overrides_only_when_set() {
        let args = ServeArgs {
            http: false,
            bind: None,
            profile: None,
            ledger_dir: None,
            qps: None,
            pack: None,
            cache_ttl: None,
            enable_batch: false,
        };
        let overrides = args.into_overrides();
        assert_eq!(overrides.http, None);
        assert_eq!(overrides.enable_batch, None);
    }
}
