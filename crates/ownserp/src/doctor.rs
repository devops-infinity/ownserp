use anyhow::Context;
use ownserp_core::{ApiKeySource, SerperClient, Settings};

use crate::cli::DoctorArgs;

pub(crate) fn run(args: DoctorArgs) -> anyhow::Result<()> {
    let settings = Settings::load(args.into_overrides()).context("resolving settings")?;
    println!("ownserp {}", env!("CARGO_PKG_VERSION"));
    println!("api key source: {}", settings.api_key_source().label());
    if let Some(error) = settings.api_key_error() {
        println!("keychain error: {error}");
    }
    println!("api key: {}", settings.masked_key());
    println!("pack tier: {:?}", settings.pack());
    println!("usd per 1000 credits: {}", usd_rate(&settings));
    println!("queries per second ceiling: {}", settings.qps().get());
    println!("cache ttl seconds: {}", settings.cache_ttl_secs());
    println!("cache byte cap: {}", settings.cache_max_bytes());
    println!("ledger directory: {}", settings.ledger_dir().display());
    println!("batch tool enabled: {}", settings.enable_batch());
    println!(
        "transport: {}",
        if settings.http() {
            "streamable http"
        } else {
            "stdio"
        }
    );
    if let Some(path) = settings.profile_path() {
        println!("profile: {} (exists: {})", path.display(), path.exists());
    }
    if settings.api_key_source() == ApiKeySource::Missing {
        println!("result: no API key; search tools will refuse until one is set");
        return Ok(());
    }
    let client = SerperClient::new(&settings).context("building the client")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the runtime")?;
    let (health, balance) =
        runtime.block_on(async { (client.health().await, client.live_balance().await) });
    match health {
        Some(body) => println!("serper api health: {body}"),
        None => println!("serper api health: no answer from api.serper.dev/health"),
    }
    match balance {
        Some(credits) => println!("account balance: {credits} credits"),
        None => {
            println!("account balance: not reported; GET /account did not answer with a balance")
        }
    }
    Ok(())
}

fn usd_rate(settings: &Settings) -> String {
    match settings
        .usd_per_1k()
        .or_else(|| settings.pack().usd_per_thousand())
    {
        Some(rate) => format!("${rate:.2}"),
        None => "set SERPER_USD_PER_1K".to_owned(),
    }
}
