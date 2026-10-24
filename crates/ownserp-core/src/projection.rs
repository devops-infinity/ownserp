use serde_json::Value;

use crate::response::{
    AnswerBox, ImageItem, KnowledgeGraph, LensItem, LocationItem, NewsItem, OrganicItem,
    PatentItem, PeopleAlsoAsk, PlaceItem, ReviewItem, ScholarItem, ShoppingItem, Suggestion,
    VideoItem,
};
use crate::types::Preset;

pub const MAX_RESULT_BYTES: usize = 262_144;
pub(crate) const MAX_SCRAPE_CHARS: usize = 200_000;
const STANDARD_SNIPPET: usize = 300;
const STANDARD_ANSWER: usize = 200;
const STANDARD_RELATED: usize = 6;

fn is_invisible(ch: char) -> bool {
    matches!(
        ch,
        '\u{00AD}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{180B}'..='\u{180F}'
            | '\u{034F}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{E0000}'..='\u{E007F}'
            | '\u{E0100}'..='\u{E01EF}'
    ) || matches!(
        ch,
        '\u{061C}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{FFA0}'
            | '\u{180E}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1D173}'..='\u{1D17A}'
    ) || (ch.is_control() && ch != '\n' && ch != '\t')
}

pub(crate) fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize(text)),
        Value::Array(items) => Value::Array(items.iter().map(sanitize_value).collect()),
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (key, item) in map {
                let key = sanitize(key);
                if !cleaned.contains_key(&key) {
                    cleaned.insert(key, sanitize_value(item));
                }
            }
            Value::Object(cleaned)
        }
        other => other.clone(),
    }
}

pub fn sanitize(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_space = false;
    for ch in raw.chars() {
        if is_invisible(ch) {
            continue;
        }
        let ch = if ch == '\n' || ch == '\t' { ' ' } else { ch };
        if ch == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(ch);
    }
    let trimmed = out.trim();
    if trimmed.len() == out.len() {
        out
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn cap(input: &str, max_chars: usize) -> (String, bool) {
    if input.chars().count() <= max_chars {
        return (input.to_owned(), false);
    }
    let mut out = String::with_capacity(max_chars.saturating_mul(4).saturating_add(3));
    for ch in input.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("...");
    (out, true)
}

pub(crate) fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(sanitize)
        .filter(|text| !text.is_empty())
}

pub(crate) fn capped_field(value: &Value, key: &str, max_chars: usize) -> (Option<String>, bool) {
    match text_field(value, key) {
        Some(text) => {
            let (capped, truncated) = cap(&text, max_chars);
            (Some(capped), truncated)
        }
        None => (None, false),
    }
}

pub(crate) fn number_field(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

pub(crate) fn integer_field(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
}

pub(crate) fn count_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}

pub(crate) fn text_array(value: &Value, key: &str) -> Option<Vec<String>> {
    let items = value.get(key)?.as_array()?;
    let out: Vec<String> = items
        .iter()
        .filter_map(Value::as_str)
        .map(sanitize)
        .filter(|text| !text.is_empty())
        .collect();
    if out.is_empty() { None } else { Some(out) }
}

pub(crate) fn array<'a>(raw: &'a Value, key: &str) -> &'a [Value] {
    raw.get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

pub(crate) fn object<'a>(raw: &'a Value, key: &str) -> Option<&'a Value> {
    raw.get(key).filter(|value| value.is_object())
}

fn snippet_for(value: &Value, preset: Preset) -> (Option<String>, bool) {
    match preset {
        Preset::Minimal => (None, false),
        Preset::Standard => capped_field(value, "snippet", STANDARD_SNIPPET),
        Preset::Full => (text_field(value, "snippet"), false),
    }
}

pub(crate) fn organic(raw: &Value, preset: Preset) -> (Vec<OrganicItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "organic") {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        let sitelinks = match preset {
            Preset::Full => value
                .get("sitelinks")
                .and_then(Value::as_array)
                .map(|items| items.iter().map(sanitize_value).collect::<Vec<_>>()),
            _ => None,
        };
        let attributes = match preset {
            Preset::Full => value
                .get("attributes")
                .filter(|value| value.is_object())
                .map(sanitize_value),
            _ => None,
        };
        items.push(OrganicItem {
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            snippet,
            position: integer_field(value, "position"),
            date: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "date"),
            },
            sitelinks,
            attributes,
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn images(raw: &Value, preset: Preset) -> (Vec<ImageItem>, bool) {
    let truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "images") {
        items.push(ImageItem {
            title: text_field(value, "title"),
            image_url: text_field(value, "imageUrl"),
            thumbnail_url: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "thumbnailUrl"),
            },
            image_width: match preset {
                Preset::Minimal => None,
                _ => integer_field(value, "imageWidth"),
            },
            image_height: match preset {
                Preset::Minimal => None,
                _ => integer_field(value, "imageHeight"),
            },
            thumbnail_width: match preset {
                Preset::Full => integer_field(value, "thumbnailWidth"),
                _ => None,
            },
            thumbnail_height: match preset {
                Preset::Full => integer_field(value, "thumbnailHeight"),
                _ => None,
            },
            source: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "source"),
            },
            domain: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "domain"),
            },
            google_url: match preset {
                Preset::Full => text_field(value, "googleUrl"),
                _ => None,
            },
            link: match preset {
                Preset::Full => text_field(value, "link"),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: None,
        });
    }
    (items, truncated)
}

pub(crate) fn news(raw: &Value, preset: Preset) -> (Vec<NewsItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "news") {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        items.push(NewsItem {
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            snippet,
            date: text_field(value, "date"),
            source: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "source"),
            },
            image_url: match preset {
                Preset::Full => text_field(value, "imageUrl"),
                _ => None,
            },
            position: match preset {
                Preset::Minimal => None,
                _ => integer_field(value, "position"),
            },
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn places(raw: &Value, preset: Preset) -> (Vec<PlaceItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "places") {
        let (description, cut) = match preset {
            Preset::Standard => capped_field(value, "description", STANDARD_SNIPPET),
            Preset::Full => (text_field(value, "description"), false),
            Preset::Minimal => (None, false),
        };
        truncated |= cut;
        items.push(PlaceItem {
            title: text_field(value, "title"),
            address: text_field(value, "address"),
            latitude: match preset {
                Preset::Minimal => None,
                _ => number_field(value, "latitude"),
            },
            longitude: match preset {
                Preset::Minimal => None,
                _ => number_field(value, "longitude"),
            },
            rating: number_field(value, "rating"),
            rating_count: match preset {
                Preset::Minimal => None,
                _ => count_field(value, "ratingCount"),
            },
            kind: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "type"),
            },
            price_level: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "priceLevel"),
            },
            place_id: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "placeId"),
            },
            fid: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "fid"),
            },
            opening_hours: match preset {
                Preset::Full => value.get("openingHours").map(sanitize_value),
                _ => None,
            },
            thumbnail_url: match preset {
                Preset::Full => text_field(value, "thumbnailUrl"),
                _ => None,
            },
            phone: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "phone").or_else(|| text_field(value, "phoneNumber")),
            },
            website: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "website"),
            },
            cid: match preset {
                Preset::Standard | Preset::Full => text_field(value, "cid"),
                Preset::Minimal => None,
            },
            category: match preset {
                Preset::Full => text_field(value, "category"),
                _ => None,
            },
            description,
            types: match preset {
                Preset::Full => text_array(value, "types"),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn videos(raw: &Value, preset: Preset) -> (Vec<VideoItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "videos") {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        items.push(VideoItem {
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            snippet,
            duration: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "duration"),
            },
            channel: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "channel"),
            },
            channel_link: match preset {
                Preset::Standard | Preset::Full => text_field(value, "channelLink"),
                Preset::Minimal => None,
            },
            source: match preset {
                Preset::Standard | Preset::Full => text_field(value, "source"),
                Preset::Minimal => None,
            },
            date: text_field(value, "date"),
            image_url: match preset {
                Preset::Full => text_field(value, "imageUrl"),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn shopping(raw: &Value, preset: Preset) -> (Vec<ShoppingItem>, bool) {
    let truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "shopping") {
        items.push(ShoppingItem {
            title: text_field(value, "title"),
            price: text_field(value, "price"),
            rating: match preset {
                Preset::Minimal => None,
                _ => number_field(value, "rating"),
            },
            rating_count: match preset {
                Preset::Minimal => None,
                _ => count_field(value, "ratingCount"),
            },
            product_id: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "productId"),
            },
            link: text_field(value, "link"),
            source: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "source"),
            },
            delivery: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "delivery"),
            },
            image_url: match preset {
                Preset::Full => text_field(value, "imageUrl"),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: None,
        });
    }
    (items, truncated)
}

pub(crate) fn scholar(raw: &Value, preset: Preset) -> (Vec<ScholarItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "organic") {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        items.push(ScholarItem {
            id: match preset {
                Preset::Full => text_field(value, "id"),
                _ => None,
            },
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            snippet,
            publication_info: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "publicationInfo"),
            },
            year: text_field(value, "year"),
            cited_by: match preset {
                Preset::Minimal => None,
                _ => count_field(value, "citedBy"),
            },
            pdf_url: match preset {
                Preset::Full => text_field(value, "pdfUrl"),
                _ => None,
            },
            cluster_id: match preset {
                Preset::Full => text_field(value, "clusterId"),
                _ => None,
            },
            versions: match preset {
                Preset::Full => value
                    .get("versions")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(sanitize_value).collect()),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn patents(raw: &Value, preset: Preset) -> (Vec<PatentItem>, bool) {
    let mut truncated = false;
    let source = if array(raw, "patents").is_empty() {
        array(raw, "organic")
    } else {
        array(raw, "patents")
    };
    let mut items = Vec::new();
    for value in source {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        items.push(PatentItem {
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            snippet,
            assignee: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "assignee"),
            },
            inventor: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "inventor"),
            },
            language: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "language"),
            },
            publication_date: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "publicationDate"),
            },
            publication_number: text_field(value, "publicationNumber"),
            filing_date: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "filingDate"),
            },
            grant_date: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "grantDate"),
            },
            priority_date: match preset {
                Preset::Standard | Preset::Full => text_field(value, "priorityDate"),
                Preset::Minimal => None,
            },
            figures: match preset {
                Preset::Full => value
                    .get("figures")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(sanitize_value).collect()),
                _ => None,
            },
            position: integer_field(value, "position"),
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn suggestions(raw: &Value) -> Vec<Suggestion> {
    array(raw, "suggestions")
        .iter()
        .filter_map(|value| {
            let text = match value {
                Value::String(text) => Some(text.clone()),
                _ => value
                    .get("value")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            }?;
            Some(Suggestion {
                value: sanitize(&text),
            })
        })
        .filter(|item| !item.value.is_empty())
        .collect()
}

pub(crate) fn reviews(raw: &Value, preset: Preset) -> (Vec<ReviewItem>, bool) {
    let mut truncated = false;
    let mut items = Vec::new();
    for value in array(raw, "reviews") {
        let (snippet, cut) = snippet_for(value, preset);
        truncated |= cut;
        items.push(ReviewItem {
            rating: number_field(value, "rating"),
            date: text_field(value, "date"),
            snippet,
            author: match preset {
                Preset::Standard | Preset::Full => text_field(value, "author"),
                Preset::Minimal => None,
            },
            title: match preset {
                Preset::Standard | Preset::Full => text_field(value, "title"),
                Preset::Minimal => None,
            },
            position: integer_field(value, "position"),
            truncated: cut.then_some(true),
        });
    }
    (items, truncated)
}

pub(crate) fn lens(raw: &Value, preset: Preset) -> (Vec<LensItem>, bool) {
    let key = if raw.get("visualMatches").is_some() {
        "visualMatches"
    } else {
        "organic"
    };
    let truncated = false;
    let mut items = Vec::new();
    for value in array(raw, key) {
        items.push(LensItem {
            title: text_field(value, "title"),
            image_url: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "imageUrl"),
            },
            link: text_field(value, "link"),
            source: text_field(value, "source"),
            thumbnail_url: match preset {
                Preset::Minimal => None,
                _ => text_field(value, "thumbnailUrl"),
            },
            position: integer_field(value, "position"),
            truncated: None,
        });
    }
    (items, truncated)
}

pub(crate) fn locations(raw: &Value) -> Vec<LocationItem> {
    let source = match raw {
        Value::Array(items) => items.as_slice(),
        _ => array(raw, "locations"),
    };
    source
        .iter()
        .map(|value| LocationItem {
            name: text_field(value, "name"),
            canonical_name: text_field(value, "canonicalName"),
            google_id: text_field(value, "googleId"),
            country_code: text_field(value, "countryCode"),
            target_type: text_field(value, "targetType"),
        })
        .collect()
}

#[derive(Default)]
pub struct SearchSections {
    pub knowledge_graph: Option<KnowledgeGraph>,
    pub answer_box: Option<AnswerBox>,
    pub people_also_ask: Option<Vec<PeopleAlsoAsk>>,
    pub related_searches: Option<Vec<String>>,
    pub search_parameters: Option<Value>,
    pub truncated: bool,
}

pub fn search_sections(raw: &Value, preset: Preset) -> SearchSections {
    if preset == Preset::Minimal {
        return SearchSections::default();
    }
    let mut truncated = false;
    let knowledge_graph = object(raw, "knowledgeGraph").map(|value| {
        let (description, cut) = match preset {
            Preset::Standard => capped_field(value, "description", STANDARD_SNIPPET),
            Preset::Full => (text_field(value, "description"), false),
            Preset::Minimal => (None, false),
        };
        truncated |= cut;
        KnowledgeGraph {
            title: text_field(value, "title"),
            kind: text_field(value, "type"),
            website: text_field(value, "website"),
            image_url: text_field(value, "imageUrl"),
            description,
            description_source: text_field(value, "descriptionSource"),
            description_link: text_field(value, "descriptionLink"),
            attributes: match preset {
                Preset::Full => value
                    .get("attributes")
                    .filter(|value| value.is_object())
                    .map(sanitize_value),
                _ => None,
            },
        }
    });
    let answer_box = object(raw, "answerBox").map(|value| AnswerBox {
        title: text_field(value, "title"),
        answer: match preset {
            Preset::Standard => {
                let (answer, cut) = capped_field(value, "answer", STANDARD_SNIPPET);
                truncated |= cut;
                answer
            }
            Preset::Full => text_field(value, "answer"),
            Preset::Minimal => None,
        },
        snippet: text_field(value, "snippet"),
        link: text_field(value, "link"),
        list: text_array(value, "list"),
    });
    let people_also_ask = {
        let source = array(raw, "peopleAlsoAsk");
        if source.is_empty() {
            None
        } else {
            let limit = match preset {
                Preset::Standard => STANDARD_RELATED,
                _ => source.len(),
            };
            let items: Vec<PeopleAlsoAsk> = source
                .iter()
                .take(limit)
                .map(|value| {
                    let (snippet, cut) = match preset {
                        Preset::Standard => capped_field(value, "snippet", STANDARD_ANSWER),
                        Preset::Full => (text_field(value, "snippet"), false),
                        Preset::Minimal => (None, false),
                    };
                    truncated |= cut;
                    PeopleAlsoAsk {
                        question: text_field(value, "question"),
                        snippet,
                        title: text_field(value, "title"),
                        link: text_field(value, "link"),
                    }
                })
                .collect();
            truncated |= source.len() > limit;
            Some(items)
        }
    };
    let related_searches = {
        let source = array(raw, "relatedSearches");
        if source.is_empty() {
            None
        } else {
            let limit = match preset {
                Preset::Standard => STANDARD_RELATED,
                _ => source.len(),
            };
            let values: Vec<String> = source
                .iter()
                .take(limit)
                .filter_map(|value| text_field(value, "query"))
                .collect();
            truncated |= source.len() > limit;
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
    };
    let search_parameters = match preset {
        Preset::Full => raw.get("searchParameters").map(sanitize_value),
        _ => None,
    };
    SearchSections {
        knowledge_graph,
        answer_box,
        people_also_ask,
        related_searches,
        search_parameters,
        truncated,
    }
}

pub struct ScrapeContent {
    pub markdown: Option<String>,
    pub text: Option<String>,
    pub title: Option<String>,
    pub credits: Option<u32>,
    pub images: Option<Vec<String>>,
    pub links: Option<Vec<String>>,
    pub videos: Option<Vec<String>>,
    pub truncated: bool,
}

pub(crate) fn scrape_content(raw: &Value, preset: Preset) -> ScrapeContent {
    let mut truncated = false;
    let mut pick = |key: &str| -> Option<String> {
        let text = text_field(raw, key)?;
        let (capped, cut) = cap(&text, MAX_SCRAPE_CHARS);
        truncated |= cut;
        Some(capped)
    };
    let markdown = pick("markdown");
    let text = match preset {
        Preset::Minimal => None,
        _ => pick("text"),
    };
    ScrapeContent {
        markdown,
        text,
        title: text_field(raw, "title"),
        credits: integer_field(raw, "credits"),
        images: match preset {
            Preset::Minimal => None,
            _ => text_array(raw, "images"),
        },
        links: match preset {
            Preset::Minimal => None,
            _ => text_array(raw, "links"),
        },
        videos: match preset {
            Preset::Full => text_array(raw, "videos"),
            _ => None,
        },
        truncated,
    }
}

pub fn fit_to_budget(mut envelope: Value, max_bytes: usize) -> Value {
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    mark_truncated(&mut envelope);
    loop {
        let key = if array_len(&envelope, "results") > 0 {
            "results"
        } else if array_len(&envelope, "entries") > 0 {
            "entries"
        } else {
            break;
        };
        let len = array_len(&envelope, key);
        if len == 0 {
            break;
        }
        truncate_array(&mut envelope, key, len / 2);
        if fits_budget(&envelope, max_bytes) {
            return envelope;
        }
    }
    if !fits_budget(&envelope, max_bytes) {
        shrink_text(&mut envelope, max_bytes);
    }
    if !fits_budget(&envelope, max_bytes) {
        let string_budget = (max_bytes / 64).max(256);
        shrink_values(&mut envelope, string_budget, 20);
    }
    if !fits_budget(&envelope, max_bytes) {
        keep_essential(&mut envelope);
    }
    if !fits_budget(&envelope, max_bytes) {
        truncate_array(&mut envelope, "results", 0);
        truncate_array(&mut envelope, "entries", 0);
        if let Some(markdown) = envelope.get_mut("markdown") {
            *markdown = Value::String(String::new());
        }
        if let Some(text) = envelope.get_mut("text") {
            *text = Value::String(String::new());
        }
    }
    if !fits_budget(&envelope, max_bytes) {
        let cost = envelope.get("cost").cloned();
        let mut minimal = serde_json::Map::new();
        minimal.insert("truncated".to_owned(), Value::Bool(true));
        if let Some(cost) = cost {
            minimal.insert("cost".to_owned(), cost);
        }
        envelope = Value::Object(minimal);
    }
    envelope
}

fn shrink_values(value: &mut Value, string_budget: usize, item_budget: usize) {
    match value {
        Value::String(text) => {
            if text.chars().count() > string_budget {
                let capped: String = text.chars().take(string_budget).collect::<String>() + "...";
                *text = capped;
            }
        }
        Value::Array(items) => {
            items.truncate(item_budget);
            for item in items.iter_mut() {
                shrink_values(item, string_budget, item_budget);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                shrink_values(item, string_budget, item_budget);
            }
        }
        _ => {}
    }
}

fn keep_essential(value: &mut Value) {
    const KEEP: [&str; 10] = [
        "results",
        "entries",
        "cost",
        "truncated",
        "status",
        "markdown",
        "text",
        "title",
        "credits",
        "info",
    ];
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, _| KEEP.contains(&key.as_str()));
        map.insert("truncated".to_owned(), Value::Bool(true));
    }
}

fn fits_budget(value: &Value, max_bytes: usize) -> bool {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= max_bytes)
}

fn array_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

fn truncate_array(value: &mut Value, key: &str, keep: usize) {
    if let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) {
        items.truncate(keep);
    }
}

fn mark_truncated(value: &mut Value) {
    if let Some(object) = value.as_object_mut() {
        object.insert("truncated".to_owned(), Value::Bool(true));
    }
}

fn shrink_text(value: &mut Value, max_bytes: usize) {
    let keep = max_bytes / 8;
    for key in ["markdown", "text"] {
        let Some(text) = value.get(key).and_then(Value::as_str) else {
            continue;
        };
        if text.len() <= keep {
            continue;
        }
        let capped: String = text.chars().take(keep).collect::<String>() + "...";
        if let Some(object) = value.as_object_mut() {
            object.insert(key.to_owned(), Value::String(capped));
        }
    }
}

pub fn summary_text(
    results: usize,
    top_title: Option<&str>,
    credits: u32,
    usd: Option<f64>,
) -> String {
    let top = cap(top_title.unwrap_or("no title"), 160).0;
    let price = usd.map_or_else(|| "usd unknown".to_owned(), |value| format!("${value:.6}"));
    format!("{results} results; top: {top}; cost: {credits} credits ({price})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_shrinks_nested_strings_and_arrays() {
        let big = "x".repeat(4096);
        let value = serde_json::json!({
            "results": [{"title": big, "attributes": {"k": big}}],
            "cost": {"credits": 1}
        });
        let fitted = fit_to_budget(value, 2048);
        let size = serde_json::to_vec(&fitted).expect("serialize").len();
        assert!(size <= 2048, "fitted size was {size}");
    }

    #[test]
    fn the_budget_keeps_a_hard_cap_on_a_wide_object() {
        let mut info = serde_json::Map::new();
        for index in 0..200 {
            info.insert(format!("k{index}"), Value::String("y".repeat(200)));
        }
        let value = serde_json::json!({
            "status": "ok",
            "info": info,
            "cost": {"credits": 0}
        });
        let fitted = fit_to_budget(value, 4096);
        let size = serde_json::to_vec(&fitted).expect("serialize").len();
        assert!(size <= 4096, "fitted size was {size}");
        assert_eq!(fitted.get("truncated").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn cap_marks_the_clipped_string() {
        let (capped, truncated) = cap("abcdef", 3);
        assert_eq!(capped, "abc...");
        assert!(truncated);
        let (kept, truncated) = cap("abc", 3);
        assert_eq!(kept, "abc");
        assert!(!truncated);
    }

    #[test]
    fn cap_counts_characters_not_bytes() {
        let text = "é".repeat(400);
        let (capped, truncated) = cap(&text, 300);
        assert!(truncated);
        assert_eq!(capped.chars().count(), 303);
        assert!(capped.ends_with("..."));
    }

    #[test]
    fn sanitize_strips_separators_and_variation_supplements() {
        let text = sanitize("a\u{2028}b\u{E0100}c\u{180B}d");
        assert!(!text.contains('\u{2028}'));
        assert!(!text.contains('\u{E0100}'));
        assert!(!text.contains('\u{180B}'));
    }

    #[test]
    fn a_key_collision_keeps_the_first_value() {
        let value: Value = serde_json::from_str(r#"{"a":1,"a\u200B":2}"#).expect("json");
        let cleaned = sanitize_value(&value);
        assert_eq!(cleaned.get("a").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn sanitize_collapses_whitespace_and_keeps_text() {
        assert_eq!(sanitize("  a\n\tb  "), "a b");
    }

    #[test]
    fn the_standard_preset_caps_snippets_and_sections() {
        let raw = serde_json::json!({
            "organic": [{
                "title": "t",
                "link": "l",
                "position": 1,
                "snippet": "x".repeat(400)
            }],
            "relatedSearches": (0..9).map(|i| serde_json::json!({"query": format!("q{i}")})).collect::<Vec<_>>()
        });
        let (items, truncated) = organic(&raw, Preset::Standard);
        let first = items.first().expect("one item");
        let snippet = first.snippet.as_deref().expect("snippet kept");
        assert_eq!(snippet.chars().count(), 303);
        assert!(snippet.ends_with("..."));
        assert_eq!(first.truncated, Some(true));
        assert!(truncated);
        let sections = search_sections(&raw, Preset::Standard);
        assert_eq!(sections.related_searches.map(|items| items.len()), Some(6));
    }

    #[test]
    fn the_minimal_preset_keeps_rank_title_and_link() {
        let raw = serde_json::json!({
            "organic": [{"title": "t", "link": "l", "position": 2, "snippet": "s", "date": "d"}]
        });
        let (items, _) = organic(&raw, Preset::Minimal);
        let first = items.first().expect("one item");
        assert_eq!(first.position, Some(2));
        assert!(first.snippet.is_none());
        assert!(first.date.is_none());
    }

    #[test]
    fn the_budget_guard_halves_results_until_it_fits() {
        let envelope = serde_json::json!({
            "results": (0..64).map(|i| serde_json::json!({"title": i.to_string().repeat(20)})).collect::<Vec<_>>(),
            "truncated": false
        });
        let fitted = fit_to_budget(envelope, 512);
        let kept = fitted
            .get("results")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        assert!(kept < 64);
        assert_eq!(
            fitted.get("truncated"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(serde_json::to_vec(&fitted).expect("serialize").len() <= 512);
    }
}

#[cfg(test)]
mod search_item_tests {
    use super::*;
    use serde_json::json;

    fn raw() -> Value {
        json!({
            "images": [{"title":"i","imageUrl":"u","thumbnailUrl":"t","imageWidth":10,"imageHeight":20,"source":"s","domain":"d","googleUrl":"g","link":"l","position":1}],
            "news": [{"title":"n","link":"l","snippet":"s","date":"d","source":"src","imageUrl":"i","position":1}],
            "places": [{"title":"p","address":"a","latitude":1.5,"longitude":2.5,"rating":4.5,"ratingCount":10,"type":"t","phone":"ph","website":"w","cid":"c","category":"cat","description":"desc","types":["t1","t2"],"position":1}],
            "videos": [{"title":"v","link":"l","snippet":"s","duration":"1:00","channel":"c","channelLink":"cl","date":"d","imageUrl":"i","position":1}],
            "shopping": [{"title":"sh","price":"$1","link":"l","source":"src","delivery":"d","imageUrl":"i","position":1}],
            "patents": [{"title":"pt","link":"l","snippet":"s","assignee":"a","publicationNumber":"1","filingDate":"f","grantDate":"g","priorityDate":"p","position":1}],
            "suggestions": [{"value":"s1"}, {"value":"s2"}],
            "reviews": [{"rating":5.0,"date":"d","snippet":"s","author":"a","title":"t","position":1}],
            "visualMatches": [{"title":"l","link":"l","source":"s","thumbnailUrl":"t","position":1}]
        })
    }

    #[test]
    fn every_family_projects_its_own_array_in_standard_mode() {
        let raw = raw();
        assert_eq!(images(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(news(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(places(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(videos(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(shopping(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(patents(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(suggestions(&raw).len(), 2);
        assert_eq!(reviews(&raw, Preset::Standard).0.len(), 1);
        assert_eq!(lens(&raw, Preset::Standard).0.len(), 1);
    }

    #[test]
    fn patents_reads_the_organic_array_the_api_returns() {
        let raw = serde_json::json!({
            "organic": [{
                "title": "Wireless charging pad",
                "link": "https://patents.google.com/patent/US123",
                "snippet": "A charging pad.",
                "publicationNumber": "US123",
                "assignee": "Example Corp",
                "position": 1
            }]
        });
        let (items, _) = patents(&raw, Preset::Standard);
        let first = items.first().expect("patent item");
        assert_eq!(first.publication_number.as_deref(), Some("US123"));
        assert_eq!(first.assignee.as_deref(), Some("Example Corp"));
    }

    #[test]
    fn the_full_preset_keeps_fields_the_standard_preset_drops() {
        let raw = raw();
        let (images_std, _) = images(&raw, Preset::Standard);
        let (images_full, _) = images(&raw, Preset::Full);
        let std_image = images_std.first().expect("image");
        let full_image = images_full.first().expect("image");
        assert!(std_image.google_url.is_none());
        assert_eq!(full_image.google_url.as_deref(), Some("g"));

        let (places_std, _) = places(&raw, Preset::Standard);
        let (places_full, _) = places(&raw, Preset::Full);
        assert!(
            places_std
                .first()
                .and_then(|place| place.types.as_ref())
                .is_none()
        );
        assert_eq!(
            places_full
                .first()
                .and_then(|place| place.types.as_ref())
                .map(Vec::len),
            Some(2)
        );

        let (shopping_std, _) = shopping(&raw, Preset::Standard);
        let (shopping_full, _) = shopping(&raw, Preset::Full);
        assert!(
            shopping_std
                .first()
                .and_then(|shopping_item| shopping_item.image_url.as_ref())
                .is_none()
        );
        assert!(
            shopping_full
                .first()
                .and_then(|shopping_item| shopping_item.image_url.as_ref())
                .is_some()
        );
    }

    #[test]
    fn the_minimal_preset_keeps_only_the_rank_and_identity_fields() {
        let raw = raw();
        let (images_min, _) = images(&raw, Preset::Minimal);
        let image = images_min.first().expect("image");
        assert!(image.thumbnail_url.is_none());
        assert!(image.source.is_none());
        assert_eq!(image.position, Some(1));
        let (places_min, _) = places(&raw, Preset::Minimal);
        let place = places_min.first().expect("place");
        assert!(place.phone.is_none());
        assert_eq!(place.address.as_deref(), Some("a"));
    }

    #[test]
    fn passthrough_fields_are_sanitized() {
        let raw = serde_json::json!({
            "organic": [{
                "title": "t",
                "sitelinks": [{"title": "a\u{200B}b"}],
                "attributes": {"k\u{FEFF}": "v\u{202E}"}
            }],
            "knowledgeGraph": {"title": "k", "attributes": {"x": "y\u{061C}"}},
            "searchParameters": {"q": "q\u{180E}q"}
        });
        let (items, _) = organic(&raw, Preset::Full);
        let first = items.first().expect("item");
        let sitelinks = first.sitelinks.as_ref().expect("sitelinks");
        assert!(
            !sitelinks
                .iter()
                .any(|item| item.to_string().contains('\u{200B}'))
        );
        let attributes = first.attributes.as_ref().expect("attributes");
        assert!(!attributes.to_string().contains('\u{FEFF}'));
        assert!(!attributes.to_string().contains('\u{202E}'));

        let sections = search_sections(&raw, Preset::Full);
        let graph = sections.knowledge_graph.expect("graph");
        assert!(
            !graph
                .attributes
                .unwrap_or_default()
                .to_string()
                .contains('\u{061C}')
        );
        let params = sections.search_parameters.expect("params");
        assert!(!params.to_string().contains('\u{180E}'));
    }

    #[test]
    fn full_sections_keep_the_parameters_and_attributes() {
        let raw = json!({
            "searchParameters": {"q": "x"},
            "knowledgeGraph": {"title": "t", "attributes": {"a": "b"}},
            "answerBox": {"title": "a", "answer": "yes", "list": ["one", "two"]},
            "peopleAlsoAsk": [{"question": "q"}],
            "relatedSearches": [{"query": "r"}]
        });
        let sections = search_sections(&raw, Preset::Full);
        assert!(sections.search_parameters.is_some());
        assert!(
            sections
                .knowledge_graph
                .as_ref()
                .and_then(|graph| graph.attributes.as_ref())
                .is_some()
        );
        assert_eq!(
            sections
                .answer_box
                .as_ref()
                .and_then(|answer_box| answer_box.list.as_ref())
                .map(Vec::len),
            Some(2)
        );
    }

    #[test]
    fn scrape_content_reads_the_markdown_and_the_reported_credits() {
        let raw = json!({"markdown": "# Title", "text": "plain", "credits": 6, "images": ["i"], "links": ["l"], "videos": ["v"]});
        let standard = scrape_content(&raw, Preset::Standard);
        assert_eq!(standard.markdown.as_deref(), Some("# Title"));
        assert_eq!(standard.text.as_deref(), Some("plain"));
        assert_eq!(standard.credits, Some(6));
        assert!(standard.images.is_some());
        assert!(standard.videos.is_none());

        let full = scrape_content(&raw, Preset::Full);
        assert!(full.videos.is_some());

        let minimal = scrape_content(&raw, Preset::Minimal);
        assert!(minimal.text.is_none());
        assert!(minimal.links.is_none());
    }

    #[test]
    fn locations_accept_a_bare_array_or_a_wrapped_one() {
        let bare = json!([{"name": "Dhaka", "canonicalName": "Dhaka,BD"}]);
        assert_eq!(locations(&bare).len(), 1);
        let wrapped = json!({"locations": [{"name": "Dhaka"}]});
        assert_eq!(locations(&wrapped).len(), 1);
    }

    #[test]
    fn the_summary_line_carries_the_count_and_the_cost() {
        let line = summary_text(3, Some("Top"), 2, Some(0.0015));
        assert!(line.starts_with("3 results"));
        assert!(line.contains("Top"));
        assert!(line.contains("2 credits"));
        assert!(line.contains("$0.001500"));
    }

    #[test]
    fn an_unknown_rate_is_named_in_the_summary() {
        let line = summary_text(1, None, 2, None);
        assert!(line.contains("usd unknown"));
    }
}
