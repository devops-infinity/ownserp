use anyhow::{Context, bail};
use ownserp_core::{Overrides, Settings};

use crate::cli::{AuditAction, AuditArgs};
use crate::output::emit;

pub(crate) fn run(args: AuditArgs) -> anyhow::Result<()> {
    match args.action {
        AuditAction::Verify {
            ledger_file,
            profile,
            ledger_dir,
        } => {
            let given = ledger_file.is_some();
            let path = resolve(ledger_file, overrides(profile, ledger_dir))?;
            if !path.exists() {
                if !given {
                    emit(&format!("no ledger yet at {}", path.display()))?;
                    return Ok(());
                }
                bail!("no ledger at {}", path.display());
            }
            let rows = ownserp_core::verify(&path).context("verifying the ledger")?;
            emit(&format!(
                "ledger verified: {rows} rows, hash chain intact at {}",
                path.display()
            ))?;
            Ok(())
        }
        AuditAction::Rotate {
            ledger_file,
            profile,
            ledger_dir,
            archive_damaged,
        } => {
            let path = resolve(ledger_file, overrides(profile, ledger_dir))?;
            let (rows, archive) = if archive_damaged {
                ownserp_core::rotate_damaged(&path).context("archiving the damaged ledger")?
            } else {
                ownserp_core::rotate(&path).context("rotating the ledger")?
            };
            if archive_damaged {
                emit(&format!("counted {rows} good rows before archiving"))?;
            } else {
                emit(&format!("verified {rows} rows, hash chain intact"))?;
            }
            emit(&format!("archived to {}", archive.display()))?;
            emit(&next_write_message(&path))?;
            Ok(())
        }
        AuditAction::Remove {
            profile,
            ledger_dir,
            yes,
        } => {
            let dir = match ledger_dir {
                Some(dir) => dir,
                None => Settings::load(overrides(profile, None))
                    .context("resolving settings")?
                    .ledger_dir()
                    .to_path_buf(),
            };
            if !yes {
                emit(&format!(
                    "this would remove the ledger, its archives, and the arguments key in {}",
                    dir.display()
                ))?;
                bail!("nothing was removed; pass --yes to confirm the deletion");
            }
            let removed = ownserp_core::remove(&dir).context("removing the local ledger files")?;
            if removed.is_empty() {
                emit(&format!("nothing to remove in {}", dir.display()))?;
            } else {
                for path in removed {
                    emit(&format!("removed {}", path.display()))?;
                }
            }
            Ok(())
        }
    }
}

fn overrides(
    profile: Option<std::path::PathBuf>,
    ledger_dir: Option<std::path::PathBuf>,
) -> Overrides {
    Overrides {
        profile,
        ledger_dir,
        ..Overrides::default()
    }
}

fn resolve(
    ledger_file: Option<std::path::PathBuf>,
    overrides: Overrides,
) -> anyhow::Result<std::path::PathBuf> {
    match ledger_file {
        Some(path) => Ok(path),
        None => {
            let settings = Settings::load(overrides).context("resolving settings")?;
            Ok(settings.ledger_dir().join("calls.jsonl"))
        }
    }
}

fn next_write_message(path: &std::path::Path) -> String {
    format!(
        "the next write starts a fresh chain at {}; restart a running server so it opens the new chain",
        path.display()
    )
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
                ledger_file: Some(path.clone()),
                profile: None,
                ledger_dir: None,
            },
        })
        .expect("verify");
        run(AuditArgs {
            action: AuditAction::Rotate {
                ledger_file: Some(path.clone()),
                profile: None,
                ledger_dir: None,
                archive_damaged: false,
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
    fn the_rotation_message_names_the_next_write() {
        let message = next_write_message(std::path::Path::new("/tmp/calls.jsonl"));
        assert!(message.contains("next write"));
        assert!(!message.contains("next start"));
    }

    #[test]
    fn verifying_a_missing_named_ledger_under_the_data_directory_is_an_error() {
        let path = ownserp_core::default_data_dir()
            .join("ledger")
            .join(format!("ownserp-missing-{}.jsonl", std::process::id()));
        let outcome = run(AuditArgs {
            action: AuditAction::Verify {
                ledger_file: Some(path),
                profile: None,
                ledger_dir: None,
            },
        });
        assert!(outcome.is_err());
    }

    #[test]
    fn verifying_a_missing_named_ledger_is_an_error() {
        let path = temp_dir("missing-verify").join("calls.jsonl");
        let outcome = run(AuditArgs {
            action: AuditAction::Verify {
                ledger_file: Some(path),
                profile: None,
                ledger_dir: None,
            },
        });
        assert!(outcome.is_err());
    }

    #[test]
    fn rotating_a_missing_ledger_reports_an_error() {
        let path = temp_dir("missing").join("calls.jsonl");
        let outcome = run(AuditArgs {
            action: AuditAction::Rotate {
                ledger_file: Some(path),
                profile: None,
                ledger_dir: None,
                archive_damaged: false,
            },
        });
        assert!(outcome.is_err());
    }
}
