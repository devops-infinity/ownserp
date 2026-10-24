#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ownserp_core::{Overrides, SerperClient, Settings, ToolContext};

mod common {
    use super::*;

    pub(crate) fn context(label: &str, base_url: &str, cache_ttl: u64) -> ToolContext {
        context_with(label, base_url, cache_ttl, false)
    }

    pub(crate) fn context_with(
        label: &str,
        base_url: &str,
        cache_ttl: u64,
        enable_batch: bool,
    ) -> ToolContext {
        let settings = Settings::load(Overrides {
            profile: Some(
                std::env::temp_dir()
                    .join(format!("ownserp-contract-{}-{label}", std::process::id()))
                    .join("absent-profile.toml"),
            ),
            ledger_dir: Some(
                std::env::temp_dir()
                    .join(format!("ownserp-contract-{}-{label}", std::process::id())),
            ),
            cache_ttl_secs: Some(cache_ttl),
            enable_batch: Some(enable_batch),
            ..Overrides::default()
        })
        .expect("settings load");
        let client = SerperClient::with_base_url(&settings, base_url).expect("client");
        let mut context = ToolContext::new(settings).expect("context");
        context.client = client;
        context
    }
}

use common::{context, context_with};
use ownserp_core::call_tool;
use ownserp_core::{AnswerBox, KnowledgeGraph, OrganicItem, PeopleAlsoAsk};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn organic(title: &str, link: &str, snippet: &str, position: u32) -> OrganicItem {
    OrganicItem {
        title: Some(title.to_owned()),
        link: Some(link.to_owned()),
        snippet: Some(snippet.to_owned()),
        position: Some(position),
        date: None,
        sitelinks: None,
        attributes: None,
        truncated: None,
    }
}

fn fixture() -> Value {
    let mut first = organic(
        "Apple",
        "https://www.apple.com",
        "Discover the innovative world of Apple and shop everything iPhone, iPad, Apple Watch, Mac, and Apple TV, plus explore accessories, entertainment, and expert device support.",
        1,
    );
    first.sitelinks = Some(vec![
        json!({"title": "iPhone", "link": "https://www.apple.com/iphone/"}),
        json!({"title": "Mac", "link": "https://www.apple.com/mac/"}),
    ]);
    first.attributes = Some(json!({"Region": "United States"}));

    let mut second = organic(
        "Apple Inc. - Wikipedia",
        "https://en.wikipedia.org/wiki/Apple_Inc.",
        "Apple Inc. is an American multinational technology company headquartered in Cupertino, California. Apple is the world's largest technology company by revenue.",
        2,
    );
    second.date = Some("2026-10-20".to_owned());

    let mut fifth = organic(
        "Apple Inc - Reuters",
        "https://www.reuters.com/markets/companies/AAPL.OQ/",
        "Find the latest Apple Inc news, earnings and analyst research.",
        5,
    );
    fifth.date = Some("2 weeks ago".to_owned());

    let organic_items: Vec<OrganicItem> = vec![
        first,
        second,
        organic(
            "Apple Inc. | History, Products, Headquarters, and Facts",
            "https://www.britannica.com/topic/Apple-Inc",
            "Apple Inc., formerly Apple Computer Inc., is an American multinational corporation that is one of the world's leading consumer electronics and software companies.",
            3,
        ),
        organic(
            "Apple Inc. (AAPL) Company Profile and News",
            "https://www.bloomberg.com/profile/company/AAPL:US",
            "Apple Inc. designs, manufactures, and markets smartphones, personal computers, tablets, wearables, and accessories worldwide.",
            4,
        ),
        fifth,
        organic(
            "Apple Inc. Company Profile - MarketWatch",
            "https://www.marketwatch.com/investing/stock/aapl/company-profile",
            "Apple Inc. engages in the design, manufacture, and sale of smartphones, personal computers, tablets, wearables, and accessories.",
            6,
        ),
        organic(
            "Apple Inc. | LinkedIn",
            "https://www.linkedin.com/company/apple",
            "Apple Inc. | 21,000,000 followers on LinkedIn. Apple revolutionized personal technology with the introduction of the Macintosh in 1984.",
            7,
        ),
        organic(
            "Apple Inc. Company Profile and News",
            "https://www.cnbc.com/quotes/AAPL",
            "Apple Inc. engages in the design, manufacture, and marketing of smartphones, personal computers, and portable digital music players.",
            8,
        ),
    ];

    let graph = KnowledgeGraph {
        title: Some("Apple".to_owned()),
        kind: Some("Technology company".to_owned()),
        website: Some("https://www.apple.com".to_owned()),
        image_url: Some("https://example.com/apple.png".to_owned()),
        description: Some("Apple Inc. is an American multinational technology company headquartered in Cupertino, California. Apple is the world's largest technology company by revenue and one of the big five American information technology companies.".to_owned()),
        description_source: Some("Wikipedia".to_owned()),
        description_link: Some("https://en.wikipedia.org/wiki/Apple_Inc.".to_owned()),
        attributes: Some(json!({"Headquarters": "Cupertino, CA", "Founded": "1976"})),
    };

    let answer = AnswerBox {
        title: Some("Apple Inc.".to_owned()),
        answer: Some("Apple Inc. designs, manufactures and markets smartphones, personal computers, tablets, wearables and accessories.".to_owned()),
        snippet: None,
        link: Some("https://www.apple.com".to_owned()),
        list: None,
    };

    let questions = vec![
        PeopleAlsoAsk {
            question: Some("What does Apple Inc do?".to_owned()),
            snippet: Some("Apple designs and sells consumer electronics and software.".to_owned()),
            title: Some("Apple Inc. | History, Products, Headquarters and Facts".to_owned()),
            link: Some("https://www.britannica.com/topic/Apple-Inc".to_owned()),
        },
        PeopleAlsoAsk {
            question: Some("Who founded Apple?".to_owned()),
            snippet: Some(
                "Steve Jobs, Steve Wozniak and Ronald Wayne founded Apple in 1976.".to_owned(),
            ),
            title: Some("Apple Inc.".to_owned()),
            link: Some("https://en.wikipedia.org/wiki/Apple_Inc.".to_owned()),
        },
        PeopleAlsoAsk {
            question: Some("Where is Apple headquartered?".to_owned()),
            snippet: Some("Apple Park, One Apple Park Way, Cupertino, California.".to_owned()),
            title: Some("Apple Park".to_owned()),
            link: Some("https://www.apple.com/apple-park/".to_owned()),
        },
    ];

    let related: Vec<Value> = [
        "apple inc stock",
        "apple inc history",
        "apple inc revenue",
        "apple inc headquarters",
        "apple inc products",
        "apple inc ceo",
        "apple inc news",
        "apple inc careers",
    ]
    .iter()
    .map(|query| json!({"query": query}))
    .collect();

    json!({
        "searchParameters": {
            "q": "apple inc",
            "gl": "us",
            "hl": "en",
            "autocorrect": true,
            "page": 1,
            "type": "search"
        },
        "knowledgeGraph": graph,
        "answerBox": answer,
        "peopleAlsoAsk": questions,
        "relatedSearches": related,
        "organic": organic_items
    })
}

#[test]
fn the_typed_builder_reproduces_the_recorded_payload() {
    let recorded: Value =
        serde_json::from_str(include_str!("fixtures/search.json")).expect("recorded fixture");
    assert_eq!(fixture(), recorded);
}

fn arguments(value: Value) -> Map<String, Value> {
    value.as_object().expect("object arguments").clone()
}

fn result_json(result: &impl serde::Serialize) -> Value {
    serde_json::to_value(result).expect("result serializes")
}

fn code(value: &Value) -> Option<&str> {
    value
        .get("structuredContent")
        .and_then(|body| body.get("code"))
        .and_then(Value::as_str)
}

#[tokio::test]
async fn search_returns_a_typed_envelope_with_cost() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = context("search", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc", "num": 10}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)));
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(
        body.get("results").and_then(Value::as_array).map(Vec::len),
        Some(8)
    );
    assert!(body.get("knowledgeGraph").is_some());
    assert!(body.get("answerBox").is_some());
    assert_eq!(
        body.get("relatedSearches")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(6),
        "standard preset keeps six related searches"
    );
    assert!(
        body.pointer("/results/0/attributes").is_none(),
        "standard preset drops the attributes map"
    );
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(1)
    );
    let text = value
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .expect("summary text");
    assert!(text.starts_with("8 results"), "summary was: {text}");
    insta::assert_json_snapshot!("search_standard", body);
}

#[tokio::test]
async fn minimal_preset_drops_snippets_and_sections() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .mount(&server)
        .await;
    let ctx = context("minimal", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc", "fields": "minimal"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    let body = value.get("structuredContent").expect("structured content");
    assert!(body.get("knowledgeGraph").is_none());
    assert!(body.get("relatedSearches").is_none());
    assert!(body.pointer("/results/0/snippet").is_none());
    assert!(body.pointer("/results/0/title").is_some());
}

#[tokio::test]
async fn unknown_arguments_are_rejected_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(0)
        .mount(&server)
        .await;
    let ctx = context("unknown", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc", "bogus": true}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(true)));
    assert_eq!(code(&value), Some("serper.bad_request"));
}

#[tokio::test]
async fn upstream_auth_failure_maps_to_a_stable_code() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"message": "Unauthorized"})))
        .mount(&server)
        .await;
    let ctx = context("auth", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(code(&value), Some("serper.auth_failed"));
    let message = value
        .pointer("/structuredContent/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(message.contains("401"));
}

#[tokio::test]
async fn every_call_lands_in_the_ledger_chain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .mount(&server)
        .await;
    let ctx = context("ledger", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let path = ctx.ledger.path().expect("ledger path");
    let rows = ownserp_core::verify(&path).expect("chain verifies");
    assert_eq!(rows, 1);
    let totals = ctx.ledger.totals().expect("totals");
    assert_eq!(totals.calls, 1);
    assert_eq!(totals.credits, 1);
    assert_eq!(
        totals
            .per_tool
            .get("serper_search")
            .map(|entry| entry.calls),
        Some(1)
    );
}

#[tokio::test]
async fn a_cache_hit_costs_nothing_and_says_so() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = context("cache", &server.uri(), 60);
    let args = arguments(json!({"q": "apple inc"}));
    let first = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let second = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let first = result_json(&first);
    let second = result_json(&second);
    assert_eq!(
        first
            .pointer("/structuredContent/cost/credits")
            .and_then(Value::as_u64),
        Some(1)
    );
    assert_eq!(
        second
            .pointer("/structuredContent/cost/credits")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        second
            .pointer("/structuredContent/cost/cacheHit")
            .and_then(Value::as_bool),
        Some(true)
    );
    let totals = ctx.ledger.totals().expect("totals");
    assert_eq!(totals.cache_hits, 1);
    assert_eq!(totals.credits, 1);
}

#[tokio::test]
async fn locations_are_free_and_unauthenticated() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/locations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {
                "name": "Dhaka, Bangladesh",
                "canonicalName": "Dhaka,Bangladesh",
                "googleId": "ChIJL6wWn9nKVTcR",
                "countryCode": "BD",
                "targetType": "City"
            }
        ])))
        .mount(&server)
        .await;
    let ctx = context("locations", &server.uri(), 0);
    let args = arguments(json!({"q": "dhaka"}));
    let result = call_tool(
        &ctx,
        "serper_locations",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)));
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(
        body.pointer("/results/0/canonicalName")
            .and_then(Value::as_str),
        Some("Dhaka,Bangladesh")
    );
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(0)
    );
}

#[tokio::test]
async fn scrape_reports_the_credits_the_response_names() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "markdown": "# Example\n\nBody text.",
            "credits": 6
        })))
        .mount(&server)
        .await;
    let ctx = context("scrape", &server.uri(), 0);
    let args = arguments(json!({"url": "https://example.org/page"}));
    let result = call_tool(
        &ctx,
        "serper_scrape",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(body.get("credits").and_then(Value::as_u64), Some(6));
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(6)
    );
    assert!(
        body.get("markdown")
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("Example"))
    );
}

#[tokio::test]
async fn a_disabled_batch_tool_is_refused() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(0)
        .mount(&server)
        .await;
    let ctx = context("batchoff", &server.uri(), 0);
    let args = arguments(json!({"kind": "search", "queries": [{"q": "a"}]}));
    let result = call_tool(
        &ctx,
        "serper_search_batch",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(code(&value), Some("serper.tool_not_found"));
}

#[tokio::test]
async fn scraping_above_the_ceiling_is_refused_before_spending() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"markdown": "x"})))
        .expect(0)
        .mount(&server)
        .await;
    let dir = std::env::temp_dir().join(format!("ownserp-ceiling-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let profile = dir.join("profile.toml");
    std::fs::write(&profile, "per_call_credit_ceiling = 5\n").expect("profile");
    let settings = Settings::load(Overrides {
        profile: Some(profile),
        ledger_dir: Some(dir.join("ledger")),
        cache_ttl_secs: Some(0),
        ..Overrides::default()
    })
    .expect("settings");
    let client = SerperClient::with_base_url(&settings, &server.uri()).expect("client");
    let mut ctx = ToolContext::new(settings).expect("context");
    ctx.client = client;
    let args = arguments(json!({"url": "https://example.org/hard"}));
    let result = call_tool(
        &ctx,
        "serper_scrape",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(code(&value), Some("serper.budget_refused"));
}

#[tokio::test]
async fn reviews_requires_exactly_one_identifier() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(0)
        .mount(&server)
        .await;
    let ctx = context("reviews", &server.uri(), 0);
    let args = arguments(json!({"cid": "123", "fid": "456"}));
    let result = call_tool(
        &ctx,
        "serper_reviews",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(code(&value), Some("serper.bad_request"));
}

#[tokio::test]
async fn every_search_family_tool_projects_its_own_section() {
    let cases = vec![
        (
            "/news",
            "serper_news",
            json!({"q": "x"}),
            json!({"news": [{"title": "n", "link": "l", "position": 1}]}),
        ),
        (
            "/images",
            "serper_images",
            json!({"q": "x"}),
            json!({"images": [{"title": "i", "imageUrl": "u", "position": 1}]}),
        ),
        (
            "/places",
            "serper_places",
            json!({"q": "x"}),
            json!({"places": [{"title": "p", "address": "a", "position": 1}]}),
        ),
        (
            "/videos",
            "serper_videos",
            json!({"q": "x"}),
            json!({"videos": [{"title": "v", "link": "l", "position": 1}]}),
        ),
        (
            "/shopping",
            "serper_shopping",
            json!({"q": "x"}),
            json!({"shopping": [{"title": "s", "price": "$1", "position": 1}]}),
        ),
        (
            "/scholar",
            "serper_scholar",
            json!({"q": "x"}),
            json!({"organic": [{"title": "p", "link": "l", "position": 1}]}),
        ),
        (
            "/patents",
            "serper_patents",
            json!({"q": "x"}),
            json!({"patents": [{"title": "p", "link": "l", "position": 1}]}),
        ),
        (
            "/autocomplete",
            "serper_autocomplete",
            json!({"q": "x"}),
            json!({"suggestions": [{"value": "a"}]}),
        ),
        (
            "/lens",
            "serper_lens",
            json!({"url": "https://example.org/a.png"}),
            json!({"visualMatches": [{"title": "m", "link": "l", "position": 1}]}),
        ),
        (
            "/maps",
            "serper_maps",
            json!({"q": "x", "ll": "@1.5,2.5,14z"}),
            json!({"places": [{"title": "p", "position": 1}]}),
        ),
    ];
    for (endpoint, tool, args, body) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let ctx = context(&format!("family{tool}"), &server.uri(), 0);
        let args = arguments(args);
        let result = call_tool(&ctx, tool, Some(&args), &CancellationToken::new()).await;
        let value = result_json(&result);
        assert_eq!(
            value.get("isError"),
            Some(&Value::Bool(false)),
            "{tool} failed: {value}"
        );
        let count = value
            .pointer("/structuredContent/results")
            .and_then(Value::as_array)
            .map(Vec::len);
        assert_eq!(count, Some(1), "{tool} returned {count:?}");
    }
}

#[tokio::test]
async fn the_batch_tool_reports_each_query() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"searchParameters": {"q": "a"}, "organic": [{"title": "one", "link": "l", "position": 1}]},
            {"searchParameters": {"q": "b"}, "organic": [{"title": "two", "link": "l", "position": 1}]}
        ])))
        .mount(&server)
        .await;
    let ctx = context_with("batchtool", &server.uri(), 0, true);
    let args = arguments(json!({"kind": "search", "queries": [{"q": "a"}, {"q": "b"}]}));
    let result = call_tool(
        &ctx,
        "serper_search_batch",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)), "{value}");
    let body = value.get("structuredContent").expect("structured content");
    let entries = body
        .get("entries")
        .and_then(Value::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 2);
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(6),
        "two queries at three credits each"
    );
    assert_eq!(
        entries
            .first()
            .and_then(|entry| entry.get("query"))
            .and_then(Value::as_str),
        Some("a")
    );
}

#[tokio::test]
async fn the_session_tool_reads_the_local_ledger() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .mount(&server)
        .await;
    let ctx = context("sessiontool", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let session = arguments(json!({}));
    let result = call_tool(
        &ctx,
        "serper_session",
        Some(&session),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(body.get("calls").and_then(Value::as_u64), Some(1));
    assert_eq!(body.get("credits").and_then(Value::as_u64), Some(1));
    assert!(body.get("liveBalance").is_none());
}

#[tokio::test]
async fn a_non_json_success_body_is_an_upstream_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json</html>"))
        .mount(&server)
        .await;
    let ctx = context("nonjson", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(true)));
    assert_eq!(code(&value), Some("serper.upstream_failed"));
    let message = value
        .pointer("/structuredContent/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(message.contains("not JSON"), "message was: {message}");
}

#[tokio::test]
async fn an_oversized_announced_body_is_refused_before_reading() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_string(" ".repeat(4 * 1024 * 1024 + 10)))
        .mount(&server)
        .await;
    let ctx = context("oversize", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(true)));
    let message = value
        .pointer("/structuredContent/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(message.contains("byte cap"), "message was: {message}");
}

#[tokio::test]
async fn a_scrape_larger_than_the_budget_is_trimmed_and_marked() {
    let server = MockServer::start().await;
    let big = "word ".repeat(60_000);
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"markdown": big, "credits": 2})),
        )
        .mount(&server)
        .await;
    let ctx = context("scrapebudget", &server.uri(), 0);
    let args = arguments(json!({"url": "https://example.org/long"}));
    let result = call_tool(
        &ctx,
        "serper_scrape",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)));
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(body.get("truncated"), Some(&Value::Bool(true)));
    let size = serde_json::to_vec(body).expect("serialize").len();
    assert!(size <= 262_144, "trimmed body is {size} bytes");
}

#[tokio::test]
async fn a_transient_failure_is_retried_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"message": "busy"})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .mount(&server)
        .await;
    let ctx = context("retryok", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)), "{value}");
    assert_eq!(
        value
            .pointer("/structuredContent/results")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(8)
    );
}

#[tokio::test]
async fn a_rate_limit_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({"message": "slow down"})))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = context("retry429", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(code(&value), Some("serper.rate_limited"));
}

#[tokio::test]
async fn an_upstream_error_message_is_sanitized_and_capped() {
    let server = MockServer::start().await;
    let noisy = format!("bad\u{200B}\u{061C}{}", "word ".repeat(200));
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"message": noisy})))
        .mount(&server)
        .await;
    let ctx = context("noisyerror", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    let message = value
        .pointer("/structuredContent/message")
        .and_then(Value::as_str)
        .expect("message");
    assert!(!message.contains('\u{200B}'));
    assert!(!message.contains('\u{061C}'));
    assert!(
        message.chars().count() <= 330,
        "message was {} chars, above the capped body plus the 23 character prefix",
        message.chars().count()
    );
}
