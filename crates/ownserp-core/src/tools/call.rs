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
use crate::cost::{BATCH_MULTIPLIER, Endpoint, usd_for, usd_for_credits};
use crate::error::{Error, Result};
use crate::ledger::{CallRecord, Ledger, Reservation};
use crate::projection::{self, SearchSections};
use crate::response::{
    BatchEntry, BatchOutput, CostInfo, Envelope, HealthOutput, ListOutput, ScrapeOutput,
    SessionOutput, ToolUsage,
};
use crate::tools::args::{
    BatchArgs, HealthArgs, LensArgs, LocationsArgs, MAX_BATCH_QUERIES, ReviewsArgs, ScrapeArgs,
    SearchArgs, SessionArgs,
};
use crate::types::{Preset, TextMode};

const SCRAPE_MAX_CREDITS: u32 = 10;

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
        Ok(_) => {
            let bytes = std::fs::read(&path)?;
            return <[u8; 32]>::try_from(bytes.as_slice()).map_err(|_| {
                Error::Config(format!(
                    "{} is {} bytes, not the 32-byte arguments key; restore it or remove it on purpose to start a new ledger chain",
                    path.display(),
                    bytes.len()
                ))
            });
        }
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
    std::fs::create_dir_all(dir)?;
    let temporary = dir.join("args.key.tmp");
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
    let mut file = options.open(&temporary).or_else(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            std::fs::remove_file(&temporary).and_then(|()| options.open(&temporary))
        } else {
            Err(error)
        }
    })?;
    file.write_all(&key)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, &path)?;
    temporary_guard.armed = false;
    sync_directory(dir);
    Ok(key)
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
    name: &str,
    arguments: Option<&Map<String, Value>>,
    cancellation: &CancellationToken,
) -> CallToolResult {
    match dispatch(ctx, name, arguments, cancellation).await {
        Ok(result) => result,
        Err(error) => {
            tracing::warn!(
                tool = %name.chars().filter(|ch| !ch.is_control()).take(64).collect::<String>(),
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
    name: &str,
    arguments: Option<&Map<String, Value>>,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    match name {
        "serper_search" => {
            execute_search_endpoint(ctx, Endpoint::Search, parse(arguments)?, cancellation).await
        }
        "serper_images" => {
            execute_search_endpoint(ctx, Endpoint::Images, parse(arguments)?, cancellation).await
        }
        "serper_news" => {
            execute_search_endpoint(ctx, Endpoint::News, parse(arguments)?, cancellation).await
        }
        "serper_places" => {
            execute_search_endpoint(ctx, Endpoint::Places, parse(arguments)?, cancellation).await
        }
        "serper_maps" => {
            execute_search_endpoint(ctx, Endpoint::Maps, parse(arguments)?, cancellation).await
        }
        "serper_videos" => {
            execute_search_endpoint(ctx, Endpoint::Videos, parse(arguments)?, cancellation).await
        }
        "serper_shopping" => {
            execute_search_endpoint(ctx, Endpoint::Shopping, parse(arguments)?, cancellation).await
        }
        "serper_scholar" => {
            execute_search_endpoint(ctx, Endpoint::Scholar, parse(arguments)?, cancellation).await
        }
        "serper_patents" => {
            execute_search_endpoint(ctx, Endpoint::Patents, parse(arguments)?, cancellation).await
        }
        "serper_autocomplete" => {
            execute_search_endpoint(ctx, Endpoint::Autocomplete, parse(arguments)?, cancellation)
                .await
        }
        "serper_reviews" => reviews(ctx, parse(arguments)?, cancellation).await,
        "serper_lens" => lens(ctx, parse(arguments)?, cancellation).await,
        "serper_scrape" => scrape(ctx, parse(arguments)?, cancellation).await,
        "serper_locations" => locations(ctx, parse(arguments)?, cancellation).await,
        "serper_search_batch" => {
            reject_oversized_queries(arguments)?;
            batch(ctx, parse(arguments)?, cancellation).await
        }
        "serper_health" => health(ctx, parse(arguments)?).await,
        "serper_session" => session_totals(ctx, parse(arguments)?).await,
        other => Err(Error::InvalidArgs(format!("unknown tool: {other}"))),
    }
}

fn parse<T: DeserializeOwned>(arguments: Option<&Map<String, Value>>) -> Result<T> {
    let value = match arguments {
        Some(map) => Value::Object(map.clone()),
        None => Value::Object(Map::new()),
    };
    serde_json::from_value(value).map_err(|error| Error::InvalidArgs(error.to_string()))
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

async fn execute_search_endpoint(
    ctx: &ToolContext,
    endpoint: Endpoint,
    args: SearchArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    let body = args.body(endpoint)?;
    let credits = endpoint.estimated_cost(args.num).get();
    let cacheable = args.tbs.is_none();
    execute_search(
        ctx,
        endpoint,
        body,
        args.preset(),
        args.text_mode(),
        credits,
        cacheable,
        cancellation,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn execute_search(
    ctx: &ToolContext,
    endpoint: Endpoint,
    body: Value,
    preset: Preset,
    mode: TextMode,
    credits: u32,
    cacheable: bool,
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
            _reservation = reserve_budget(ctx, credits)?;
            let raw = match ctx.client.post(endpoint, &body, cancellation).await {
                Ok(raw) => raw,
                Err(error) => {
                    record_failure(
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
    let prepared: Result<(Value, usize, Option<String>, Option<String>)> = (|| {
        let projection = project(endpoint, &raw, preset)?;
        Ok((
            projection.envelope,
            projection.results,
            projection.top_title,
            projection.next_token,
        ))
    })();
    let (envelope, results, top_title, next_token) = match prepared {
        Ok(value) => {
            if cacheable
                && cache_age.is_none()
                && let Err(error) = ctx.cache.put(key, Arc::clone(&raw))
            {
                tracing::warn!(%error, "the response could not be cached");
            }
            value
        }
        Err(error) => {
            record_call(
                ctx,
                CallRecord {
                    tool: endpoint.tool_name().to_owned(),
                    endpoint: endpoint.path().to_owned(),
                    args_hash: args_hash(ctx.hash_key.as_ref(), &body),
                    credits_estimated: credits,
                    credits_actual: reported_credits,
                    batch_multiplier: 1,
                    duration_ms: elapsed_ms(started),
                    status: format!("error:{}", error.code()),
                    response_bytes: 0,
                    tokens: 0,
                    tokens_saved: 0,
                    cache_hit: cache_age.is_some(),
                },
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
    record_call(
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
    )
    .await;
    let cost = cost_info(ctx, charged, cache_age)?;
    let mut fitted = fitted;
    set_cost(&mut fitted, &cost)?;
    let fitted = projection::fit_to_budget(fitted, projection::MAX_RESULT_BYTES);
    let text = match mode {
        TextMode::Full => serde_json::to_string(&fitted)?,
        TextMode::Summary => {
            let mut line =
                projection::summary_text(results, top_title.as_deref(), charged, cost.usd);
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
    results: usize,
    top_title: Option<String>,
    next_token: Option<String>,
}

fn project(endpoint: Endpoint, raw: &Value, preset: Preset) -> Result<Projection> {
    let sections = projection::search_sections(raw, preset);
    let mut truncated = sections.truncated;
    let (envelope, results, top_title) = match endpoint {
        Endpoint::Search => {
            let (items, cut) = projection::organic(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = with_sections(Envelope::new(items, placeholder_cost()), sections)?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Images => {
            let (items, cut) = projection::images(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::News => {
            let (items, cut) = projection::news(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Places | Endpoint::Maps => {
            let (items, cut) = projection::places(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Videos => {
            let (items, cut) = projection::videos(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Shopping => {
            let (items, cut) = projection::shopping(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Scholar => {
            let (items, cut) = projection::scholar(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Patents => {
            let (items, cut) = projection::patents(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Autocomplete => {
            let items = projection::suggestions(raw);
            let top = items.first().map(|item| item.value.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Reviews => {
            let (items, cut) = projection::reviews(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Lens => {
            let (items, cut) = projection::lens(raw, preset);
            truncated |= cut;
            let top = items.first().and_then(|item| item.title.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Locations => {
            let items = projection::locations(raw);
            let top = items.first().and_then(|item| item.canonical_name.clone());
            let envelope = serde_json::to_value(ListOutput::new(items, placeholder_cost()))?;
            {
                let result_count = result_count(&envelope);
                (envelope, result_count, top)
            }
        }
        Endpoint::Scrape => {
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
        results,
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
    execute_search(
        ctx,
        Endpoint::Reviews,
        body,
        args.preset(),
        args.text_mode(),
        credits,
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
    execute_search(
        ctx,
        Endpoint::Lens,
        body,
        args.preset(),
        args.text_mode(),
        credits,
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
    let credits = Endpoint::Scrape.base_cost().get();
    let _reservation = reserve_budget(ctx, SCRAPE_MAX_CREDITS)?;
    let started = Instant::now();
    let raw = match ctx.client.post(Endpoint::Scrape, &body, cancellation).await {
        Ok(raw) => raw,
        Err(error) => {
            record_failure(
                ctx,
                "serper_scrape",
                Endpoint::Scrape.path(),
                &body,
                credits,
                started,
                &error,
            )
            .await;
            return Err(error);
        }
    };
    let preset = args.preset();
    let content = projection::scrape_content(&raw, preset);
    let actual = content.credits.unwrap_or(credits);
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
    let mut value = serde_json::to_value(&output)?;
    value = projection::fit_to_budget(value, projection::MAX_RESULT_BYTES);
    let serialized = serde_json::to_vec(&value)?;
    record_call(
        ctx,
        CallRecord {
            tool: "serper_scrape".to_owned(),
            endpoint: Endpoint::Scrape.path().to_owned(),
            args_hash: args_hash(ctx.hash_key.as_ref(), &body),
            credits_estimated: credits,
            credits_actual: Some(actual),
            batch_multiplier: 1,
            duration_ms: elapsed_ms(started),
            status: "ok".to_owned(),
            response_bytes: serialized.len() as u64,
            tokens: (serialized.len() / 4) as u64,
            tokens_saved: 0,
            cache_hit: false,
        },
    )
    .await;
    let cost = cost_info(ctx, actual, None)?;
    let mut value = value;
    set_cost(&mut value, &cost)?;
    let value = projection::fit_to_budget(value, projection::MAX_RESULT_BYTES);
    let text = match args.text_mode() {
        TextMode::Full => serde_json::to_string(&value)?,
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
    Ok(tool_result(value, text))
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
    record_call(
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
    )
    .await;
    let cost = cost_info(ctx, 0, None)?;
    set_cost(&mut envelope, &cost)?;
    let envelope = projection::fit_to_budget(envelope, projection::MAX_RESULT_BYTES);
    let text = projection::summary_text(result_count(&envelope), top.as_deref(), 0, Some(0.0));
    Ok(tool_result(envelope, text))
}

async fn batch(
    ctx: &ToolContext,
    args: BatchArgs,
    cancellation: &CancellationToken,
) -> Result<CallToolResult> {
    if !ctx.settings.enable_batch() {
        return Err(Error::UnknownTool {
            name: "serper_search_batch".to_owned(),
        });
    }
    let endpoint = args.kind.endpoint();
    let bodies = args.bodies()?;
    let total = args
        .queries
        .iter()
        .map(|query| endpoint.estimated_cost(query.num).get())
        .sum::<u32>()
        .saturating_mul(BATCH_MULTIPLIER);
    let _reservation = reserve_budget(ctx, total)?;
    let body = Value::Array(bodies);
    ctx.client
        .acquire_queries(args.queries.len(), cancellation)
        .await?;
    let started = Instant::now();
    let raw = match ctx.client.post(endpoint, &body, cancellation).await {
        Ok(raw) => raw,
        Err(error) => {
            record_failure(
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
    let Some(items) = raw.as_array() else {
        let error = Error::Upstream {
            status: 200,
            message: "the batch response was not an array".to_owned(),
        };
        record_failure(
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
    let entries = items
        .iter()
        .map(|item| {
            let query = item
                .get("searchParameters")
                .and_then(|params| params.get("q"))
                .and_then(Value::as_str)
                .map(crate::projection::sanitize);
            let (results, error) = match project(endpoint, item, preset) {
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
    let mut value = serde_json::to_value(&output)?;
    value = projection::fit_to_budget(value, projection::MAX_RESULT_BYTES);
    let serialized = serde_json::to_vec(&value)?;
    record_call(
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
    )
    .await;
    let cost = cost_info(ctx, total, None)?;
    let mut value = value;
    set_cost(&mut value, &cost)?;
    let value = projection::fit_to_budget(value, projection::MAX_RESULT_BYTES);
    let text = match args.text_mode() {
        TextMode::Full => serde_json::to_string(&value)?,
        TextMode::Summary => format!(
            "{} queries; cost: {} credits ({})",
            output.entries.len(),
            cost.credits,
            usd_text(cost.usd)
        ),
    };
    Ok(tool_result(value, text))
}

async fn health(ctx: &ToolContext, _args: HealthArgs) -> Result<CallToolResult> {
    let started = Instant::now();
    let body = ctx.client.health().await.ok_or_else(|| Error::Upstream {
        status: 503,
        message: "api.serper.dev/health did not answer".to_owned(),
    })?;
    let status = body
        .get("status")
        .and_then(Value::as_str)
        .map(|raw| crate::projection::cap(&crate::projection::sanitize(raw), 64).0);
    let info = body.get("info").map(crate::projection::sanitize_value);
    record_call(
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
            response_bytes: serde_json::to_vec(&body)?.len() as u64,
            tokens: 0,
            tokens_saved: 0,
            cache_hit: false,
        },
    )
    .await;
    let output = HealthOutput {
        status: status.clone(),
        info,
        cost: cost_info(ctx, 0, None)?,
    };
    let value = serde_json::to_value(&output)?;
    let value = projection::fit_to_budget(value, projection::MAX_RESULT_BYTES);
    let text = format!("service status: {}", status.as_deref().unwrap_or("unknown"));
    Ok(tool_result(value, text))
}

async fn session_totals(ctx: &ToolContext, args: SessionArgs) -> Result<CallToolResult> {
    let totals = ctx.ledger.totals()?;
    let pending = ctx.ledger.pending()?;
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
    let live_balance = if args.live.unwrap_or(false) {
        ctx.client.live_balance().await
    } else {
        None
    };
    let budget_remaining = ctx
        .settings
        .session_budget_credits()
        .map(|budget| budget.saturating_sub(totals.credits.saturating_add(pending)));
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
    };
    let value = serde_json::to_value(&output)?;
    let text = format!(
        "{} calls; {} credits ({}); {} tokens, {} saved",
        output.calls,
        output.credits,
        usd_text(output.usd),
        output.tokens,
        output.tokens_saved
    );
    Ok(tool_result(value, text))
}

fn reserve_budget(ctx: &ToolContext, credits: u32) -> Result<Option<Reservation>> {
    if let Some(ceiling) = ctx.settings.per_call_credit_ceiling()
        && credits > ceiling
    {
        return Err(Error::BudgetRefused(format!(
            "{credits} credits is above the per-call ceiling of {ceiling}"
        )));
    }
    match ctx.settings.session_budget_credits() {
        Some(budget) => Ok(Some(ctx.ledger.reserve(credits, budget)?)),
        None => Ok(None),
    }
}

fn usd_text(usd: Option<f64>) -> String {
    usd.map_or_else(|| "usd unknown".to_owned(), |value| format!("${value:.6}"))
}

fn cost_info(ctx: &ToolContext, credits: u32, cache_age: Option<u64>) -> Result<CostInfo> {
    let totals = ctx.ledger.totals()?;
    Ok(CostInfo {
        credits,
        usd: usd_for(credits, ctx.settings.pack(), ctx.settings.usd_per_1k()),
        session_credits: totals.credits,
        session_usd: usd_for_credits(
            totals.credits,
            ctx.settings.pack(),
            ctx.settings.usd_per_1k(),
        ),
        cache_hit: cache_age.map(|_| true),
        cached_age_secs: cache_age,
    })
}

async fn append_call_record(ctx: &ToolContext, call_record: CallRecord) -> Result<()> {
    tracing::debug!(
        tool = %call_record.tool,
        credits = call_record.credits_estimated,
        cache_hit = call_record.cache_hit,
        status = %call_record.status,
        "tool call"
    );
    let ledger = ctx.ledger.clone();
    tokio::task::spawn_blocking(move || ledger.append(call_record).map(|_| ()))
        .await
        .map_err(|error| Error::Ledger(error.to_string()))?
}

async fn record_call(ctx: &ToolContext, call_record: CallRecord) {
    if let Err(error) = append_call_record(ctx, call_record).await {
        tracing::error!(%error, "a completed call could not be written to the ledger");
    }
}

async fn record_failure(
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
        // The request may have reached Serper, so charge the estimate rather than zero.
        credits_actual: None,
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
