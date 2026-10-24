use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::cost::Endpoint;
use crate::error::{Error, Result};
use crate::types::{MapViewport, Preset, TextMode, TimeFilter};

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

    pub fn body(&self, endpoint: Endpoint) -> Result<Value> {
        let mut body = Map::new();
        body.insert("q".to_owned(), Value::String(validate_query(&self.q)?));
        if let Some(gl) = &self.gl {
            body.insert("gl".to_owned(), Value::String(validate_code(gl, "gl")?));
        }
        if let Some(hl) = &self.hl {
            body.insert("hl".to_owned(), Value::String(validate_code(hl, "hl")?));
        }
        if let Some(num) = self.num {
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
        if let Some(tbs) = self.tbs {
            body.insert("tbs".to_owned(), Value::String(tbs.as_str().to_owned()));
        }
        if let Some(autocorrect) = self.autocorrect {
            body.insert("autocorrect".to_owned(), Value::Bool(autocorrect));
        }
        if let Some(location) = &self.location {
            let location = crate::types::LocationQuery::try_from(location.clone())?;
            body.insert("location".to_owned(), Value::String(location.into()));
        }
        if let Some(ll) = &self.ll {
            body.insert("ll".to_owned(), Value::String(ll.as_query().to_owned()));
        }
        reject_unsupported(endpoint, &body)?;
        Ok(Value::Object(body))
    }
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
    let is_valid =
        trimmed.chars().count() == 2 && trimmed.chars().all(|ch| ch.is_ascii_alphabetic());
    if !is_valid {
        let shown: String = raw.chars().take(16).collect();
        return Err(Error::InvalidArgs(format!(
            "{field} takes a two-letter code, got: {shown}"
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
            body.insert("hl".to_owned(), Value::String(validate_code(hl, "hl")?));
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
            body.insert("hl".to_owned(), Value::String(validate_code(hl, "hl")?));
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
    if trimmed.chars().count() > 2048 {
        return Err(Error::InvalidArgs(
            "url is longer than 2048 characters".into(),
        ));
    }
    Ok(trimmed.to_owned())
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
pub struct BatchArgs {
    pub kind: BatchKind,
    pub queries: Vec<SearchArgs>,
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

    pub fn bodies(&self) -> Result<Vec<Value>> {
        if self.queries.is_empty() {
            return Err(Error::InvalidArgs("queries is empty".into()));
        }
        if self.queries.len() > 100 {
            return Err(Error::InvalidArgs(format!(
                "queries holds {} items, above the 100 limit",
                self.queries.len()
            )));
        }
        self.queries
            .iter()
            .map(|query| query.body(self.kind.endpoint()))
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

    fn search(raw: serde_json::Value) -> SearchArgs {
        serde_json::from_value(raw).expect("args parse")
    }

    #[test]
    fn a_search_body_carries_only_what_was_set() {
        let args = search(serde_json::json!({"q": "apple inc", "gl": "US", "num": 5}));
        let body = args.body(Endpoint::Search).expect("body");
        assert_eq!(body.get("gl").and_then(Value::as_str), Some("us"));
        assert_eq!(body.get("num").and_then(Value::as_u64), Some(5));
        assert!(body.get("hl").is_none());
        assert!(body.get("tbs").is_none());
    }

    #[test]
    fn codes_and_bounds_are_enforced() {
        let bad_gl = search(serde_json::json!({"q": "x", "gl": "usa"}));
        assert!(bad_gl.body(Endpoint::Search).is_err());
        let bad_num = search(serde_json::json!({"q": "x", "num": 0}));
        assert!(bad_num.body(Endpoint::Search).is_err());
        let big_num = search(serde_json::json!({"q": "x", "num": 101}));
        assert!(big_num.body(Endpoint::Search).is_err());
        let bad_page = search(serde_json::json!({"q": "x", "page": 0}));
        assert!(bad_page.body(Endpoint::Search).is_err());
        let empty_query = search(serde_json::json!({"q": "   "}));
        assert!(empty_query.body(Endpoint::Search).is_err());
    }

    #[test]
    fn autocomplete_rejects_parameters_it_does_not_take() {
        let args = search(serde_json::json!({"q": "appl", "num": 5}));
        assert!(args.body(Endpoint::Autocomplete).is_err());
        let valid_args = search(serde_json::json!({"q": "appl", "gl": "us"}));
        assert!(valid_args.body(Endpoint::Autocomplete).is_ok());
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
        let ok: LocationsArgs =
            serde_json::from_value(serde_json::json!({"q": "dhaka", "limit": 5})).expect("parse");
        let query = ok.query().expect("query");
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
    fn long_urls_are_refused() {
        let long = format!("https://example.org/{}", "a".repeat(3000));
        let args: LensArgs =
            serde_json::from_value(serde_json::json!({"url": long})).expect("parse");
        assert!(args.body().is_err());
    }

    #[test]
    fn batch_bounds_are_enforced() {
        let empty: BatchArgs =
            serde_json::from_value(serde_json::json!({"kind": "search", "queries": []}))
                .expect("parse");
        assert!(empty.bodies().is_err());
        let one: BatchArgs = serde_json::from_value(
            serde_json::json!({"kind": "search", "queries": [{"q": "a"}, {"q": "b"}]}),
        )
        .expect("parse");
        assert_eq!(one.bodies().expect("bodies").len(), 2);
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
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HealthArgs {}
