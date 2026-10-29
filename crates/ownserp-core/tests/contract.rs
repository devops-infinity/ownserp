#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ownserp_core::{Overrides, SerperClient, Settings, ToolContext};

mod contexts {
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
        let dir =
            std::env::temp_dir().join(format!("ownserp-contract-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let profile = dir.join("profile.toml");
        std::fs::write(&profile, "").expect("profile");
        let settings = Settings::load(Overrides {
            profile: Some(profile),
            ledger_dir: Some(dir),
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

use contexts::{context, context_with};
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
            truncated: None,
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
            truncated: None,
        title: Some("Apple Inc.".to_owned()),
        answer: Some("Apple Inc. designs, manufactures and markets smartphones, personal computers, tablets, wearables and accessories.".to_owned()),
        snippet: None,
        link: Some("https://www.apple.com".to_owned()),
        list: None,
    };

    let questions = vec![
        PeopleAlsoAsk {
            truncated: None,
            question: Some("What does Apple Inc do?".to_owned()),
            snippet: Some("Apple designs and sells consumer electronics and software.".to_owned()),
            title: Some("Apple Inc. | History, Products, Headquarters and Facts".to_owned()),
            link: Some("https://www.britannica.com/topic/Apple-Inc".to_owned()),
        },
        PeopleAlsoAsk {
            truncated: None,
            question: Some("Who founded Apple?".to_owned()),
            snippet: Some(
                "Steve Jobs, Steve Wozniak and Ronald Wayne founded Apple in 1976.".to_owned(),
            ),
            title: Some("Apple Inc.".to_owned()),
            link: Some("https://en.wikipedia.org/wiki/Apple_Inc.".to_owned()),
        },
        PeopleAlsoAsk {
            truncated: None,
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

fn error_code(value: &Value) -> Option<&str> {
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
    assert_eq!(error_code(&value), Some("serper.bad_request"));
}

#[tokio::test]
async fn upstream_auth_failure_maps_to_a_stable_error_code() {
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
    assert_eq!(error_code(&value), Some("serper.auth_failed"));
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
async fn a_reported_credit_count_is_not_charged_again_on_a_cache_hit() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "credits": 2,
            "organic": []
        })))
        .expect(1)
        .mount(&server)
        .await;
    let ctx = context("cache-credits", &server.uri(), 60);
    let args = arguments(json!({"q": "apple inc"}));
    for _ in 0..2 {
        call_tool(
            &ctx,
            "serper_search",
            Some(&args),
            &CancellationToken::new(),
        )
        .await;
    }
    let totals = ctx.ledger.totals().expect("totals");
    assert_eq!(totals.cache_hits, 1);
    assert_eq!(totals.credits, 2);
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
    assert_eq!(error_code(&value), Some("serper.tool_disabled"));
    assert!(
        value.to_string().contains("enable_batch"),
        "the refusal must name the switch: {value}"
    );
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
    assert_eq!(error_code(&value), Some("serper.budget_refused"));
}

#[tokio::test]
async fn the_reviews_tool_requires_exactly_one_identifier() {
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
    assert_eq!(error_code(&value), Some("serper.bad_request"));
}

#[tokio::test]
async fn a_one_credit_call_fits_a_one_credit_budget() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/reviews"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reviews": [
                {"rating": 4.5, "date": "2026-01-02", "snippet": "fine", "author": "A", "title": "T", "position": 1}
            ]
        })))
        .mount(&server)
        .await;
    let dir = std::env::temp_dir().join(format!("ownserp-budget-once-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let profile = dir.join("profile.toml");
    std::fs::write(&profile, "session_budget_credits = 1\n").expect("profile");
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
    let args = arguments(json!({"cid": "123"}));
    let result = call_tool(
        &ctx,
        "serper_reviews",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(
        error_code(&value),
        None,
        "a one-credit call must fit a one-credit budget"
    );
    assert_eq!(
        value
            .get("structuredContent")
            .and_then(|body| body.get("cost"))
            .and_then(|cost| cost.get("credits"))
            .and_then(Value::as_u64),
        Some(1)
    );
}

#[tokio::test]
async fn a_server_error_charges_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(500).set_body_json(json!({"message": "boom"})))
        .mount(&server)
        .await;
    let ctx = context("server-error-charge", &server.uri(), 0);
    let failed = call_tool(
        &ctx,
        "serper_search",
        Some(&arguments(json!({"q": "x"}))),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(
        error_code(&result_json(&failed)),
        Some("serper.upstream_failed")
    );

    let ledger = std::fs::read_to_string(ctx.ledger.path().expect("path")).expect("ledger");
    assert!(
        ledger.contains("\"credits_actual\":0"),
        "a 5xx must be booked at zero: {ledger}"
    );

    let session = call_tool(
        &ctx,
        "serper_session",
        Some(&arguments(json!({}))),
        &CancellationToken::new(),
    )
    .await;
    let body = result_json(&session);
    assert_eq!(
        body.pointer("/structuredContent/credits")
            .and_then(Value::as_u64),
        Some(0),
        "a 5xx must not spend credits"
    );
    assert_eq!(
        body.pointer("/structuredContent/calls")
            .and_then(Value::as_u64),
        Some(1)
    );
}

#[tokio::test]
async fn a_scrape_without_a_reported_price_books_the_maximum_estimate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"markdown": "# Title\n\nBody."})),
        )
        .mount(&server)
        .await;
    let ctx = context("scrape-estimate", &server.uri(), 0);
    let args = arguments(json!({"url": "https://example.org/page"}));
    let result = call_tool(
        &ctx,
        "serper_scrape",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(error_code(&result_json(&result)), None);
    let ledger = std::fs::read_to_string(ctx.ledger.path().expect("path")).expect("ledger");
    assert!(
        ledger.contains("\"credits_estimated\":10"),
        "an unknown scrape price must keep the maximum estimate: {ledger}"
    );
    assert!(
        ledger.contains("\"credits_actual\":null"),
        "an unknown scrape price must not book a figure: {ledger}"
    );
}

#[tokio::test]
async fn an_exhausted_balance_reports_its_own_code_and_top_up_remedy() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(
            ResponseTemplate::new(400).set_body_json(json!({"message": "Not enough credits"})),
        )
        .mount(&server)
        .await;
    let ctx = context("exhausted", &server.uri(), 0);
    let failed = call_tool(
        &ctx,
        "serper_search",
        Some(&arguments(json!({"q": "x"}))),
        &CancellationToken::new(),
    )
    .await;
    let body = result_json(&failed);
    assert_eq!(
        error_code(&body),
        Some("serper.credits_exhausted"),
        "an exhausted balance must not read as a rate limit"
    );
    assert!(
        body.to_string().contains("serper.dev/dashboard"),
        "the remedy must name the top-up step: {body}"
    );
}

#[tokio::test]
async fn a_rejected_call_charges_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"message": "bad key"})))
        .mount(&server)
        .await;
    let ctx = context("rejected-charge", &server.uri(), 0);
    let failed = call_tool(
        &ctx,
        "serper_search",
        Some(&arguments(json!({"q": "x"}))),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(
        error_code(&result_json(&failed)),
        Some("serper.auth_failed")
    );

    let ledger = std::fs::read_to_string(ctx.ledger.path().expect("path")).expect("ledger");
    assert!(
        ledger.contains("\"credits_actual\":0"),
        "a rejected call must be booked at zero: {ledger}"
    );

    let session = call_tool(
        &ctx,
        "serper_session",
        Some(&arguments(json!({}))),
        &CancellationToken::new(),
    )
    .await;
    let body = result_json(&session);
    assert_eq!(
        body.pointer("/structuredContent/credits")
            .and_then(Value::as_u64),
        Some(0),
        "a rejected call must not spend credits"
    );
    assert_eq!(
        body.pointer("/structuredContent/calls")
            .and_then(Value::as_u64),
        Some(1),
        "the rejected call is still one recorded call"
    );
}

#[tokio::test]
async fn a_recorded_call_survives_a_ledger_lock() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "searchParameters": {"q": "x", "type": "search"},
            "organic": [{"title": "t", "link": "l", "snippet": "s", "position": 1}],
            "credits": 1
        })))
        .mount(&server)
        .await;
    let ctx = context("locked-ledger", &server.uri(), 0);
    let path = ctx.ledger.path().expect("path");
    let holder = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(&path)
        .expect("holder");
    holder.try_lock().expect("exclusive lock");
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(6));
        let _ = holder.unlock();
    });

    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&arguments(json!({"q": "x"}))),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(
        error_code(&value),
        None,
        "a paid call must answer even when its row cannot be written"
    );
    assert_eq!(
        value
            .pointer("/structuredContent/cost/credits")
            .and_then(Value::as_u64),
        Some(1)
    );
    let _ = releaser.join();
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
    assert_eq!(error_code(&value), Some("serper.upstream_failed"));
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
    let oversized_markdown = "word ".repeat(60_000);
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"markdown": oversized_markdown, "credits": 2})),
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
    assert_eq!(error_code(&value), Some("serper.rate_limited"));
}

#[tokio::test]
async fn a_cancelled_call_reports_cancellation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_secs(2))
                .set_body_json(json!({"organic": []})),
        )
        .mount(&server)
        .await;
    let ctx = context("cancel", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let token = CancellationToken::new();
    let signal = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        signal.cancel();
    });
    let result = call_tool(&ctx, "serper_search", Some(&args), &token).await;
    let value = result_json(&result);
    assert_eq!(error_code(&value), Some("serper.cancelled"));
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

#[tokio::test]
async fn the_health_and_balance_probes_read_their_own_paths() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "ok"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/account"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"credits": 1234})))
        .mount(&server)
        .await;
    let ctx = context("probes", &server.uri(), 0);
    assert!(ctx.client.health().await.is_some());
    assert_eq!(ctx.client.live_balance().await, Some(1234));
}

#[tokio::test]
async fn a_failed_probe_reports_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/account"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let ctx = context("probesfail", &server.uri(), 0);
    assert!(ctx.client.health().await.is_none());
    assert!(ctx.client.live_balance().await.is_none());
}

#[tokio::test]
async fn the_health_tool_reports_the_service_state() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok",
            "info": {"region": "us"}
        })))
        .mount(&server)
        .await;
    let ctx = context("health", &server.uri(), 0);
    let args = arguments(json!({}));
    let result = call_tool(
        &ctx,
        "serper_health",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    assert_eq!(value.get("isError"), Some(&Value::Bool(false)), "{value}");
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(body.get("status").and_then(Value::as_str), Some("ok"));
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(0)
    );
}

#[tokio::test]
async fn reported_credits_replace_the_estimate() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "credits": 3,
            "organic": [{"title": "one", "link": "l", "position": 1}]
        })))
        .mount(&server)
        .await;
    let ctx = context("reported", &server.uri(), 0);
    let args = arguments(json!({"q": "apple inc"}));
    let result = call_tool(
        &ctx,
        "serper_search",
        Some(&args),
        &CancellationToken::new(),
    )
    .await;
    let value = result_json(&result);
    let body = value.get("structuredContent").expect("structured content");
    assert_eq!(
        body.pointer("/cost/credits").and_then(Value::as_u64),
        Some(3),
        "the cost object carries the reported figure"
    );
    let ledger = std::fs::read_to_string(ctx.ledger.path().expect("path")).expect("ledger");
    assert!(
        ledger.contains("\"credits_actual\":3"),
        "ledger was: {ledger}"
    );
}

fn keyless_settings(label: &str) -> Settings {
    let dir = std::env::temp_dir().join(format!("ownserp-keyless-{}-{label}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let profile = dir.join("profile.toml");
    std::fs::write(&profile, "").expect("profile");
    Settings::load_with_keychain(
        Overrides {
            profile: Some(profile),
            ledger_dir: Some(dir),
            cache_ttl_secs: Some(0),
            ..Overrides::default()
        },
        |_| None,
        || Ok(None),
    )
    .expect("settings load")
}

#[tokio::test]
async fn a_paid_call_without_a_key_is_refused_locally_with_the_local_remedy() {
    let ctx = ToolContext::new(keyless_settings("paid")).expect("context");
    let failed = call_tool(
        &ctx,
        "serper_search",
        Some(&arguments(json!({"q": "x"}))),
        &CancellationToken::new(),
    )
    .await;
    let body = result_json(&failed);
    assert_eq!(
        error_code(&body),
        Some("serper.auth_missing"),
        "a missing key must be caught before the network call: {body}"
    );
    assert!(
        body.to_string().contains("config set-api-key"),
        "the remedy must name the local fix: {body}"
    );
}

#[tokio::test]
async fn a_free_tool_still_answers_when_no_key_is_set() {
    let ctx = ToolContext::new(keyless_settings("free")).expect("context");
    let answered = call_tool(&ctx, "serper_session", None, &CancellationToken::new()).await;
    assert_eq!(
        result_json(&answered)
            .get("isError")
            .and_then(Value::as_bool),
        Some(false),
        "a free tool must not be refused for a missing key"
    );
}

#[tokio::test]
async fn a_disabled_batch_tool_names_the_switch_that_enables_it() {
    let server = MockServer::start().await;
    let ctx = context_with("batch-hidden", &server.uri(), 0, false);
    let failed = call_tool(
        &ctx,
        "serper_search_batch",
        Some(&arguments(json!({"queries": [{"q": "x"}]}))),
        &CancellationToken::new(),
    )
    .await;
    let body = result_json(&failed);
    assert_eq!(error_code(&body), Some("serper.tool_disabled"));
    assert!(
        body.to_string().contains("enable_batch"),
        "the refusal must name the switch: {body}"
    );
}

#[test]
fn every_error_variant_carries_a_code_a_message_and_a_remedy() {
    let json_error = serde_json::from_str::<Value>("{").expect_err("invalid json");
    let errors = vec![
        ownserp_core::Error::MissingApiKey { cause: None },
        ownserp_core::Error::MissingApiKey {
            cause: Some("no session bus".to_owned()),
        },
        ownserp_core::Error::InvalidArgs("q".to_owned()),
        ownserp_core::Error::Upstream {
            status: 500,
            message: "down".to_owned(),
        },
        ownserp_core::Error::RateLimited {
            message: "slow down".to_owned(),
        },
        ownserp_core::Error::CreditsExhausted {
            message: "empty".to_owned(),
        },
        ownserp_core::Error::BudgetRefused("10 credits".to_owned()),
        ownserp_core::Error::network("calling Serper", std::io::Error::other("reset")),
        ownserp_core::Error::Config("bad value".to_owned()),
        ownserp_core::Error::keychain("reading the key", std::io::Error::other("denied")),
        ownserp_core::Error::Ledger("broken chain".to_owned()),
        ownserp_core::Error::LedgerBusy("held".to_owned()),
        ownserp_core::Error::Io {
            context: "writing the ledger",
            source: std::io::Error::other("full"),
        },
        ownserp_core::Error::Json {
            context: "reading a body",
            source: json_error,
        },
        ownserp_core::Error::Projection("bad shape".to_owned()),
        ownserp_core::Error::UnknownTool {
            name: "nope".to_owned(),
        },
        ownserp_core::Error::ToolDisabled {
            name: "serper_search_batch".to_owned(),
            switch: "enable_batch",
        },
        ownserp_core::Error::Cancelled,
        ownserp_core::Error::DeadlineExceeded,
        ownserp_core::Error::ConnectFailed {
            message: "unreachable".to_owned(),
        },
        ownserp_core::Error::Refused {
            code: "serper.busy",
            message: "busy".to_owned(),
            remedy: "Retry after one second.",
        },
    ];
    for error in errors {
        let body = error.to_json();
        let code = body.get("code").and_then(Value::as_str).unwrap_or_default();
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let remedy = body
            .get("remedy")
            .and_then(Value::as_str)
            .unwrap_or_default();
        assert!(code.starts_with("serper."), "{code} is not a serper code");
        assert!(!message.is_empty(), "{code} carries no message");
        assert!(!remedy.is_empty(), "{code} carries no remedy");
    }
}
