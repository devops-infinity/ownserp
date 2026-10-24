use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use ownserp_core::{Overrides, PackTier};

#[derive(Debug, Parser)]
#[command(
    name = "ownserp",
    display_name = "OwnSERP",
    version,
    propagate_version = true,
    about = "OwnSERP, an MCP server for the Serper.dev search API",
    long_about = "OwnSERP exposes the Serper.dev search endpoints as MCP tools over stdio or Streamable HTTP, with per-call credit accounting."
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    #[command(about = "Serve the MCP tools over stdio, or over Streamable HTTP with --http")]
    Serve(ServeArgs),
    #[command(about = "Print the resolved settings and probe the Serper API")]
    Doctor(DoctorArgs),
    #[command(about = "Store, show, or remove the Serper API key")]
    Config(ConfigArgs),
    #[command(about = "Verify or rotate the call ledger")]
    Audit(AuditArgs),
    #[command(about = "Print shell completions for a supported shell")]
    Completions {
        #[arg(value_enum, help = "Shell to generate completions for")]
        shell: clap_complete::Shell,
    },
    #[command(about = "Print the manual page")]
    Man,
}

#[derive(Debug, Args)]
pub(crate) struct ServeArgs {
    #[arg(
        long,
        help = "Serve Streamable HTTP instead of stdio",
        conflicts_with = "no_http"
    )]
    pub(crate) http: bool,
    #[arg(
        long = "no-http",
        help = "Serve over stdio even when the profile sets http = true"
    )]
    pub(crate) no_http: bool,
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
    #[arg(
        long = "cache-ttl",
        help = "Enable the response cache for this many seconds"
    )]
    pub(crate) cache_ttl_secs: Option<u64>,
    #[arg(
        long,
        help = "Enable the batch search tool",
        conflicts_with = "no_batch"
    )]
    pub(crate) enable_batch: bool,
    #[arg(
        long = "no-batch",
        help = "Disable the batch search tool even when the profile enables it"
    )]
    pub(crate) no_batch: bool,
}

impl ServeArgs {
    pub(crate) fn into_overrides(self) -> Overrides {
        let http = if self.http {
            Some(true)
        } else if self.no_http {
            Some(false)
        } else {
            None
        };
        let enable_batch = if self.enable_batch {
            Some(true)
        } else if self.no_batch {
            Some(false)
        } else {
            None
        };
        Overrides {
            http,
            bind: self.bind,
            profile: self.profile,
            ledger_dir: self.ledger_dir,
            qps: self.qps,
            pack: self.pack,
            cache_ttl_secs: self.cache_ttl_secs,
            enable_batch,
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
            long = "file",
            help = "Ledger file, defaulting to calls.jsonl in the data directory"
        )]
        ledger_file: Option<PathBuf>,
    },
    #[command(
        about = "Verify the chain, archive the ledger, and leave a fresh one for the next start"
    )]
    Rotate {
        #[arg(
            long = "file",
            help = "Ledger file, defaulting to calls.jsonl in the data directory"
        )]
        ledger_file: Option<PathBuf>,
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
            no_http: false,
            bind: None,
            profile: None,
            ledger_dir: None,
            qps: None,
            pack: None,
            cache_ttl_secs: None,
            enable_batch: false,
            no_batch: false,
        };
        let overrides = args.into_overrides();
        assert_eq!(overrides.http, None);
        assert_eq!(overrides.enable_batch, None);
    }

    #[test]
    fn the_negative_flags_turn_a_profile_setting_off() {
        let args = ServeArgs {
            http: false,
            no_http: true,
            bind: None,
            profile: None,
            ledger_dir: None,
            qps: None,
            pack: None,
            cache_ttl_secs: None,
            enable_batch: false,
            no_batch: true,
        };
        let overrides = args.into_overrides();
        assert_eq!(overrides.http, Some(false));
        assert_eq!(overrides.enable_batch, Some(false));
    }

    #[test]
    fn the_command_tree_is_internally_consistent() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
