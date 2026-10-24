use anyhow::Context;
use ownserp_core::{Overrides, Settings, default_data_dir};

use crate::cli::{AuditAction, AuditArgs};

pub(crate) fn run(args: AuditArgs) -> anyhow::Result<()> {
    match args.action {
        AuditAction::Verify { file } => {
            let path = match file {
                Some(path) => path,
                None => {
                    let settings =
                        Settings::load(Overrides::default()).context("resolving settings")?;
                    settings.ledger_dir().join("calls.jsonl")
                }
            };
            if !path.exists() && file_is_default(&path) {
                println!("no ledger yet at {}", path.display());
                return Ok(());
            }
            let rows = ownserp_core::verify(&path).context("verifying the ledger")?;
            println!("ledger verified: {rows} rows, hash chain intact");
            Ok(())
        }
        AuditAction::Rotate { file } => {
            let path = match file {
                Some(path) => path,
                None => {
                    let settings =
                        Settings::load(Overrides::default()).context("resolving settings")?;
                    settings.ledger_dir().join("calls.jsonl")
                }
            };
            let (rows, archive) = ownserp_core::rotate(&path).context("rotating the ledger")?;
            println!("verified {rows} rows, hash chain intact");
            println!("archived to {}", archive.display());
            println!("the next start writes a fresh chain at {}", path.display());
            Ok(())
        }
    }
}

fn file_is_default(path: &std::path::Path) -> bool {
    path.starts_with(default_data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{AuditAction, AuditArgs};
    use ownserp_core::{CallRecord, Ledger};

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ownserp-audit-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn record() -> CallRecord {
        CallRecord {
            tool: "serper_search".to_owned(),
            endpoint: "/search".to_owned(),
            args_hash: "0".repeat(64),
            credits_estimated: 1,
            credits_actual: None,
            batch_multiplier: 1,
            duration_ms: 2,
            status: "ok".to_owned(),
            response_bytes: 10,
            tokens: 2,
            tokens_saved: 1,
            cache_hit: false,
        }
    }

    #[test]
    fn verify_then_rotate_moves_the_file_and_leaves_the_chain_valid() {
        let dir = temp_dir("rotate");
        let path = dir.join("calls.jsonl");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("append");
        }
        run(AuditArgs {
            action: AuditAction::Verify {
                file: Some(path.clone()),
            },
        })
        .expect("verify");
        run(AuditArgs {
            action: AuditAction::Rotate {
                file: Some(path.clone()),
            },
        })
        .expect("rotate");
        assert!(!path.exists(), "live ledger is archived");
        let archived = std::fs::read_dir(&dir)
            .expect("dir")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name() != "calls.jsonl");
        assert!(archived, "an archive must exist");
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }

    #[test]
    fn a_file_under_the_data_directory_counts_as_the_default() {
        assert!(file_is_default(
            &ownserp_core::default_data_dir().join("ledger/calls.jsonl")
        ));
        assert!(!file_is_default(std::path::Path::new("/tmp/calls.jsonl")));
    }

    #[test]
    fn rotating_a_missing_ledger_reports_an_error() {
        let path = temp_dir("missing").join("calls.jsonl");
        let outcome = run(AuditArgs {
            action: AuditAction::Rotate { file: Some(path) },
        });
        assert!(outcome.is_err());
    }
}
