use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::cost::Endpoint;
use crate::error::{Error, Result};
use crate::types::{MapViewport, Preset, TextMode, TimeFilter};

pub(crate) const MAX_BATCH_QUERIES: usize = 100;
pub(crate) const MAX_RESOLVE_LINKS: usize = 100;
pub(crate) const DEFAULT_RESULT_COUNT: u8 = 10;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    pub q: String,
    #[serde(default)]
    pub gl: Option<String>,
    #[serde(default)]
    pub hl: Option<String>,
    #[serde(default)]
    pub num: Option<u8>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub tbs: Option<TimeFilter>,
    #[serde(default)]
    pub autocorrect: Option<bool>,
    #[serde(default)]
    pub resolve_links: Option<bool>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub ll: Option<MapViewport>,
    #[serde(default)]
    pub fields: Option<Preset>,
    #[serde(default)]
    pub result_text: Option<TextMode>,
}

impl SearchArgs {
    pub fn preset(&self) -> Preset {
        self.fields.unwrap_or_default()
    }

    pub fn text_mode(&self) -> TextMode {
        self.result_text.unwrap_or_default()
    }

    pub fn body(&self, endpoint: Endpoint, default_freshness: Option<TimeFilter>) -> Result<Value> {
        let mut body = Map::new();
        body.insert("q".to_owned(), Value::String(validate_query(&self.q)?));
        if let Some(gl) = &self.gl {
            body.insert("gl".to_owned(), Value::String(validate_code(gl, "gl")?));
        }
        if let Some(hl) = &self.hl {
            body.insert("hl".to_owned(), Value::String(validate_language(hl, "hl")?));
        }
        if let Some(num) = self.num.or(default_result_count(endpoint)) {
            if !(1..=100).contains(&num) {
                return Err(Error::InvalidArgs(format!("num {num} is outside 1..100")));
            }
            body.insert("num".to_owned(), json!(num));
        }
        if let Some(page) = self.page {
            if page == 0 {
                return Err(Error::InvalidArgs("page starts at 1".into()));
            }
            body.insert("page".to_owned(), json!(page));
        }
        let tbs = match self.tbs {
            Some(filter) => Some(filter),
            None if endpoint == Endpoint::Search => default_freshness,
            None => None,
        };
        if let Some(value) = tbs.and_then(TimeFilter::as_str) {
            body.insert("tbs".to_owned(), Value::String(value.to_owned()));
        }
        if let Some(autocorrect) = self.autocorrect {
            body.insert("autocorrect".to_owned(), Value::Bool(autocorrect));
        }
        if let Some(resolve_links) = self.resolve_links {
            body.insert("resolveLinks".to_owned(), Value::Bool(resolve_links));
        }
        if let Some(location) = &self.location {
            let location = crate::types::LocationQuery::try_from(location.clone())?;
            body.insert("location".to_owned(), Value::String(location.into()));
        }
        if let Some(ll) = &self.ll {
            body.insert("ll".to_owned(), Value::String(ll.as_str().to_owned()));
        }
        reject_unsupported(endpoint, &body)?;
        Ok(Value::Object(body))
    }
}

fn default_result_count(endpoint: Endpoint) -> Option<u8> {
    (endpoint == Endpoint::Search).then_some(DEFAULT_RESULT_COUNT)
}

fn reject_unsupported(endpoint: Endpoint, body: &Map<String, Value>) -> Result<()> {
    let allowed_only = |keys: &[&str]| -> Result<()> {
        for key in body.keys() {
            if !keys.contains(&key.as_str()) {
                return Err(Error::InvalidArgs(format!(
                    "{} does not accept {key}",
                    endpoint.tool_name()
                )));
            }
        }
        Ok(())
    };
    if endpoint == Endpoint::Autocomplete {
        allowed_only(&["q", "gl", "hl"])?;
    }
    Ok(())
}

fn validate_query(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidArgs("q is empty".into()));
    }
    if trimmed.chars().count() > 2048 {
        return Err(Error::InvalidArgs(
            "q is longer than 2048 characters".into(),
        ));
    }
    Ok(trimmed.to_owned())
}

fn validate_code(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    let is_valid = trimmed.chars().count() == 2
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphabetic());
    if !is_valid {
        let shown: String = raw.chars().take(16).collect();
        return Err(Error::InvalidArgs(format!(
            "{field} takes a two-letter code, got: {shown}"
        )));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn validate_language(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    let mut parts = trimmed.split('-');
    let base = parts.next().unwrap_or_default();
    let region = parts.next();
    let base_ok = (2..=3).contains(&base.chars().count())
        && base
            .chars()
            .all(|character| character.is_ascii_alphabetic());
    let region_ok = match region {
        None => true,
        Some(region) => {
            (2..=3).contains(&region.chars().count())
                && region
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        }
    };
    let valid = base_ok && region_ok && parts.next().is_none();
    if !valid {
        let shown: String = raw.chars().take(16).collect();
        return Err(Error::InvalidArgs(format!(
            "{field} takes a two or three letter code with an optional region, got: {shown}"
        )));
    }
    Ok(trimmed.to_ascii_lowercase())
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReviewsArgs {
    #[serde(default)]
    pub cid: Option<String>,
    #[serde(default)]
    pub fid: Option<String>,
    #[serde(default, rename = "placeId")]
    pub place_id: Option<String>,
    #[serde(default)]
    pub gl: Option<String>,
    #[serde(default)]
    pub hl: Option<String>,
    #[serde(default)]
    pub next_page_token: Option<String>,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub topic_id: Option<String>,
    #[serde(default)]
    pub num: Option<u8>,
    #[serde(default)]
    pub fields: Option<Preset>,
    #[serde(default)]
    pub result_text: Option<TextMode>,
}

impl ReviewsArgs {
    pub fn preset(&self) -> Preset {
        self.fields.unwrap_or_default()
    }

    pub fn text_mode(&self) -> TextMode {
        self.result_text.unwrap_or_default()
    }

    pub fn body(&self) -> Result<Value> {
        let identifiers = [&self.cid, &self.fid, &self.place_id]
            .iter()
            .filter(|value| value.is_some())
            .count();
        if identifiers != 1 {
            return Err(Error::InvalidArgs(
                "pass exactly one of cid, fid, or placeId".into(),
            ));
        }
        let mut body = Map::new();
        if let Some(cid) = &self.cid {
            body.insert("cid".to_owned(), Value::String(identifier(cid, "cid")?));
        }
        if let Some(fid) = &self.fid {
            body.insert("fid".to_owned(), Value::String(identifier(fid, "fid")?));
        }
        if let Some(place_id) = &self.place_id {
            body.insert(
                "placeId".to_owned(),
                Value::String(identifier(place_id, "placeId")?),
            );
        }
        if let Some(gl) = &self.gl {
            body.insert("gl".to_owned(), Value::String(validate_code(gl, "gl")?));
        }
        if let Some(hl) = &self.hl {
            body.insert("hl".to_owned(), Value::String(validate_language(hl, "hl")?));
        }
        if let Some(token) = &self.next_page_token {
            body.insert(
                "nextPageToken".to_owned(),
                Value::String(identifier(token, "nextPageToken")?),
            );
        }
        if let Some(sort_by) = &self.sort_by {
            body.insert(
                "sortBy".to_owned(),
                Value::String(identifier(sort_by, "sortBy")?),
            );
        }
        if let Some(topic) = &self.topic_id {
            body.insert(
                "topicId".to_owned(),
                Value::String(identifier(topic, "topicId")?),
            );
        }
        if let Some(num) = self.num {
            if !(1..=100).contains(&num) {
                return Err(Error::InvalidArgs(format!("num {num} is outside 1..100")));
            }
            body.insert("num".to_owned(), json!(num));
        }
        Ok(Value::Object(body))
    }
}

fn identifier(raw: &str, field: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 512 {
        return Err(Error::InvalidArgs(format!("{field} is empty or too long")));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(Error::InvalidArgs(format!(
            "{field} contains a control character"
        )));
    }
    Ok(trimmed.to_owned())
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LensArgs {
    pub url: String,
    #[serde(default)]
    pub gl: Option<String>,
    #[serde(default)]
    pub hl: Option<String>,
    #[serde(default)]
    pub fields: Option<Preset>,
    #[serde(default)]
    pub result_text: Option<TextMode>,
}

impl LensArgs {
    pub fn preset(&self) -> Preset {
        self.fields.unwrap_or_default()
    }

    pub fn text_mode(&self) -> TextMode {
        self.result_text.unwrap_or_default()
    }

    pub fn body(&self) -> Result<Value> {
        let mut body = Map::new();
        body.insert("url".to_owned(), Value::String(valid_url(&self.url)?));
        if let Some(gl) = &self.gl {
            body.insert("gl".to_owned(), Value::String(validate_code(gl, "gl")?));
        }
        if let Some(hl) = &self.hl {
            body.insert("hl".to_owned(), Value::String(validate_language(hl, "hl")?));
        }
        Ok(Value::Object(body))
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScrapeArgs {
    pub url: String,
    #[serde(default)]
    pub include_markdown: Option<bool>,
    #[serde(default)]
    pub include_images: Option<bool>,
    #[serde(default)]
    pub include_links: Option<bool>,
    #[serde(default)]
    pub include_videos: Option<bool>,
    #[serde(default)]
    pub fields: Option<Preset>,
    #[serde(default)]
    pub result_text: Option<TextMode>,
}

impl ScrapeArgs {
    pub fn preset(&self) -> Preset {
        self.fields.unwrap_or_default()
    }

    pub fn text_mode(&self) -> TextMode {
        self.result_text.unwrap_or_default()
    }

    pub fn body(&self) -> Result<Value> {
        let mut body = Map::new();
        body.insert("url".to_owned(), Value::String(valid_url(&self.url)?));
        body.insert(
            "includeMarkdown".to_owned(),
            Value::Bool(self.include_markdown.unwrap_or(true)),
        );
        body.insert(
            "includeImages".to_owned(),
            Value::Bool(self.include_images.unwrap_or(false)),
        );
        body.insert(
            "includeLinks".to_owned(),
            Value::Bool(self.include_links.unwrap_or(false)),
        );
        body.insert(
            "includeVideos".to_owned(),
            Value::Bool(self.include_videos.unwrap_or(false)),
        );
        Ok(Value::Object(body))
    }
}

fn valid_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let parsed = Url::parse(trimmed)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(Error::InvalidArgs(
            "url must start with http:// or https://".into(),
        ));
    }
    if parsed.host_str().is_none() {
        return Err(Error::InvalidArgs("url has no host".into()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(Error::InvalidArgs(
            "url must not carry a username or password".into(),
        ));
    }
    let normalized = parsed.as_str().to_owned();
    if normalized.chars().count() > 2048 {
        return Err(Error::InvalidArgs(
            "url is longer than 2048 characters".into(),
        ));
    }
    Ok(normalized)
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LocationsArgs {
    pub q: String,
    #[serde(default)]
    pub limit: Option<u8>,
}

impl LocationsArgs {
    pub fn query(&self) -> Result<Vec<(&'static str, String)>> {
        let query = validate_query(&self.q)?;
        let mut pairs = vec![("q", query)];
        if let Some(limit) = self.limit {
            if !(1..=100).contains(&limit) {
                return Err(Error::InvalidArgs(format!(
                    "limit {limit} is outside 1..100"
                )));
            }
            pairs.push(("limit", limit.to_string()));
        }
        Ok(pairs)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveArgs {
    pub links: Vec<String>,
}

impl ResolveArgs {
    pub fn body(&self) -> Result<Value> {
        if self.links.is_empty() {
            return Err(Error::InvalidArgs("links is empty".into()));
        }
        if self.links.len() > MAX_RESOLVE_LINKS {
            return Err(Error::InvalidArgs(format!(
                "links holds {} items, above the {MAX_RESOLVE_LINKS} limit",
                self.links.len()
            )));
        }
        let organic = self
            .links
            .iter()
            .map(|link| Ok(json!({ "link": valid_url(link)? })))
            .collect::<Result<Vec<_>>>()?;
        Ok(json!({ "organic": organic }))
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    Search,
    Images,
    News,
    Places,
    Videos,
    Shopping,
    Scholar,
    Patents,
}

impl BatchKind {
    pub const fn endpoint(self) -> Endpoint {
        match self {
            Self::Search => Endpoint::Search,
            Self::Images => Endpoint::Images,
            Self::News => Endpoint::News,
            Self::Places => Endpoint::Places,
            Self::Videos => Endpoint::Videos,
            Self::Shopping => Endpoint::Shopping,
            Self::Scholar => Endpoint::Scholar,
            Self::Patents => Endpoint::Patents,
        }
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchQueryArgs {
    pub q: String,
    #[serde(default)]
    pub gl: Option<String>,
    #[serde(default)]
    pub hl: Option<String>,
    #[serde(default)]
    pub num: Option<u8>,
    #[serde(default)]
    pub page: Option<u32>,
    #[serde(default)]
    pub tbs: Option<TimeFilter>,
    #[serde(default)]
    pub autocorrect: Option<bool>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub ll: Option<MapViewport>,
}

impl BatchQueryArgs {
    fn body(&self, endpoint: Endpoint, default_freshness: Option<TimeFilter>) -> Result<Value> {
        SearchArgs {
            q: self.q.clone(),
            gl: self.gl.clone(),
            hl: self.hl.clone(),
            num: self.num,
            page: self.page,
            tbs: self.tbs,
            autocorrect: self.autocorrect,
            resolve_links: None,
            location: self.location.clone(),
            ll: self.ll.clone(),
            fields: None,
            result_text: None,
        }
        .body(endpoint, default_freshness)
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchArgs {
    pub kind: BatchKind,
    pub queries: Vec<BatchQueryArgs>,
    #[serde(default)]
    pub fields: Option<Preset>,
    #[serde(default)]
    pub result_text: Option<TextMode>,
}

impl BatchArgs {
    pub fn preset(&self) -> Preset {
        self.fields.unwrap_or_default()
    }

    pub fn text_mode(&self) -> TextMode {
        self.result_text.unwrap_or_default()
    }

    pub fn bodies(&self, default_freshness: Option<TimeFilter>) -> Result<Vec<Value>> {
        if self.queries.is_empty() {
            return Err(Error::InvalidArgs("queries is empty".into()));
        }
        if self.queries.len() > MAX_BATCH_QUERIES {
            return Err(Error::InvalidArgs(format!(
                "queries holds {} items, above the {MAX_BATCH_QUERIES} limit",
                self.queries.len()
            )));
        }
        self.queries
            .iter()
            .map(|query| query.body(self.kind.endpoint(), default_freshness))
            .collect()
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionArgs {
    #[serde(default)]
    pub live: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_search_args(raw: serde_json::Value) -> SearchArgs {
        serde_json::from_value(raw).expect("args parse")
    }

    #[test]
    fn a_web_search_asks_for_a_full_page_by_default() {
        let args = parse_search_args(serde_json::json!({"q": "apple"}));
        let search = args.body(Endpoint::Search, None).expect("search body");
        assert_eq!(
            search.get("num").and_then(Value::as_u64),
            Some(u64::from(DEFAULT_RESULT_COUNT))
        );
        let asked = parse_search_args(serde_json::json!({"q": "apple", "num": 100}));
        let search = asked.body(Endpoint::Search, None).expect("search body");
        assert_eq!(search.get("num").and_then(Value::as_u64), Some(100));
    }

    #[test]
    fn a_result_count_outside_web_search_is_sent_only_when_asked() {
        let args = parse_search_args(serde_json::json!({"q": "apple"}));
        for endpoint in [Endpoint::News, Endpoint::Images, Endpoint::Videos] {
            let body = args.body(endpoint, None).expect("body");
            assert!(
                body.get("num").is_none(),
                "{} sent a default",
                endpoint.tool_name()
            );
        }
        let asked = parse_search_args(serde_json::json!({"q": "apple", "num": 20}));
        let news = asked.body(Endpoint::News, None).expect("news body");
        assert_eq!(news.get("num").and_then(Value::as_u64), Some(20));
    }

    #[test]
    fn a_search_body_carries_only_what_was_set() {
        let args = parse_search_args(serde_json::json!({"q": "apple inc", "gl": "US", "num": 5}));
        let body = args.body(Endpoint::Search, None).expect("body");
        assert_eq!(body.get("gl").and_then(Value::as_str), Some("us"));
        assert_eq!(body.get("num").and_then(Value::as_u64), Some(5));
        assert!(body.get("hl").is_none());
        assert!(body.get("tbs").is_none());
    }

    #[test]
    fn search_body_rejects_bad_codes_out_of_range_numbers_and_blank_queries() {
        let bad_gl = parse_search_args(serde_json::json!({"q": "x", "gl": "usa"}));
        assert!(bad_gl.body(Endpoint::Search, None).is_err());
        let bad_num = parse_search_args(serde_json::json!({"q": "x", "num": 0}));
        assert!(bad_num.body(Endpoint::Search, None).is_err());
        let big_num = parse_search_args(serde_json::json!({"q": "x", "num": 101}));
        assert!(big_num.body(Endpoint::Search, None).is_err());
        let bad_page = parse_search_args(serde_json::json!({"q": "x", "page": 0}));
        assert!(bad_page.body(Endpoint::Search, None).is_err());
        let empty_query = parse_search_args(serde_json::json!({"q": "   "}));
        assert!(empty_query.body(Endpoint::Search, None).is_err());
    }

    #[test]
    fn autocomplete_rejects_parameters_it_does_not_take() {
        let args = parse_search_args(serde_json::json!({"q": "appl", "num": 5}));
        assert!(args.body(Endpoint::Autocomplete, None).is_err());
        let valid_args = parse_search_args(serde_json::json!({"q": "appl", "gl": "us"}));
        assert!(valid_args.body(Endpoint::Autocomplete, None).is_ok());
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let parsed =
            serde_json::from_value::<SearchArgs>(serde_json::json!({"q": "x", "extra": 1}));
        assert!(parsed.is_err());
    }

    #[test]
    fn reviews_needs_exactly_one_identifier() {
        let two: ReviewsArgs =
            serde_json::from_value(serde_json::json!({"cid": "1", "fid": "2"})).expect("parse");
        assert!(two.body().is_err());
        let one: ReviewsArgs =
            serde_json::from_value(serde_json::json!({"placeId": "abc"})).expect("parse");
        let body = one.body().expect("body");
        assert_eq!(body.get("placeId").and_then(Value::as_str), Some("abc"));
    }

    #[test]
    fn a_resolve_body_wraps_each_link_in_an_organic_entry() {
        let args: ResolveArgs = serde_json::from_value(serde_json::json!({
            "links": ["https://www.google.com/goto?url=CAESabc", "https://example.org/a"]
        }))
        .expect("parse");
        let body = args.body().expect("body");
        let organic = body
            .get("organic")
            .and_then(Value::as_array)
            .expect("organic");
        assert_eq!(organic.len(), 2);
        assert_eq!(
            organic[1].get("link").and_then(Value::as_str),
            Some("https://example.org/a")
        );
    }

    #[test]
    fn a_resolve_body_refuses_no_links_a_crowd_or_a_non_http_link() {
        let empty: ResolveArgs =
            serde_json::from_value(serde_json::json!({"links": []})).expect("parse");
        assert!(empty.body().is_err());
        let crowd: ResolveArgs = serde_json::from_value(serde_json::json!({
            "links": vec!["https://example.org/a"; MAX_RESOLVE_LINKS + 1]
        }))
        .expect("parse");
        assert!(crowd.body().is_err());
        let bad: ResolveArgs =
            serde_json::from_value(serde_json::json!({"links": ["ftp://example.org/a"]}))
                .expect("parse");
        assert!(bad.body().is_err());
    }

    #[test]
    fn a_search_asks_serper_to_resolve_links_only_when_told() {
        let quiet = parse_search_args(serde_json::json!({"q": "apple"}));
        let body = quiet.body(Endpoint::Search, None).expect("body");
        assert!(body.get("resolveLinks").is_none());
        let told = parse_search_args(serde_json::json!({"q": "apple", "resolve_links": false}));
        let body = told.body(Endpoint::Search, None).expect("body");
        assert_eq!(
            body.get("resolveLinks").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn urls_must_be_http_or_https() {
        let args: LensArgs =
            serde_json::from_value(serde_json::json!({"url": "ftp://example.org/a.png"}))
                .expect("parse");
        assert!(args.body().is_err());
        let valid_args: LensArgs =
            serde_json::from_value(serde_json::json!({"url": "https://example.org/a.png"}))
                .expect("parse");
        assert!(valid_args.body().is_ok());
    }

    #[test]
    fn location_queries_carry_the_limit_within_bounds() {
        let within_bounds: LocationsArgs =
            serde_json::from_value(serde_json::json!({"q": "dhaka", "limit": 5})).expect("parse");
        let query = within_bounds.query().expect("query");
        assert_eq!(query.len(), 2);
        assert_eq!(query[1].0, "limit");
        assert_eq!(query[1].1, "5");

        let too_many: LocationsArgs =
            serde_json::from_value(serde_json::json!({"q": "dhaka", "limit": 101})).expect("parse");
        assert!(too_many.query().is_err());
    }

    #[test]
    fn the_session_tool_parses_the_live_flag() {
        let live: SessionArgs =
            serde_json::from_value(serde_json::json!({"live": true})).expect("parse");
        assert_eq!(live.live, Some(true));
        let default: SessionArgs = serde_json::from_value(serde_json::json!({})).expect("parse");
        assert_eq!(default.live, None);
    }

    #[test]
    fn a_url_is_normalized_before_it_is_sent() {
        let args: ScrapeArgs =
            serde_json::from_value(serde_json::json!({"url": "https://example.org/\tevil"}))
                .expect("parse");
        let body = args.body().expect("body");
        let sent = body.get("url").and_then(Value::as_str).expect("url");
        assert!(!sent.contains('\t'));
        assert_eq!(sent, "https://example.org/evil");
    }

    #[test]
    fn long_urls_are_refused() {
        let long = format!("https://example.org/{}", "a".repeat(3000));
        let args: LensArgs =
            serde_json::from_value(serde_json::json!({"url": long})).expect("parse");
        assert!(args.body().is_err());
    }

    #[test]
    fn batch_rejects_an_empty_query_list_and_accepts_two_queries() {
        let empty: BatchArgs =
            serde_json::from_value(serde_json::json!({"kind": "search", "queries": []}))
                .expect("parse");
        assert!(empty.bodies(None).is_err());
        let one: BatchArgs = serde_json::from_value(
            serde_json::json!({"kind": "search", "queries": [{"q": "a"}, {"q": "b"}]}),
        )
        .expect("parse");
        assert_eq!(one.bodies(None).expect("bodies").len(), 2);
    }

    #[test]
    fn scrape_defaults_to_markdown_only() {
        let args: ScrapeArgs =
            serde_json::from_value(serde_json::json!({"url": "https://example.org"}))
                .expect("parse");
        let body = args.body().expect("body");
        assert_eq!(
            body.get("includeMarkdown").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            body.get("includeImages").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn hl_accepts_three_letter_bases_and_regions() {
        for code in ["zh-cn", "zh-tw", "pt-br", "es-419", "fil", "EN-gb"] {
            let args = parse_search_args(serde_json::json!({"q": "x", "hl": code}));
            let body = args.body(Endpoint::Search, None).expect("body");
            let expected = code.to_ascii_lowercase();
            assert_eq!(
                body.get("hl").and_then(Value::as_str),
                Some(expected.as_str()),
                "hl {code} was refused"
            );
        }
    }

    #[test]
    fn gl_still_takes_only_two_letters() {
        let args = parse_search_args(serde_json::json!({"q": "x", "gl": "zh-cn"}));
        assert!(args.body(Endpoint::Search, None).is_err());
    }

    #[test]
    fn hl_rejects_a_malformed_region() {
        for code in ["en-", "en-usa-extra", "e", "123", "en_US", "fil-"] {
            let args = parse_search_args(serde_json::json!({"q": "x", "hl": code}));
            assert!(
                args.body(Endpoint::Search, None).is_err(),
                "hl {code} was accepted"
            );
        }
    }

    #[test]
    fn batch_queries_reject_per_query_presentation_fields() {
        let with_fields = serde_json::from_value::<BatchArgs>(
            serde_json::json!({"kind": "search", "queries": [{"q": "a", "fields": "full"}]}),
        );
        assert!(with_fields.is_err());
        let with_text = serde_json::from_value::<BatchArgs>(
            serde_json::json!({"kind": "search", "queries": [{"q": "a", "result_text": "full"}]}),
        );
        assert!(with_text.is_err());
    }

    #[test]
    fn a_web_search_takes_the_configured_freshness_window() {
        let args = parse_search_args(serde_json::json!({"q": "apple"}));
        let body = args
            .body(Endpoint::Search, Some(TimeFilter::Year))
            .expect("body");
        assert_eq!(body.get("tbs").and_then(Value::as_str), Some("qdr:y"));
        let images = args
            .body(Endpoint::Images, Some(TimeFilter::Year))
            .expect("images body");
        assert!(images.get("tbs").is_none());
        let off = args.body(Endpoint::Search, None).expect("body");
        assert!(off.get("tbs").is_none());
    }

    #[test]
    fn a_caller_time_filter_beats_the_configured_one() {
        let asked = parse_search_args(serde_json::json!({"q": "apple", "tbs": "qdr:d"}));
        let body = asked
            .body(Endpoint::Search, Some(TimeFilter::Year))
            .expect("body");
        assert_eq!(body.get("tbs").and_then(Value::as_str), Some("qdr:d"));
    }

    #[test]
    fn the_all_filter_asks_google_for_every_year() {
        let any_time = parse_search_args(serde_json::json!({"q": "apple", "tbs": "all"}));
        let body = any_time
            .body(Endpoint::Search, Some(TimeFilter::Year))
            .expect("body");
        assert!(body.get("tbs").is_none());
    }

    #[test]
    fn batch_queries_still_build_their_body() {
        let args: BatchArgs = serde_json::from_value(
            serde_json::json!({"kind": "search", "queries": [{"q": "a", "num": 5}]}),
        )
        .expect("parse");
        let bodies = args.bodies(None).expect("bodies");
        let num = bodies
            .first()
            .and_then(|body| body.get("num"))
            .and_then(Value::as_u64);
        assert_eq!(num, Some(5));
    }

    #[test]
    fn urls_with_embedded_credentials_are_refused() {
        for url in [
            "https://user:secret@example.org/x",
            "https://user@example.org/x",
        ] {
            let args: ScrapeArgs =
                serde_json::from_value(serde_json::json!({"url": url})).expect("parse");
            assert!(args.body().is_err(), "accepted {url}");
        }
        let clean: ScrapeArgs =
            serde_json::from_value(serde_json::json!({"url": "https://example.org/x"}))
                .expect("parse");
        assert!(clean.body().is_ok());
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthArgs {}
