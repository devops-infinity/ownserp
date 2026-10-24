use anyhow::{Context, bail};
use ownserp_core::{ApiKeySource, SerperClient, Settings};

use crate::cli::DoctorArgs;
use crate::output::emit;

pub(crate) fn run(args: DoctorArgs) -> anyhow::Result<()> {
    let settings = Settings::load(args.into_overrides()).context("resolving settings")?;
    emit(&format!("OwnSERP {}", env!("CARGO_PKG_VERSION")))?;
    emit(&format!(
        "api key source: {}",
        settings.api_key_source().label()
    ))?;
    if let Some(error) = settings.api_key_error() {
        emit(&format!("keychain error: {error}"))?;
    }
    emit(&format!("api key: {}", settings.masked_key()))?;
    emit(&format!("pack tier: {:?}", settings.pack()))?;
    emit(&format!(
        "usd per 1000 credits: {}",
        format_usd_rate(&settings)
    ))?;
    emit(&format!(
        "queries per second ceiling: {}",
        settings.qps().get()
    ))?;
    emit(&format!("cache ttl seconds: {}", settings.cache_ttl_secs()))?;
    emit(&format!("cache byte cap: {}", settings.cache_max_bytes()))?;
    emit(&format!(
        "ledger directory: {}",
        settings.ledger_dir().display()
    ))?;
    emit(&format!("batch tool enabled: {}", settings.enable_batch()))?;
    emit(&format!(
        "transport: {}",
        if settings.http() {
            "streamable http"
        } else {
            "stdio"
        }
    ))?;
    let hosts = settings.allowed_hosts().map_or_else(
        || "from the bind address".to_owned(),
        |hosts| hosts.join(", "),
    );
    emit(&format!("allowed hosts: {hosts}"))?;
    if let Some(path) = settings.profile_path() {
        emit(&format!(
            "profile: {} (exists: {})",
            path.display(),
            path.exists()
        ))?;
    }
    if settings.api_key_source() == ApiKeySource::Missing {
        emit("result: no API key; search tools will refuse until one is set")?;
        bail!("no API key; set SERPER_API_KEY or run ownserp config set-api-key");
    }
    let client = SerperClient::new(&settings).context("building the client")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the runtime")?;
    let (health, balance) =
        runtime.block_on(async { (client.health().await, client.live_balance().await) });
    let reachable = health.is_some();
    match health {
        Some(body) => emit(&format!("serper api health: {body}"))?,
        None => emit("serper api health: no answer from api.serper.dev/health")?,
    }
    match balance {
        Some(credits) => emit(&format!("account balance: {credits} credits"))?,
        None => emit("account balance: not reported; GET /account did not answer with a balance")?,
    }
    if !reachable {
        bail!("api.serper.dev/health did not answer");
    }
    Ok(())
}

fn format_usd_rate(settings: &Settings) -> String {
    match settings
        .usd_per_1k()
        .or_else(|| settings.pack().usd_per_1k())
    {
        Some(rate) => format!("${rate:.2}"),
        None => "set SERPER_USD_PER_1K".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use ownserp_core::Overrides;

    use super::*;

    fn settings(pairs: &[(&str, &str)]) -> Settings {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect();
        let profile = std::env::temp_dir().join("ownserp-doctor-test.toml");
        std::fs::write(&profile, "").expect("profile");
        Settings::load_with(
            Overrides {
                profile: Some(profile),
                ledger_dir: Some(std::env::temp_dir().join("ownserp-doctor-test")),
                cache_ttl_secs: Some(0),
                ..Overrides::default()
            },
            |name| map.get(name).cloned(),
        )
        .expect("settings")
    }

    #[test]
    fn the_rate_comes_from_the_pack_or_the_override() {
        assert_eq!(format_usd_rate(&settings(&[])), "$1.00");
        assert_eq!(
            format_usd_rate(&settings(&[("SERPER_USD_PER_1K", "0.42")])),
            "$0.42"
        );
    }
}
