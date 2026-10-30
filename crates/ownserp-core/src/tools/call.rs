use std::sync::Arc;
use std::time::Instant;

use hmac::{Hmac, KeyInit, Mac};
use rmcp::model::{CallToolResult, ContentBlock};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::cache::ResponseCache;
use crate::client::SerperClient;
use crate::config::Settings;
use crate::cost::{BATCH_MULTIPLIER, Endpoint, usd_for_call, usd_for_credits};
use crate::error::{Error, Result};
use crate::ledger::{CallRecord, Ledger, Reservation};
use crate::projection::{self, SearchSections};
use crate::response::{
    BatchEntry, BatchOutput, CostInfo, Envelope, HealthOutput, ListOutput, ResolveOutput,
    ScrapeOutput, SessionOutput, ToolUsage,
};
use crate::tools::args::{
    BatchArgs, HealthArgs, LensArgs, LocationsArgs, MAX_BATCH_QUERIES, ResolveArgs, ReviewsArgs,
    ScrapeArgs, SearchArgs, SessionArgs,
};
use crate::types::{Preset, TextMode};

pub struct ToolContext {
    pub client: SerperClient,
    pub settings: Settings,
    pub ledger: Ledger,
    pub cache: ResponseCache,
    hash_key: Arc<[u8; 32]>,
}

impl ToolContext {
    pub fn new(settings: Settings) -> Result<Self> {
        let client = SerperClient::new(&settings)?;
        let ledger = Ledger::open(settings.ledger_dir().join("calls.jsonl"))?;
        let cache = ResponseCache::new(settings.cache_ttl_secs(), settings.cache_max_bytes());
        let hash_key = Arc::new(load_hash_key(settings.ledger_dir())?);
        Ok(Self {
            client,
            settings,
            ledger,
            cache,
            hash_key,
        })
    }
}

struct TempKeyFile {
    path: std::path::PathBuf,
    armed: bool,
}

impl Drop for TempKeyFile {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn load_hash_key(ledger_dir: &std::path::Path) -> Result<[u8; 32]> {
    let dir = ledger_dir.parent().unwrap_or(ledger_dir);
    let path = dir.join("args.key");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(Error::Config(format!(
                "{} is not a regular file; point the ledger at a directory only you can write",
                path.display()
            )));
        }
        Ok(metadata) if shared_with_others(&metadata) => {
            return Err(Error::Config(format!(
                "{} is readable or writable by another user; make it owner-only or remove it on purpose to start a new ledger chain",
                path.display()
            )));
        }
        Ok(_) => return read_hash_key(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(Error::Config(format!(
                "cannot inspect {}: {error}",
                path.display()
            )));
        }
    }
    let mut key = [0u8; 32];
    getrandom::fill(&mut key).map_err(|error| Error::Config(format!("os random: {error}")))?;
    let mut tag = [0u8; 8];
    getrandom::fill(&mut tag).map_err(|error| Error::Config(format!("os random: {error}")))?;
    std::fs::create_dir_all(dir)?;
    lock_down_key_dir(dir)?;
    let temporary = dir.join(format!(
        "args.key.{}-{}",
        std::process::id(),
        hex::encode(tag)
    ));
    let mut temporary_guard = TempKeyFile {
        path: temporary.clone(),
        armed: true,
    };
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    use std::io::Write as _;
    let mut file = options.open(&temporary)?;
    file.write_all(&key)?;
    file.sync_all()?;
    drop(file);
    let linked = std::fs::hard_link(&temporary, &path);
    let _ = std::fs::remove_file(&temporary);
    temporary_guard.armed = false;
    match linked {
        Ok(()) => {
            sync_directory(dir);
            Ok(key)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => read_hash_key(&path),
        Err(error) => Err(Error::Config(format!(
            "cannot put {} in place: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn shared_with_others(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o077 != 0
}

#[cfg(not(unix))]
fn shared_with_others(_metadata: &std::fs::Metadata) -> bool {
    false
}

fn read_hash_key(path: &std::path::Path) -> Result<[u8; 32]> {
    let bytes = std::fs::read(path)?;
    <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
        Error::Config(format!(
            "{} is {} bytes, not the 32-byte arguments key; restore it or remove it on purpose to start a new ledger chain",
            path.display(),
            bytes.len()
        ))
    })
}

#[cfg(unix)]
fn lock_down_key_dir(dir: &std::path::Path) -> Result<()> {
    if dir.starts_with(crate::config::default_data_dir()) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
            Error::Config(format!(
                "cannot restrict {} to its owner: {error}",
                dir.display()
            ))
        })
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn lock_down_key_dir(_dir: &std::path::Path) -> Result<()> {
    Ok(())
}

fn sync_directory(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        if let Ok(handle) = std::fs::File::open(dir) {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = dir;
}

pub async fn call(
    ctx: &ToolContext,
    tool_name: &str,
    arguments: Option<&Map<String, Value>>,
    cancellation: &CancellationToken,
) -> CallToolResult {
    match dispatch(ctx, tool_name, arguments, cancellation).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                tool = %tool_name.chars().filter(|character| !character.is_control()).take(64).collect::<String>(),
                code = error.code(),
                causes = ?error.causes(),
                "tool call failed"
            );
            CallToolResult::structured_error(error.to_json())
        }
    }
}

async fn dispatch(
    ctx: &ToolContext,
    tool_name: &str,
    arguments: Option<&Map<String, Value>>,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let spec = crate::tools::specs::SPECS
        .iter()
        .find(|spec| spec.name == tool_name);
    if let Some(spec) = spec
        && let Some(switch) = crate::tools::specs::switch(spec)
        && !crate::tools::specs::enabled(spec, &ctx.settings)
    {
        return Err(Error::ToolDisabled {
            name: tool_name.to_owned(),
            switch,
        });
    }
    if spec
        .and_then(|spec| spec.endpoint)
        .is_some_and(|endpoint| endpoint.requires_key())
    {
        ctx.settings.require_api_key()?;
    }
    match tool_name {
        "serper_search" => {
            search_from_args(ctx, Endpoint::Search, parse(arguments)?, cancellation).await
        }
        "serper_images" => {
            search_from_args(ctx, Endpoint::Images, parse(arguments)?, cancellation).await
        }
        "serper_news" => {
            search_from_args(ctx, Endpoint::News, parse(arguments)?, cancellation).await
        }
        "serper_places" => {
            search_from_args(ctx, Endpoint::Places, parse(arguments)?, cancellation).await
        }
        "serper_maps" => {
            search_from_args(ctx, Endpoint::Maps, parse(arguments)?, cancellation).await
        }
        "serper_videos" => {
            search_from_args(ctx, Endpoint::Videos, parse(arguments)?, cancellation).await
        }
        "serper_shopping" => {
            search_from_args(ctx, Endpoint::Shopping, parse(arguments)?, cancellation).await
        }
        "serper_scholar" => {
            search_from_args(ctx, Endpoint::Scholar, parse(arguments)?, cancellation).await
        }
        "serper_patents" => {
            search_from_args(ctx, Endpoint::Patents, parse(arguments)?, cancellation).await
        }
        "serper_autocomplete" => {
            search_from_args(ctx, Endpoint::Autocomplete, parse(arguments)?, cancellation).await
        }
        "serper_reviews" => reviews(ctx, parse(arguments)?, cancellation).await,
        "serper_lens" => lens(ctx, parse(arguments)?, cancellation).await,
        "serper_scrape" => scrape(ctx, parse(arguments)?, cancellation).await,
        "serper_locations" => locations(ctx, parse(arguments)?, cancellation).await,
        "serper_resolve" => resolve(ctx, parse(arguments)?, cancellation).await,
        "serper_search_batch" => {
            reject_oversized_queries(arguments)?;
            batch(ctx, parse(arguments)?, cancellation).await
        }
        "serper_health" => health(ctx, parse(arguments)?).await,
        "serper_session" => session_totals(ctx, parse(arguments)?).await,
        other => Err(Error::UnknownTool {
            name: other.chars().filter(|c| !c.is_control()).take(64).collect(),
        }),
    }
}

fn parse<T: DeserializeOwned>(arguments: Option<&Map<String, Value>>) -> Result<T> {
    let object = match arguments {
        Some(map) => Value::Object(map.clone()),
        None => Value::Object(Map::new()),
    };
    serde_json::from_value(object).map_err(|error| Error::InvalidArgs(error.to_string()))
}

fn reject_oversized_queries(arguments: Option<&Map<String, Value>>) -> Result<()> {
    let Some(queries) = arguments
        .and_then(|map| map.get("queries"))
        .and_then(Value::as_array)
    else {
        return Ok(());
    };
    if queries.len() > MAX_BATCH_QUERIES {
        return Err(Error::InvalidArgs(format!(
            "queries holds {} items, above the {MAX_BATCH_QUERIES} limit",
            queries.len()
        )));
    }
    Ok(())
}

async fn search_from_args(
    ctx: &ToolContext,
    endpoint: Endpoint,
    args: SearchArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let resolve_links = args
        .resolve_links
        .unwrap_or_else(|| ctx.settings.auto_resolve_links());
    let mut body = args.body(endpoint, ctx.settings.default_freshness())?;
    if resolve_links && let Some(object) = body.as_object_mut() {
        object.insert("resolveLinks".to_owned(), Value::Bool(false));
    }
    let credits = endpoint.estimated_cost(args.num).get();
    let cacheable = args.tbs.is_none();
    search_and_record(
        ctx,
        endpoint,
        body,
        args.preset(),
        args.text_mode(),
        credits,
        cacheable,
        resolve_links,
        cancellation,
    )
    .await
}

async fn resolve_in_place(ctx: &ToolContext, raw: &mut Value, cancellation: &CancellationToken) {
    if !projection::holds_goto_link(raw) {
        return;
    }
    match ctx.client.post(Endpoint::Resolve, raw, cancellation).await {
        Ok(resolved) => {
            if projection::adopt_resolved_links(raw, &resolved) == 0 {
                tracing::warn!("the response held a Google link that resolve did not replace");
            }
        }
        Err(error) => {
            tracing::warn!(%error, "the redirect links could not be resolved before answering");
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn search_and_record(
    ctx: &ToolContext,
    endpoint: Endpoint,
    body: Value,
    preset: Preset,
    mode: TextMode,
    credits: u32,
    cacheable: bool,
    resolve_links: bool,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let started = Instant::now();
    let key = cache_key(endpoint.tool_name(), ctx.hash_key.as_ref(), &body);
    let cached = if cacheable {
        ctx.cache.get(&key)?
    } else {
        None
    };
    let mut _reservation: Option<Reservation> = None;
    let (raw, cache_age) = match cached {
        Some((raw, age)) => (raw, Some(age)),
        None => {
            _reservation = reserve_budget(ctx, credits).await?;
            let mut raw = match ctx.client.post(endpoint, &body, cancellation).await {
                Ok(raw) => raw,
                Err(error) => {
                    append_failure(
                        ctx,
                        endpoint.tool_name(),
                        endpoint.path(),
                        &body,
                        credits,
                        started,
                        &error,
                    )
                    .await;
                    return Err(error);
                }
            };
            if resolve_links {
                resolve_in_place(ctx, &mut raw, cancellation).await;
            }
            (Arc::new(raw), None)
        }
    };
    let reported_credits = raw
        .get("credits")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok());
    let charged = if cache_age.is_some() {
        0
    } else {
        reported_credits.unwrap_or(credits)
    };
    if cacheable && cache_age.is_none() {
        let fetched = Arc::clone(&raw);
        if let Err(error) = ctx.cache.put(key, fetched) {
            tracing::warn!(%error, "the response could not be cached");
        }
    }
    let prepared: Result<(Value, usize, Option<String>, Option<String>)> = (|| {
        let projection = project(endpoint, &raw, preset)?;
        Ok((
            projection.envelope,
            projection.result_count,
            projection.top_title,
            projection.next_token,
        ))
    })();
    let (envelope, result_count, top_title, next_token) = match prepared {
        Ok(parts) => parts,
        Err(error) => {
            append_call_or_log(
                ctx,
                CallRecord {
                    tool: endpoint.tool_name().to_owned(),
                    endpoint: endpoint.path().to_owned(),
                    args_hash: args_hash(ctx.hash_key.as_ref(), &body),
                    credits_estimated: if cache_age.is_some() { 0 } else { credits },
                    credits_actual: if cache_age.is_some() {
                        None
                    } else {
                        reported_credits
                    },
                    batch_multiplier: 1,
                    duration_ms: elapsed_ms(started),
                    status: format!("error:{}", error.code()),
                    response_bytes: 0,
                    tokens: 0,
                    tokens_saved: 0,
                    cache_hit: cache_age.is_some(),
                },
                _reservation.take(),
            )
            .await;
            return Err(error);
        }
    };
    let baseline = serde_json::to_vec(&raw)?.len();
    let fitted = projection::fit_to_budget(envelope, projection::MAX_RESULT_BYTES);
    let emitted = serde_json::to_vec(&fitted)?.len();
    let tokens = (emitted / 4) as u64;
    let saved = ((baseline / 4) as u64).saturating_sub(tokens);
    append_call_or_log(
        ctx,
        CallRecord {
            tool: endpoint.tool_name().to_owned(),
            endpoint: endpoint.path().to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &body),
            credits_estimated: if cache_age.is_some() { 0 } else { credits },
            credits_actual: if cache_age.is_some() {
                None
            } else {
                reported_credits
            },
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: if cache_age.is_some() { "cache" } else { "ok" }.to_owned(),
            response_bytes: emitted as u64,
            tokens,
            tokens_saved: saved,
            cache_hit: cache_age.is_some(),
        },
        _reservation.take(),
    )
    .await;
    let cost = cost_info(ctx, charged, cache_age);
    let mut fitted = fitted;
    set_cost(&mut fitted, &cost)?;
    let fitted = projection::fit_to_budget(fitted, projection::MAX_RESULT_BYTES);
    let delivered = fitted
        .get("results")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(result_count);
    let top = first_title(&fitted).or(top_title);
    let clipped = fitted
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = match mode {
        TextMode::Full => serde_json::to_string(&fitted)?,
        TextMode::Summary => {
            let mut line = projection::summary_text(delivered, top.as_deref(), charged, cost.usd);
            if clipped {
                line.push_str(
                    "; clipped to fit the response budget: lower num, narrow the query, or use fields=minimal to see the rest",
                );
            }
            if let Some(token) = next_token.as_deref() {
                let token = crate::projection::cap(&crate::projection::sanitize(token), 64).0;
                line.push_str(&format!("; next page token: {token}"));
            }
            if cache_age.is_some() {
                line.push_str("; served from cache");
            }
            line
        }
    };
    Ok(tool_result(fitted, text))
}

struct Projection {
    envelope: Value,
    result_count: usize,
    top_title: Option<String>,
    next_token: Option<String>,
}

fn project(endpoint: Endpoint, raw: &Value, preset: Preset) -> Result<Projection> {
    let sections = projection::search_sections(raw, preset);
    let mut truncated = sections.truncated;
    let (envelope, result_count, top_title) = match endpoint {
        Endpoint::Search => {
            let (items, cut) = projection::organic(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = with_sections(Envelope::new(items, placeholder_cost()), sections)?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Images => {
            let (items, cut) = projection::images(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::News => {
            let (items, cut) = projection::news(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Places | Endpoint::Maps => {
            let (items, cut) = projection::places(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Videos => {
            let (items, cut) = projection::videos(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Shopping => {
            let (items, cut) = projection::shopping(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Scholar => {
            let (items, cut) = projection::scholar(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Patents => {
            let (items, cut) = projection::patents(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Autocomplete => {
            let items = projection::suggestions(raw);
            let top = items.first().map(|item| item.value.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Reviews => {
            let (items, cut) = projection::reviews(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Lens => {
            let (items, cut) = projection::lens(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Locations => {
            let items = projection::locations(raw);
            let top = items.first().and_then(|item| item.canonical_name.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let count = result_count(&envelope);
                (envelope, count, top)
            }
        }
        Endpoint::Scrape | Endpoint::Resolve => {
            return Err(Error::InvalidArgs(format!(
                "{} is not a search family endpoint",
                endpoint.tool_name()
            )));
        }
    };
    let next_token = raw
        .get("nextPageToken")
        .and_then(Value::as_str)
        .map(crate::projection::sanitize);
    let mut envelope = envelope;
    if let Some(token) = &next_token
        && let Some(object) = envelope.as_object_mut()
    {
        object.insert("nextPageToken".to_owned(), Value::String(token.clone()));
    }
    if truncated && let Some(object) = envelope.as_object_mut() {
        object.insert("truncated".to_owned(), Value::Bool(true));
    }
    Ok(Projection {
        envelope,
        result_count,
        top_title,
        next_token,
    })
}

fn with_sections<T: Serialize>(envelope: Envelope<T>, sections: SearchSections) -> Result<Value> {
    let mut envelope = envelope;
    envelope.knowledge_graph = sections.knowledge_graph;
    envelope.answer_box = sections.answer_box;
    envelope.people_also_ask = sections.people_also_ask;
    envelope.related_searches = sections.related_searches;
    envelope.search_parameters = sections.search_parameters;
    envelope.truncated |= sections.truncated;
    Ok(serde_json::to_value(envelope)?)
}

fn result_count(envelope: &Value) -> usize {
    envelope
        .get("results")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

fn first_title(envelope: &Value) -> Option<String> {
    envelope
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| results.first())
        .and_then(|first| first.get("title"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn placeholder_cost() -> CostInfo {
    CostInfo {
        credits: 0,
        usd: None,
        session_credits: 0,
        session_usd: None,
        cache_hit: None,
        cached_age_secs: None,
    }
}

fn set_cost(envelope: &mut Value, cost: &CostInfo) -> Result<()> {
    let object = envelope
        .as_object_mut()
        .ok_or_else(|| Error::Projection("envelope is not a JSON object".into()))?;
    object.insert("cost".to_owned(), serde_json::to_value(cost)?);
    Ok(())
}

async fn reviews(
    ctx: &ToolContext,
    args: ReviewsArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let body = args.body()?;
    let credits = Endpoint::Reviews.base_cost().get();
    search_and_record(
        ctx,
        Endpoint::Reviews,
        body,
        args.preset(),
        args.text_mode(),
        credits,
        false,
        false,
        cancellation,
    )
    .await
}

async fn lens(
    ctx: &ToolContext,
    args: LensArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let body = args.body()?;
    let credits = Endpoint::Lens.base_cost().get();
    search_and_record(
        ctx,
        Endpoint::Lens,
        body,
        args.preset(),
        args.text_mode(),
        credits,
        false,
        false,
        cancellation,
    )
    .await
}

async fn scrape(
    ctx: &ToolContext,
    args: ScrapeArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let body = args.body()?;
    let attempt = crate::cost::SCRAPE_RESERVATION_CREDITS;
    let mut _reservation = reserve_budget(ctx, attempt).await?;
    let started = Instant::now();
    let raw = match ctx.client.post(Endpoint::Scrape, &body, cancellation).await {
        Ok(raw) => raw,
        Err(error) => {
            append_failure(
                ctx,
                "serper_scrape",
                Endpoint::Scrape.path(),
                &body,
                attempt,
                started,
                &error,
            )
            .await;
            return Err(error);
        }
    };
    let preset = args.preset();
    let content = projection::scrape_content(&raw, preset);
    let actual = content.credits;
    let output = ScrapeOutput {
        markdown: content.markdown,
        text: content.text,
        title: content.title,
        credits: content.credits,
        images: content.images,
        links: content.links,
        videos: content.videos,
        truncated: content.truncated,
        cost: placeholder_cost(),
    };
    let mut payload = serde_json::to_value(&output)?;
    payload = projection::fit_to_budget(payload, projection::MAX_RESULT_BYTES);
    let serialized = serde_json::to_vec(&payload)?;
    append_call_or_log(
        ctx,
        CallRecord {
            tool: "serper_scrape".to_owned(),
            endpoint: Endpoint::Scrape.path().to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &body),
            credits_estimated: actual.unwrap_or(attempt),
            credits_actual: actual,
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serialized.len() as u64,
            tokens: (serialized.len() / 4) as u64,
            tokens_saved: 0,
            cache_hit: false,
        },
        _reservation.take(),
    )
    .await;
    let cost = cost_info(ctx, actual.unwrap_or(attempt), None);
    set_cost(&mut payload, &cost)?;
    let payload = projection::fit_to_budget(payload, projection::MAX_RESULT_BYTES);
    let text = match args.text_mode() {
        TextMode::Full => serde_json::to_string(&payload)?,
        TextMode::Summary => {
            let chars = output
                .markdown
                .as_ref()
                .map_or(0, |text| text.chars().count());
            format!(
                "{chars} markdown characters; cost: {} credits ({})",
                cost.credits,
                usd_text(cost.usd)
            )
        }
    };
    Ok(tool_result(payload, text))
}

async fn locations(
    ctx: &ToolContext,
    args: LocationsArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let query = args.query()?;
    let started = Instant::now();
    let raw = ctx
        .client
        .get(Endpoint::Locations, &query, cancellation)
        .await?;
    let items = projection::locations(&raw);
    let top = items.first().and_then(|item| item.canonical_name.clone());
    let mut envelope = serde_json::to_value(Envelope::new(items, placeholder_cost()))?;
    append_call_or_log(
        ctx,
        CallRecord {
            tool: "serper_locations".to_owned(),
            endpoint: Endpoint::Locations.path().to_owned(),
            args_hash: args_hash(
                ctx.hash_key.as_ref(),
                &Value::Array(
                    query
                        .iter()
                        .map(|(key, value)| Value::String(format!("{key}={value}")))
                        .collect(),
                ),
            ),
            credits_estimated: 0,
            credits_actual: Some(0),
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serde_json::to_vec(&envelope)?.len() as u64,
            tokens: 0,
            tokens_saved: 0,
            cache_hit: false,
        },
        None,
    )
    .await;
    let cost = cost_info(ctx, 0, None);
    set_cost(&mut envelope, &cost)?;
    let envelope = projection::fit_to_budget(envelope, projection::MAX_RESULT_BYTES);
    let text = projection::summary_text(result_count(&envelope), top.as_deref(), 0, Some(0.0));
    Ok(tool_result(envelope, text))
}

async fn resolve(
    ctx: &ToolContext,
    args: ResolveArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let body = args.body()?;
    let started = Instant::now();
    let raw = ctx
        .client
        .post(Endpoint::Resolve, &body, cancellation)
        .await?;
    let (items, retry_after) = projection::resolved_links(&raw, &args.links);
    let count = items.len();
    let top = items.iter().find_map(|item| item.resolved.clone());
    let output = ResolveOutput {
        results: items,
        retry_after,
        cost: placeholder_cost(),
        truncated: false,
    };
    let mut envelope = serde_json::to_value(output)?;
    append_call_or_log(
        ctx,
        CallRecord {
            tool: "serper_resolve".to_owned(),
            endpoint: Endpoint::Resolve.path().to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &body),
            credits_estimated: 0,
            credits_actual: Some(0),
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serde_json::to_vec(&envelope)?.len() as u64,
            tokens: 0,
            tokens_saved: 0,
            cache_hit: false,
        },
        None,
    )
    .await;
    let cost = cost_info(ctx, 0, None);
    set_cost(&mut envelope, &cost)?;
    let envelope = projection::fit_to_budget(envelope, projection::MAX_RESULT_BYTES);
    let mut text = projection::summary_text(count, top.as_deref(), 0, Some(0.0));
    if let Some(seconds) = retry_after {
        text.push_str(&format!("; retry after {seconds}s"));
    }
    Ok(tool_result(envelope, text))
}

async fn batch(
    ctx: &ToolContext,
    args: BatchArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let endpoint = args.kind.endpoint();
    let bodies = args.bodies(ctx.settings.default_freshness())?;
    let total = args
        .queries
        .iter()
        .map(|query| endpoint.estimated_cost(query.num).get())
        .sum::<u32>()
        .saturating_mul(BATCH_MULTIPLIER);
    let mut _reservation = reserve_budget(ctx, total).await?;
    let body = Value::Array(bodies);
    let started = Instant::now();
    let raw = match ctx.client.post_many(endpoint, &body, cancellation).await {
        Ok(raw) => raw,
        Err(error) => {
            append_failure(
                ctx,
                "serper_search_batch",
                endpoint.path(),
                &body,
                total,
                started,
                &error,
            )
            .await;
            return Err(error);
        }
    };
    let Some(responses) = raw.as_array() else {
        let error = Error::Upstream {
            status: 200,
            message: "the batch response was not an array".to_owned(),
        };
        append_failure(
            ctx,
            "serper_search_batch",
            endpoint.path(),
            &body,
            total,
            started,
            &error,
        )
        .await;
        return Err(error);
    };
    let preset = args.preset();
    let entries = responses
        .iter()
        .map(|query_response| {
            let query = query_response
                .get("searchParameters")
                .and_then(|params| params.get("q"))
                .and_then(Value::as_str)
                .map(crate::projection::sanitize);
            let (results, error) = match project(endpoint, query_response, preset) {
                Ok(projection) => (
                    projection
                        .envelope
                        .get("results")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                    None,
                ),
                Err(error) => (
                    Vec::new(),
                    Some(crate::projection::cap(&error.to_string(), 300).0),
                ),
            };
            let error = error.or_else(|| {
                query_response
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|message| {
                        crate::projection::cap(&crate::projection::sanitize(message), 300).0
                    })
                    .or_else(|| {
                        query_response
                            .get("statusCode")
                            .map(|code| format!("upstream answered {code}"))
                    })
            });
            BatchEntry {
                query,
                results,
                error,
            }
        })
        .collect::<Vec<_>>();
    let output = BatchOutput {
        entries,
        cost: placeholder_cost(),
        truncated: false,
    };
    let mut payload = serde_json::to_value(&output)?;
    payload = projection::fit_to_budget(payload, projection::MAX_RESULT_BYTES);
    let serialized = serde_json::to_vec(&payload)?;
    append_call_or_log(
        ctx,
        CallRecord {
            tool: "serper_search_batch".to_owned(),
            endpoint: endpoint.path().to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &body),
            credits_estimated: total,
            credits_actual: None,
            batch_multiplier: BATCH_MULTIPLIER,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serialized.len() as u64,
            tokens: (serialized.len() / 4) as u64,
            tokens_saved: 0,
            cache_hit: false,
        },
        _reservation.take(),
    )
    .await;
    let cost = cost_info(ctx, total, None);
    set_cost(&mut payload, &cost)?;
    let payload = projection::fit_to_budget(payload, projection::MAX_RESULT_BYTES);
    let total_queries = output.entries.len();
    let delivered_queries = payload
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(total_queries);
    let text = match args.text_mode() {
        TextMode::Full => serde_json::to_string(&payload)?,
        TextMode::Summary => {
            let mut line = format!(
                "{} queries; cost: {} credits ({})",
                delivered_queries,
                cost.credits,
                usd_text(cost.usd)
            );
            if delivered_queries < total_queries {
                line.push_str(&format!(
                    "; {} queries were left out to fit the response budget: re-run them in a smaller batch",
                    total_queries - delivered_queries
                ));
            }
            line
        }
    };
    Ok(tool_result(payload, text))
}

async fn health(ctx: &ToolContext, _args: HealthArgs) -> Result<CallToolResult> {
    let started = Instant::now();
    let health_response = ctx.client.health().await.ok_or_else(|| Error::Upstream {
        status: 503,
        message: "api.serper.dev/health did not answer".to_owned(),
    })?;
    let status = health_response
        .get("status")
        .and_then(Value::as_str)
        .map(|raw| crate::projection::cap(&crate::projection::sanitize(raw), 64).0);
    let service_info = health_response
        .get("info")
        .map(crate::projection::sanitize_value);
    append_call_or_log(
        ctx,
        CallRecord {
            tool: "serper_health".to_owned(),
            endpoint: "/health".to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &Value::Object(Map::new())),
            credits_estimated: 0,
            credits_actual: Some(0),
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serde_json::to_vec(&health_response)?.len() as u64,
            tokens: 0,
            tokens_saved: 0,
            cache_hit: false,
        },
        None,
    )
    .await;
    let output = HealthOutput {
        status: status.clone(),
        info: service_info,
        cost: cost_info(ctx, 0, None),
    };
    let payload = serde_json::to_value(&output)?;
    let payload = projection::fit_to_budget(payload, projection::MAX_RESULT_BYTES);
    let text = format!("service status: {}", status.as_deref().unwrap_or("unknown"));
    Ok(tool_result(payload, text))
}

async fn session_totals(ctx: &ToolContext, args: SessionArgs) -> Result<CallToolResult> {
    let ledger = ctx.ledger.clone();
    let (totals, pending, stale) = match tokio::task::spawn_blocking(move || {
        (ledger.totals(), ledger.pending())
    })
    .await
    {
        Ok((totals, pending)) => {
            let stale = totals.is_err() || pending.is_err();
            if let Err(error) = &totals {
                tracing::warn!(
                    %error,
                    "the ledger totals could not be caught up; reporting the last known snapshot"
                );
            }
            if let Err(error) = &pending {
                tracing::warn!(%error, "the pending credits could not be read");
            }
            (
                totals.unwrap_or_else(|_| ctx.ledger.snapshot()),
                pending.unwrap_or(0),
                stale,
            )
        }
        Err(join_error) => {
            tracing::warn!(
                %join_error,
                "the ledger task failed; reporting the last known snapshot"
            );
            (ctx.ledger.snapshot(), 0, true)
        }
    };
    let per_tool = totals
        .per_tool
        .iter()
        .map(|(tool, entry)| ToolUsage {
            tool: crate::projection::cap(&crate::projection::sanitize(tool), 64).0,
            calls: entry.calls,
            credits: entry.credits,
            tokens: entry.tokens,
            cache_hits: entry.cache_hits,
        })
        .collect();
    let (live_balance, live_balance_error) = if args.live.unwrap_or(false) {
        match ctx.client.account_probe().await {
            None => (
                None,
                Some("not read: no API key is attached to this client".to_owned()),
            ),
            Some((_, Some(credits))) => (Some(credits), None),
            Some((status, None)) if !(200..300).contains(&status) => (
                None,
                Some(format!("not read: the account probe answered {status}")),
            ),
            Some((_, None)) => (
                None,
                Some("not read: the account probe carried no credit balance".to_owned()),
            ),
        }
    } else {
        (None, None)
    };
    let budget_remaining = ctx.settings.session_budget_credits().map(|budget| {
        budget.saturating_sub(
            totals
                .credits
                .saturating_add(pending)
                .saturating_add(ctx.ledger.queued_credits()),
        )
    });
    let lost = ctx.ledger.lost();
    let output = SessionOutput {
        calls: totals.calls,
        credits: totals.credits,
        usd: usd_for_credits(
            totals.credits,
            ctx.settings.pack(),
            ctx.settings.usd_per_1k(),
        ),
        tokens: totals.tokens,
        tokens_saved: totals.tokens_saved,
        cache_hits: totals.cache_hits,
        per_tool,
        budget_credits: ctx.settings.session_budget_credits(),
        budget_remaining,
        live_balance,
        live_balance_error,
        ledger: Some(crate::response::LedgerHealth {
            healthy: ctx.ledger.healthy(),
            queued: ctx.ledger.queued(),
            lost,
            credits_stale: stale,
        }),
    };
    let payload = serde_json::to_value(&output)?;
    let mut text = format!(
        "{} calls; {} credits ({}); {} tokens, {} saved",
        output.calls,
        output.credits,
        usd_text(output.usd),
        output.tokens,
        output.tokens_saved
    );
    if stale {
        text.push_str("; the ledger totals are the last known snapshot");
    }
    if lost > 0 {
        text.push_str(&format!("; {lost} earlier calls could not be recorded"));
    }
    Ok(tool_result(payload, text))
}

async fn reserve_budget(ctx: &ToolContext, credits: u32) -> Result<Option<Reservation>> {
    if let Some(ceiling) = ctx.settings.per_call_credit_ceiling()
        && credits > ceiling
    {
        return Err(Error::BudgetRefused(format!(
            "{credits} credits is above the per-call ceiling of {ceiling}"
        )));
    }
    let Some(budget) = ctx.settings.session_budget_credits() else {
        return Ok(None);
    };
    let ledger = ctx.ledger.clone();
    let reservation = tokio::task::spawn_blocking(move || ledger.reserve(credits, budget))
        .await
        .map_err(|error| Error::Ledger(error.to_string()))??;
    Ok(Some(reservation))
}

fn usd_text(usd: Option<f64>) -> String {
    usd.map_or_else(|| "usd unknown".to_owned(), |value| format!("${value:.6}"))
}

fn cost_info(ctx: &ToolContext, credits: u32, cache_age: Option<u64>) -> CostInfo {
    let totals = ctx.ledger.snapshot();
    CostInfo {
        credits,
        usd: usd_for_call(credits, ctx.settings.pack(), ctx.settings.usd_per_1k()),
        session_credits: totals.credits,
        session_usd: usd_for_credits(
            totals.credits,
            ctx.settings.pack(),
            ctx.settings.usd_per_1k(),
        ),
        cache_hit: cache_age.map(|_| true),
        cached_age_secs: cache_age,
    }
}

async fn append_call_record(
    ctx: &ToolContext,
    call_record: CallRecord,
    reservation: Option<Reservation>,
) -> Result<()> {
    tracing::debug!(
        tool = %call_record.tool,
        credits = call_record.credits_estimated,
        cache_hit = call_record.cache_hit,
        status = %call_record.status,
        "tool call"
    );
    let reserved = reservation.as_ref().map_or(0, Reservation::credits);
    let ledger = ctx.ledger.clone();
    let joined = tokio::task::spawn_blocking(move || ledger.append_reserved(call_record, reserved))
        .await
        .map_err(|error| Error::Ledger(error.to_string()));
    match joined {
        Ok(Ok(_)) => {
            if let Some(reservation) = &reservation {
                reservation.disarm();
            }
            Ok(())
        }
        Ok(Err(error)) => Err(error),
        Err(error) => Err(error),
    }
}

async fn append_call_or_log(
    ctx: &ToolContext,
    call_record: CallRecord,
    reservation: Option<Reservation>,
) {
    if let Err(error) = append_call_record(ctx, call_record, reservation).await {
        tracing::error!(%error, "a completed call could not be written to the ledger");
    }
}

async fn append_failure(
    ctx: &ToolContext,
    tool: &str,
    endpoint: &str,
    body: &Value,
    credits: u32,
    started: Instant,
    error: &Error,
) {
    let call_record = CallRecord {
        tool: tool.to_owned(),
        endpoint: endpoint.to_owned(),
        args_hash: args_hash(ctx.hash_key.as_ref(), body),
        credits_estimated: credits,
        // A rejected or rate-limited call was never billed; a timeout or 5xx may have been.
        credits_actual: if error.is_unbilled() { Some(0) } else { None },
        batch_multiplier: 1,
        duration_ms: elapsed_ms(started),
        status: format!("error:{}", error.code()),
        response_bytes: 0,
        tokens: 0,
        tokens_saved: 0,
        cache_hit: false,
    };
    let ledger = ctx.ledger.clone();
    match tokio::task::spawn_blocking(move || ledger.append(call_record)).await {
        Ok(Ok(_)) => {}
        Ok(Err(ledger_error)) => {
            tracing::warn!(error = %ledger_error, "failed to record a failed call");
        }
        Err(join_error) => {
            tracing::warn!(error = %join_error, "ledger task failed");
        }
    }
}

fn args_hash(key: &[u8], body: &Value) -> String {
    let bytes = serde_json::to_vec(body).unwrap_or_default();
    match Hmac::<Sha256>::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(&bytes);
            hex::encode(mac.finalize().into_bytes())
        }
        Err(_) => hex::encode(Sha256::digest(bytes)),
    }
}

fn cache_key(tool: &str, key: &[u8], body: &Value) -> String {
    format!("{tool}:{}", args_hash(key, body))
}

fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn tool_result(structured: Value, text: String) -> CallToolResult {
    let mut result = CallToolResult::structured(structured);
    result.content = vec![ContentBlock::text(text)];
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_reads_the_delivered_envelope() {
        let envelope = serde_json::json!({
            "results": [{"title": "first"}, {"title": "second"}],
        });
        assert_eq!(result_count(&envelope), 2);
        assert_eq!(first_title(&envelope).as_deref(), Some("first"));
        let trimmed = serde_json::json!({"results": [], "truncated": true});
        assert_eq!(result_count(&trimmed), 0);
        assert_eq!(first_title(&trimmed), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_held_ledger_lock_does_not_stall_the_async_thread() {
        let dir = std::env::temp_dir().join(format!("ownserp-reserve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let profile = dir.join("profile.toml");
        std::fs::write(&profile, "session_budget_credits = 100\n").expect("profile");
        let settings = Settings::load(crate::config::Overrides {
            profile: Some(profile),
            ledger_dir: Some(dir.clone()),
            cache_ttl_secs: Some(0),
            ..crate::config::Overrides::default()
        })
        .expect("settings");
        let context = ToolContext::new(settings).expect("context");
        let path = context.ledger.path().expect("path");
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = holder.unlock();
        });

        let ticks = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let counter = Arc::clone(&ticks);
        let ticker = tokio::spawn(async move {
            for _ in 0..40 {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        });

        let reservation = reserve_budget(&context, 1).await.expect("reservation");
        let observed = ticks.load(std::sync::atomic::Ordering::Relaxed);
        assert!(reservation.is_some(), "a budget of 100 admits one credit");
        assert!(
            observed >= 5,
            "the async thread stalled while the file lock was held: {observed} ticks"
        );
        ticker.abort();
        let _ = releaser.join();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrent_starts_agree_on_one_arguments_key() {
        let root = std::env::temp_dir().join(format!("ownserp-args-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ledger_dir = root.join("ledger");
        std::fs::create_dir_all(&ledger_dir).expect("ledger dir");
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let ledger_dir = ledger_dir.clone();
                std::thread::spawn(move || load_hash_key(&ledger_dir))
            })
            .collect();
        let keys: Vec<[u8; 32]> = workers
            .into_iter()
            .map(|worker| worker.join().expect("join").expect("key"))
            .collect();
        assert!(
            keys.windows(2).all(|pair| pair[0] == pair[1]),
            "every starter must end with the same key"
        );
        let on_disk = std::fs::read(root.join("args.key")).expect("key file");
        assert_eq!(on_disk, keys[0].to_vec());
        let _ = std::fs::remove_dir_all(&root);
    }
}
