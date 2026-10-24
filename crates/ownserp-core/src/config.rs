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
    allowed_hosts: Option<Vec<String>>,
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
    #[serde(default = "profile_version")]
    version: u32,
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
    allowed_hosts: Option<Vec<String>>,
    request_timeout_secs: Option<u64>,
    http_qps: Option<u32>,
}

impl Settings {
    pub fn load(overrides: Overrides) -> Result<Self> {
        Self::load_with(overrides, |name| std::env::var(name).ok())
    }

    pub fn load_with<F>(overrides: Overrides, env: F) -> Result<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let requested_profile = overrides
            .profile
            .clone()
            .or_else(|| env("OWN_SERPER_PROFILE").map(PathBuf::from));
        let profile_path = requested_profile
            .clone()
            .or_else(|| Some(default_config_dir().join("profile.toml")));
        let profile = match profile_path.as_deref() {
            Some(path) if path.exists() => read_profile(path)?,
            Some(path) if requested_profile.is_some() => {
                return Err(Error::Config(format!(
                    "profile {} does not exist; fix --profile or OWN_SERPER_PROFILE",
                    path.display()
                )));
            }
            _ => Profile::default(),
        };
        if profile.version > 1 {
            let shown = profile_path
                .as_deref()
                .map_or_else(|| "profile".to_owned(), |path| path.display().to_string());
            return Err(Error::Config(format!(
                "{shown} is version {}, newer than this binary supports (1)",
                profile.version
            )));
        }
        let api_key = env("SERPER_API_KEY")
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
        let qps_env = env_parse::<u32>(&env, "OWN_SERPER_QPS")?;
        let cache_ttl_env = env_parse::<u64>(&env, "OWN_SERPER_CACHE_TTL_SECS")?;
        let usd_env = env_parse::<f64>(&env, "SERPER_USD_PER_1K")?;
        let http_qps_env = env_parse::<u32>(&env, "OWN_SERPER_HTTP_QPS")?;
        let usd_rate = usd_env.or(profile.usd_per_1k);
        if let Some(rate) = usd_rate
            && (!rate.is_finite() || rate < 0.0)
        {
            return Err(Error::Config(format!(
                "the USD rate {rate} is not a finite, non-negative number; set SERPER_USD_PER_1K or usd_per_1k"
            )));
        }
        let qps_raw = overrides
            .qps
            .or(qps_env)
            .or(profile.qps)
            .unwrap_or_else(|| overrides.pack.or(profile.pack).unwrap_or_default().qps());
        let pack = overrides.pack.or(profile.pack).unwrap_or_default();
        let bearer_tokens = env("OWN_SERPER_BEARER_TOKENS")
            .map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(SecretString::from)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let allowed_hosts = env("OWN_SERPER_ALLOWED_HOSTS")
            .map(|raw| parse_host_list(&raw))
            .filter(|hosts| !hosts.is_empty())
            .or(profile.allowed_hosts);
        Ok(Self {
            api_key,
            api_key_source,
            api_key_error,
            http: overrides.http.or(profile.http).unwrap_or(false),
            bind: overrides
                .bind
                .or_else(|| env("OWN_SERPER_BIND"))
                .or(profile.bind)
                .unwrap_or_else(|| "127.0.0.1:8777".to_owned()),
            bearer_tokens,
            allowed_hosts,
            pack,
            usd_per_1k: usd_rate,
            qps: QpsLimit::new(qps_raw)?,
            cache_ttl_secs: overrides
                .cache_ttl_secs
                .or(cache_ttl_env)
                .or(profile.cache_ttl_secs)
                .unwrap_or(0),
            cache_max_bytes: profile.cache_max_bytes.unwrap_or(16 * 1024 * 1024),
            ledger_dir: overrides
                .ledger_dir
                .or_else(|| env("OWN_SERPER_LEDGER_DIR").map(PathBuf::from))
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

    pub fn allowed_hosts(&self) -> Option<&[String]> {
        self.allowed_hosts.as_deref()
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
    let count = secret.chars().count();
    if count <= 4 {
        return "****".to_owned();
    }
    let tail: String = secret.chars().skip(count.saturating_sub(4)).collect();
    format!("****{tail}")
}

fn env_parse<T: std::str::FromStr>(
    env: &impl Fn(&str) -> Option<String>,
    name: &str,
) -> Result<Option<T>> {
    let Some(raw) = env(name) else {
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

fn parse_host_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|host| !host.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

const fn profile_version() -> u32 {
    1
}

fn read_profile(path: &Path) -> Result<Profile> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        Error::Config(format!("cannot read profile {}: {error}", path.display()))
    })?;
    toml::from_str::<Profile>(&raw)
        .map_err(|error| Error::Config(format!("profile {}: {error}", path.display())))
}

pub fn default_config_dir() -> PathBuf {
    base_dir("XDG_CONFIG_HOME", ".config", "APPDATA").join("ownserp")
}

pub fn default_data_dir() -> PathBuf {
    base_dir("XDG_DATA_HOME", ".local/share", "LOCALAPPDATA").join("ownserp")
}

fn base_dir(variable: &str, fallback: &str, windows_variable: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return path;
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(fallback);
    }
    #[cfg(windows)]
    if let Some(path) = std::env::var_os(windows_variable) {
        return PathBuf::from(path);
    }
    #[cfg(not(windows))]
    let _ = windows_variable;
    PathBuf::from(".")
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
            .field("allowed_hosts", &self.allowed_hosts)
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn profile_file(label: &str, body: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ownserp-config-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("profile.toml");
        std::fs::write(&path, body).expect("profile");
        path
    }

    fn load(
        label: &str,
        profile: &str,
        pairs: &[(&str, &str)],
        overrides: Overrides,
    ) -> Result<Settings> {
        let map = environment(pairs);
        Settings::load_with(
            Overrides {
                profile: Some(profile_file(label, profile)),
                ..overrides
            },
            |name| map.get(name).cloned(),
        )
    }

    fn base_overrides() -> Overrides {
        Overrides {
            ledger_dir: Some(std::env::temp_dir().join("ownserp-config-ledger")),
            cache_ttl_secs: Some(0),
            ..Overrides::default()
        }
    }

    #[test]
    fn the_environment_beats_the_profile_for_bind() {
        let settings = load(
            "bind",
            "bind = \"127.0.0.1:1111\"\n",
            &[
                ("SERPER_API_KEY", "test-key"),
                ("OWN_SERPER_BIND", "127.0.0.1:2222"),
            ],
            base_overrides(),
        )
        .expect("settings");
        assert_eq!(settings.bind(), "127.0.0.1:2222");
    }

    #[test]
    fn an_override_beats_the_environment() {
        let overrides = Overrides {
            qps: Some(7),
            ..base_overrides()
        };
        let settings = load(
            "qps",
            "",
            &[("SERPER_API_KEY", "test-key"), ("OWN_SERPER_QPS", "5")],
            overrides,
        )
        .expect("settings");
        assert_eq!(settings.qps().get().get(), 7);
    }

    #[test]
    fn an_unparseable_environment_value_is_an_error() {
        let error = load(
            "badqps",
            "",
            &[
                ("SERPER_API_KEY", "test-key"),
                ("OWN_SERPER_HTTP_QPS", "fast"),
            ],
            base_overrides(),
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("OWN_SERPER_HTTP_QPS"));
    }

    #[test]
    fn an_absent_variable_falls_back_to_the_profile() {
        let settings = load(
            "ttl",
            "cache_ttl_secs = 30\n",
            &[("SERPER_API_KEY", "test-key")],
            Overrides {
                cache_ttl_secs: None,
                ..base_overrides()
            },
        )
        .expect("settings");
        assert_eq!(settings.cache_ttl_secs(), 30);
    }

    #[test]
    fn the_usd_override_reads_from_the_environment() {
        let settings = load(
            "usd",
            "",
            &[
                ("SERPER_API_KEY", "test-key"),
                ("SERPER_USD_PER_1K", "0.42"),
            ],
            base_overrides(),
        )
        .expect("settings");
        assert_eq!(settings.usd_per_1k(), Some(0.42));
    }

    #[test]
    fn an_explicit_profile_that_is_missing_is_refused() {
        let map = environment(&[("SERPER_API_KEY", "test-key")]);
        let error = Settings::load_with(
            Overrides {
                profile: Some(
                    std::env::temp_dir().join("ownserp-profile-missing-do-not-create.toml"),
                ),
                ledger_dir: Some(std::env::temp_dir().join("ownserp-config-ledger")),
                ..Overrides::default()
            },
            |name| map.get(name).cloned(),
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn allowed_hosts_come_from_the_environment_first() {
        let settings = load(
            "hosts",
            "allowed_hosts = [\"profile.example\"]\n",
            &[
                ("SERPER_API_KEY", "test-key"),
                ("OWN_SERPER_ALLOWED_HOSTS", " one.example , two.example "),
            ],
            base_overrides(),
        )
        .expect("settings");
        let hosts = settings.allowed_hosts().expect("hosts");
        assert_eq!(
            hosts,
            vec!["one.example".to_owned(), "two.example".to_owned()]
        );
    }

    #[test]
    fn a_non_finite_rate_is_refused() {
        let error = load(
            "usdnan",
            "",
            &[("SERPER_API_KEY", "test-key"), ("SERPER_USD_PER_1K", "NaN")],
            base_overrides(),
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("SERPER_USD_PER_1K"));
    }

    #[test]
    fn bearer_tokens_are_split_and_trimmed() {
        let settings = load(
            "tokens",
            "",
            &[
                ("SERPER_API_KEY", "test-key"),
                ("OWN_SERPER_BEARER_TOKENS", " one , two ,, "),
            ],
            base_overrides(),
        )
        .expect("settings");
        assert_eq!(settings.bearer_tokens().len(), 2);
    }

    #[test]
    fn a_newer_profile_version_is_refused() {
        let error = load(
            "newer-profile",
            "version = 2\n",
            &[("SERPER_API_KEY", "test-key")],
            base_overrides(),
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("version 2"));
    }

    #[test]
    fn a_profile_with_an_unknown_key_is_rejected() {
        let error = load(
            "unknown",
            "bogus = 1\n",
            &[("SERPER_API_KEY", "test-key")],
            base_overrides(),
        )
        .expect_err("must fail");
        assert!(error.to_string().contains("profile"));
    }

    #[test]
    fn the_debug_output_never_carries_the_key() {
        let settings = load(
            "debug",
            "",
            &[("SERPER_API_KEY", "super-secret-value")],
            base_overrides(),
        )
        .expect("settings");
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains("super-secret-value"));
        assert!(rendered.contains("<set>"));
    }

    #[test]
    fn masking_keeps_only_the_tail() {
        assert_eq!(mask("abcdefgh"), "****efgh");
        assert_eq!(mask("abc"), "****");
    }
}
