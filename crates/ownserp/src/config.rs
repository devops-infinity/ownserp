use std::io::BufRead;

use anyhow::{Context, bail};
use ownserp_core::{Overrides, SecretString, Settings, keychain_delete, keychain_set};

use crate::cli::{ConfigAction, ConfigArgs};

pub(crate) fn run(args: ConfigArgs) -> anyhow::Result<()> {
    match args.action {
        ConfigAction::SetApiKey => set_key(),
        ConfigAction::Show => show(),
        ConfigAction::DeleteApiKey => {
            keychain_delete().context("removing the key")?;
            println!("stored key removed; the environment variable is untouched");
            Ok(())
        }
    }
}

fn set_key() -> anyhow::Result<()> {
    eprintln!("reading the API key from stdin");
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading stdin")?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
        bail!("no key on stdin");
    }
    let secret = SecretString::from(trimmed.to_owned());
    keychain_set(&secret).context("storing the key in the OS keychain")?;
    println!("stored the key in the OS keychain");
    Ok(())
}

fn show() -> anyhow::Result<()> {
    let settings = Settings::load(Overrides::default()).context("resolving settings")?;
    println!("api key source: {}", settings.api_key_source().label());
    println!("api key: {}", settings.masked_key());
    println!("pack tier: {:?}", settings.pack());
    println!("ledger directory: {}", settings.ledger_dir().display());
    println!(
        "profile: {}",
        settings
            .profile_path()
            .map_or_else(|| "none".to_owned(), |path| path.display().to_string())
    );
    Ok(())
}
