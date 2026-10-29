use std::io::{IsTerminal, Read};

use anyhow::{Context, bail};
use ownserp_core::{SecretString, Settings, keychain_delete, keychain_set};

use crate::cli::{ConfigAction, ConfigArgs, ConfigShowArgs};
use crate::output::emit;

pub(crate) fn run(args: ConfigArgs) -> anyhow::Result<()> {
    match args.action {
        ConfigAction::SetApiKey => set_key(),
        ConfigAction::Show(args) => show(args),
        ConfigAction::DeleteApiKey => {
            keychain_delete().context("removing the key")?;
            emit("stored key removed; the environment variable is untouched")?;
            Ok(())
        }
    }
}

fn set_key() -> anyhow::Result<()> {
    if std::io::stdin().is_terminal() {
        bail!(
            "stdin is a terminal and a typed key would be echoed; pipe the key instead, for example: printf '%s\\n' \"$SERPER_API_KEY\" | ownserp config set-api-key"
        );
    }
    eprintln!("reading the API key from stdin");
    let trimmed = read_key(std::io::stdin().lock())?;
    let secret = SecretString::from(trimmed);
    keychain_set(&secret).context("storing the key in the OS keychain")?;
    emit("stored the key in the OS keychain")?;
    if std::env::var_os("SERPER_API_KEY").is_some_and(|value| !value.is_empty()) {
        emit(
            "SERPER_API_KEY is also set and takes precedence over the keychain, so this stored key will not be used until that variable is unset",
        )?;
    }
    Ok(())
}

fn read_key(reader: impl Read) -> anyhow::Result<String> {
    let mut buffer = Vec::new();
    reader
        .take(4096)
        .read_to_end(&mut buffer)
        .context("reading stdin")?;
    let raw = std::str::from_utf8(&buffer)
        .map_err(|_| anyhow::anyhow!("the key on stdin is not valid UTF-8"))?;
    let cleaned = clean_key(raw);
    buffer.fill(0);
    let _ = std::hint::black_box(&buffer);
    cleaned
}

fn clean_key(raw: &str) -> anyhow::Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("no key on stdin");
    }
    if trimmed.chars().count() > 512 {
        bail!("the key is longer than 512 characters");
    }
    if trimmed.chars().any(char::is_control) {
        bail!("the key contains a control character");
    }
    if trimmed.chars().any(char::is_whitespace) {
        bail!("the key contains whitespace");
    }
    Ok(trimmed.to_owned())
}

fn show(args: ConfigShowArgs) -> anyhow::Result<()> {
    let settings = Settings::load(args.into_overrides()).context("resolving settings")?;
    for line in summary(&settings) {
        emit(&line)?;
    }
    Ok(())
}

fn summary(settings: &Settings) -> Vec<String> {
    let mut lines = vec![
        format!("api key source: {}", settings.api_key_source().label()),
        format!("api key: {}", settings.masked_key()),
        format!("pack tier: {:?}", settings.pack()),
        format!("ledger directory: {}", settings.ledger_dir().display()),
        format!(
            "profile: {}",
            settings
                .profile_path()
                .map_or_else(|| "none".to_owned(), |path| path.display().to_string())
        ),
    ];
    if let Some(error) = settings.api_key_error() {
        lines.insert(1, format!("keychain error: {error}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ownserp_core::Overrides;

    use super::*;

    fn settings_with_key() -> Settings {
        let map = BTreeMap::from([("SERPER_API_KEY".to_owned(), "abcdefgh-test-key".to_owned())]);
        let profile = std::env::temp_dir().join("ownserp-config-summary.toml");
        std::fs::write(&profile, "").expect("profile");
        Settings::load_with(
            Overrides {
                profile: Some(profile),
                ledger_dir: Some(std::env::temp_dir().join("ownserp-config-summary-ledger")),
                cache_ttl_secs: Some(0),
                ..Overrides::default()
            },
            |name| map.get(name).cloned(),
        )
        .expect("settings")
    }

    #[test]
    fn a_key_line_is_trimmed_and_bounded() {
        assert_eq!(clean_key("  abc123  \n").expect("key"), "abc123");
        assert!(clean_key("   ").is_err());
        assert!(clean_key(&"a".repeat(513)).is_err());
        assert!(clean_key("abc\u{7}").is_err());
    }

    #[test]
    fn a_key_with_an_inner_space_is_rejected() {
        assert!(clean_key("abc def").is_err());
        assert!(read_key(std::io::Cursor::new(b"abc def\n")).is_err());
    }

    #[test]
    fn a_key_is_read_from_a_reader() {
        assert_eq!(
            read_key(std::io::Cursor::new(b"  abc123  \n")).expect("key"),
            "abc123"
        );
    }

    #[test]
    fn the_summary_masks_the_key_and_names_every_field() {
        let lines = summary(&settings_with_key());
        assert_eq!(lines.len(), 5);
        let joined = lines.join("\n");
        assert!(joined.contains("api key source: environment"));
        assert!(joined.contains("api key: ****-key"));
        assert!(!joined.contains("abcdefgh-test-key"));
        assert!(joined.contains("pack tier: Starter"));
        assert!(joined.contains("ledger directory:"));
    }
}
