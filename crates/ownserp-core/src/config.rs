use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::{Error, Result};
use crate::types::{ApiKeySource, PackTier, QpsLimit};

pub const KEYRING_SERVICE: &str = "ownserp";
pub const KEYRING_USER: &str = "serper-api-key";

#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub http: Option<bool>,
    pub bind: Option<String>,
    pub profile: Option<PathBuf>,
    pub ledger_dir: Option<PathBuf>,
    pub qps: Option<u32>,
    pub pack: Option<PackTier>,
    pub cache_ttl_secs: Option<u64>,
    pub enable_batch: Option<bool>,
}

#[derive(Clone)]
pub struct Settings {
    api_key: Option<SecretString>,
    api_key_source: ApiKeySource,
    api_key_error: Option<String>,
    http: bool,
    bind: String,
    bearer_tokens: Vec<SecretString>,
    pack: PackTier,
    usd_per_1k: Option<f64>,
    qps: QpsLimit,
    cache_ttl_secs: u64,
    cache_max_bytes: usize,
    ledger_dir: PathBuf,
    per_call_credit_ceiling: Option<u32>,
    session_budget_credits: Option<u64>,
    enable_batch: bool,
    request_timeout_secs: u64,
    http_qps: Option<u32>,
    profile_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    pack: Option<PackTier>,
    usd_per_1k: Option<f64>,
    qps: Option<u32>,
    cache_ttl_secs: Option<u64>,
    cache_max_bytes: Option<usize>,
    ledger_dir: Option<PathBuf>,
    per_call_credit_ceiling: Option<u32>,
    session_budget_credits: Option<u64>,
    enable_batch: Option<bool>,
    http: Option<bool>,
    bind: Option<String>,
    request_timeout_secs: Option<u64>,
    http_qps: Option<u32>,
}

impl Settings {
    pub fn load(overrides: Overrides) -> Result<Self> {
        let profile_path = overrides
            .profile
            .clone()
            .or_else(|| std::env::var_os("OWN_SERPER_PROFILE").map(PathBuf::from))
            .or_else(|| Some(default_config_dir().join("profile.toml")));
        let profile = match profile_path.as_deref() {
            Some(path) if path.exists() => read_profile(path)?,
            _ => Profile::default(),
        };
        let api_key = std::env::var("SERPER_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(SecretString::from);
        let (api_key, api_key_source, api_key_error) = match api_key {
            Some(key) => (Some(key), ApiKeySource::Environment, None),
            None => match keychain_get() {
                Ok(Some(key)) => (Some(key), ApiKeySource::Keychain, None),
                Ok(None) => (None, ApiKeySource::Missing, None),
                Err(error) => (None, ApiKeySource::Missing, Some(error.to_string())),
            },
        };
        let qps_env = env_parse::<u32>("OWN_SERPER_QPS")?;
        let cache_ttl_env = env_parse::<u64>("OWN_SERPER_CACHE_TTL_SECS")?;
        let usd_env = env_parse::<f64>("SERPER_USD_PER_1K")?;
        let http_qps_env = env_parse::<u32>("OWN_SERPER_HTTP_QPS")?;
        let qps_raw = overrides
            .qps
            .or(qps_env)
            .or(profile.qps)
            .unwrap_or_else(|| overrides.pack.or(profile.pack).unwrap_or_default().qps());
        let pack = overrides.pack.or(profile.pack).unwrap_or_default();
        let bearer_tokens = std::env::var("OWN_SERPER_BEARER_TOKENS")
            .ok()
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(SecretString::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        Ok(Self {
            api_key,
            api_key_source,
            api_key_error,
            http: overrides.http.or(profile.http).unwrap_or(false),
            bind: overrides
                .bind
                .or_else(|| std::env::var("OWN_SERPER_BIND").ok())
                .or(profile.bind)
                .unwrap_or_else(|| "127.0.0.1:8777".to_owned()),
            bearer_tokens,
            pack,
            usd_per_1k: usd_env.or(profile.usd_per_1k),
            qps: QpsLimit::new(qps_raw)?,
            cache_ttl_secs: overrides
                .cache_ttl_secs
                .or(cache_ttl_env)
                .or(profile.cache_ttl_secs)
                .unwrap_or(0),
            cache_max_bytes: profile.cache_max_bytes.unwrap_or(16 * 1024 * 1024),
            ledger_dir: overrides
                .ledger_dir
                .or_else(|| std::env::var_os("OWN_SERPER_LEDGER_DIR").map(PathBuf::from))
                .or(profile.ledger_dir)
                .unwrap_or_else(|| default_data_dir().join("ledger")),
            per_call_credit_ceiling: profile.per_call_credit_ceiling,
            session_budget_credits: profile.session_budget_credits,
            enable_batch: overrides
                .enable_batch
                .or(profile.enable_batch)
                .unwrap_or(false),
            request_timeout_secs: profile.request_timeout_secs.unwrap_or(30),
            http_qps: http_qps_env.or(profile.http_qps),
            profile_path,
        })
    }

    pub fn api_key(&self) -> Option<&SecretString> {
        self.api_key.as_ref()
    }

    pub fn require_api_key(&self) -> Result<&SecretString> {
        self.api_key.as_ref().ok_or(Error::MissingApiKey)
    }

    pub const fn api_key_source(&self) -> ApiKeySource {
        self.api_key_source
    }

    pub fn api_key_error(&self) -> Option<&str> {
        self.api_key_error.as_deref()
    }

    pub fn http(&self) -> bool {
        self.http
    }

    pub fn bind(&self) -> &str {
        &self.bind
    }

    pub fn bearer_tokens(&self) -> &[SecretString] {
        &self.bearer_tokens
    }

    pub fn pack(&self) -> PackTier {
        self.pack
    }

    pub fn usd_per_1k(&self) -> Option<f64> {
        self.usd_per_1k
    }

    pub fn qps(&self) -> QpsLimit {
        self.qps
    }

    pub fn cache_ttl_secs(&self) -> u64 {
        self.cache_ttl_secs
    }

    pub fn cache_max_bytes(&self) -> usize {
        self.cache_max_bytes
    }

    pub fn ledger_dir(&self) -> &Path {
        &self.ledger_dir
    }

    pub fn per_call_credit_ceiling(&self) -> Option<u32> {
        self.per_call_credit_ceiling
    }

    pub fn session_budget_credits(&self) -> Option<u64> {
        self.session_budget_credits
    }

    pub fn enable_batch(&self) -> bool {
        self.enable_batch
    }

    pub fn request_timeout_secs(&self) -> u64 {
        self.request_timeout_secs
    }

    pub fn http_qps(&self) -> Option<u32> {
        self.http_qps
    }

    pub fn profile_path(&self) -> Option<&Path> {
        self.profile_path.as_deref()
    }

    pub fn masked_key(&self) -> String {
        match self.api_key.as_ref() {
            Some(key) => mask(key.expose_secret()),
            None => "<unset>".to_owned(),
        }
    }
}

pub fn mask(secret: &str) -> String {
    let chars: Vec<char> = secret.chars().collect();
    if chars.len() <= 4 {
        return "****".to_owned();
    }
    let tail: String = chars
        .iter()
        .skip(chars.len().saturating_sub(4))
        .collect::<String>();
    format!("****{tail}")
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Result<Option<T>> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<T>().map(Some).map_err(|_| {
        Error::Config(format!(
            "{name} is set to {trimmed:?}, which is not a valid {}",
            std::any::type_name::<T>()
        ))
    })
}

fn read_profile(path: &Path) -> Result<Profile> {
    let raw = std::fs::read_to_string(path)?;
    toml::from_str::<Profile>(&raw)
        .map_err(|error| Error::Config(format!("profile {}: {error}", path.display())))
}

pub fn default_config_dir() -> PathBuf {
    base_dir("XDG_CONFIG_HOME", ".config").join("ownserp")
}

pub fn default_data_dir() -> PathBuf {
    base_dir("XDG_DATA_HOME", ".local/share").join("ownserp")
}

fn base_dir(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(fallback)))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn keychain_get() -> Result<Option<SecretString>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| Error::keychain("using the OS keychain", error))?;
    match entry.get_password() {
        Ok(secret) => Ok(Some(SecretString::from(secret))),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(Error::keychain("using the OS keychain", error)),
    }
}

pub fn keychain_set(secret: &SecretString) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| Error::keychain("using the OS keychain", error))?;
    entry
        .set_password(secret.expose_secret())
        .map_err(|error| Error::keychain("using the OS keychain", error))
}

pub fn keychain_delete() -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|error| Error::keychain("using the OS keychain", error))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(error) => Err(Error::keychain("using the OS keychain", error)),
    }
}

impl std::fmt::Debug for Settings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Settings")
            .field(
                "api_key",
                &if self.api_key.is_some() {
                    "<set>"
                } else {
                    "<unset>"
                },
            )
            .field("api_key_source", &self.api_key_source)
            .field("api_key_error", &self.api_key_error)
            .field("http", &self.http)
            .field("bind", &self.bind)
            .field("bearer_tokens", &self.bearer_tokens.len())
            .field("pack", &self.pack)
            .field("usd_per_1k", &self.usd_per_1k)
            .field("qps", &self.qps)
            .field("cache_ttl_secs", &self.cache_ttl_secs)
            .field("cache_max_bytes", &self.cache_max_bytes)
            .field("ledger_dir", &self.ledger_dir)
            .field("per_call_credit_ceiling", &self.per_call_credit_ceiling)
            .field("session_budget_credits", &self.session_budget_credits)
            .field("enable_batch", &self.enable_batch)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("http_qps", &self.http_qps)
            .field("profile_path", &self.profile_path)
            .finish()
    }
}
