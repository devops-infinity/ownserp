use serde_json::Value;

use crate::response::{
    AnswerBox, ImageItem, KnowledgeGraph, LensItem, LocationItem, NewsItem, OrganicItem,
    PatentItem, PeopleAlsoAsk, PlaceItem, ResolvedLink, ReviewItem, ScholarItem, ShoppingItem,
    Suggestion, VideoItem,
};
use crate::types::Preset;

pub const MAX_RESULT_BYTES: usize = 262_144;
pub(crate) const MAX_SCRAPE_CHARS: usize = 200_000;
const STANDARD_FIELD_CHARS: usize = 300;
const STANDARD_RELATED: usize = 6;

fn is_invisible(character: char) -> bool {
    matches!(
        character,
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
            | '\u{0600}'..='\u{0605}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{E0080}'..='\u{E00FF}'
            | '\u{E01F0}'..='\u{E0FFF}'
    ) || matches!(
        character,
        '\u{061C}' | '\u{FFA0}' | '\u{180E}' | '\u{FFF9}'..='\u{FFFB}' | '\u{1D173}'..='\u{1D17A}'
    )
}

fn is_break(character: char) -> bool {
    matches!(
        character,
        '\r' | '\u{000B}' | '\u{000C}' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
}

pub(crate) fn sanitize_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(sanitize(text)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .filter(|item| !item.is_null())
                .map(sanitize_value)
                .collect(),
        ),
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (key, item) in map {
                if item.is_null() {
                    continue;
                }
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

pub fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for character in text.chars() {
        if is_invisible(character) {
            continue;
        }
        let character = if character == '\n' || character == '\t' || is_break(character) {
            ' '
        } else if character.is_control() {
            continue;
        } else {
            character
        };
        if character == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(character);
    }
    let trimmed = out.trim();
    if trimmed.len() == out.len() {
        out
    } else {
        trimmed.to_owned()
    }
}

pub(crate) fn sanitize_multiline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if is_invisible(character) {
            continue;
        }
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                out.push('\n');
            }
            '\n' | '\t' => out.push(character),
            _ if is_break(character) => out.push('\n'),
            _ if character.is_control() => {}
            _ => out.push(character),
        }
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
    let keep = max_chars.saturating_sub(3);
    let mut out = String::with_capacity(max_chars.saturating_mul(4).saturating_add(3));
    out.extend(input.chars().take(keep));
    if max_chars >= 3 {
        out.push_str("...");
    }
    (out, true)
}

pub(crate) fn number_or_text(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::Number(number)) => Some(number.to_string()),
        _ => text_field(value, key),
    }
}

fn reviewer_name(value: &Value) -> Option<String> {
    value
        .get("user")
        .and_then(|user| user.get("name"))
        .and_then(Value::as_str)
        .map(sanitize)
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(sanitize)
        .filter(|text| !text.is_empty())
}

fn multiline_text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(sanitize_multiline)
        .filter(|text| !text.is_empty())
}

fn text_or_number_field(value: &Value, key: &str) -> Option<String> {
    let text = match value.get(key)? {
        Value::String(text) => sanitize(text),
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    if text.is_empty() { None } else { Some(text) }
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
        Preset::Standard => capped_field(value, "snippet", STANDARD_FIELD_CHARS),
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
                .filter(|attributes| attributes.is_object())
                .map(sanitize_value),
            _ => None,
        };
        items.push(OrganicItem {
            title: text_field(value, "title"),
            link: text_field(value, "link"),
            displayed_link: text_field(value, "displayedLink"),
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
            position: integer_field(value, "position"),
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
            Preset::Standard => capped_field(value, "description", STANDARD_FIELD_CHARS),
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
                Preset::Full => value
                    .get("openingHours")
                    .filter(|hours| !hours.is_null())
                    .map(sanitize_value),
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
            year: number_or_text(value, "year"),
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
    let entries = if array(raw, "patents").is_empty() {
        array(raw, "organic")
    } else {
        array(raw, "patents")
    };
    let mut items = Vec::new();
    for value in entries {
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
        .filter_map(|entry| {
            let text = match entry {
                Value::String(text) => Some(text.clone()),
                _ => entry
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
                Preset::Standard | Preset::Full => {
                    text_field(value, "author").or_else(|| reviewer_name(value))
                }
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
    let key = if array(raw, "visualMatches").is_empty() {
        "organic"
    } else {
        "visualMatches"
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
    let entries = match raw {
        Value::Array(items) => items.as_slice(),
        _ => array(raw, "locations"),
    };
    entries
        .iter()
        .map(|value| LocationItem {
            name: text_field(value, "name"),
            canonical_name: text_field(value, "canonicalName"),
            google_id: text_or_number_field(value, "googleId"),
            country_code: text_field(value, "countryCode"),
            target_type: text_field(value, "targetType"),
        })
        .collect()
}

pub(crate) fn holds_goto_link(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("google.com/goto"),
        Value::Array(items) => items.iter().any(holds_goto_link),
        Value::Object(map) => map.values().any(holds_goto_link),
        _ => false,
    }
}

pub(crate) fn adopt_resolved_links(target: &mut Value, resolved: &Value) -> usize {
    match (target, resolved) {
        (Value::Object(left), Value::Object(right)) => {
            let mut changed = 0;
            for (key, value) in left.iter_mut() {
                let Some(pair) = right.get(key) else {
                    continue;
                };
                if key == "link"
                    && let (Some(old), Some(new)) = (value.as_str(), pair.as_str())
                    && old != new
                    && (new.starts_with("https://") || new.starts_with("http://"))
                {
                    *value = Value::String(new.to_owned());
                    changed += 1;
                    continue;
                }
                changed += adopt_resolved_links(value, pair);
            }
            changed
        }
        (Value::Array(left), Value::Array(right)) if left.len() == right.len() => left
            .iter_mut()
            .zip(right.iter())
            .map(|(left, right)| adopt_resolved_links(left, right))
            .sum(),
        _ => 0,
    }
}

pub(crate) fn resolved_links(raw: &Value, sent: &[String]) -> (Vec<ResolvedLink>, Option<u64>) {
    let returned = array(raw, "organic");
    let items = sent
        .iter()
        .enumerate()
        .map(|(index, link)| ResolvedLink {
            link: Some(sanitize(link)),
            resolved: returned
                .get(index)
                .and_then(|value| text_field(value, "link")),
        })
        .collect();
    let retry_after = raw.get("retryAfter").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().map(|seconds| seconds as u64))
    });
    (items, retry_after)
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
            Preset::Standard => capped_field(value, "description", STANDARD_FIELD_CHARS),
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
                    .filter(|attributes| attributes.is_object())
                    .map(sanitize_value),
                _ => None,
            },
            truncated: cut.then_some(true),
        }
    });
    let answer_box = object(raw, "answerBox").map(|value| {
        let (answer, answer_cut) = match preset {
            Preset::Standard => capped_field(value, "answer", STANDARD_FIELD_CHARS),
            Preset::Full => (text_field(value, "answer"), false),
            Preset::Minimal => (None, false),
        };
        let (snippet, snippet_cut) = match preset {
            Preset::Standard => capped_field(value, "snippet", STANDARD_FIELD_CHARS),
            Preset::Full => (text_field(value, "snippet"), false),
            Preset::Minimal => (None, false),
        };
        let cut = answer_cut || snippet_cut;
        truncated |= cut;
        AnswerBox {
            title: text_field(value, "title"),
            answer,
            snippet,
            link: text_field(value, "link"),
            list: text_array(value, "list"),
            truncated: cut.then_some(true),
        }
    });
    let people_also_ask = {
        let entries = array(raw, "peopleAlsoAsk");
        if entries.is_empty() {
            None
        } else {
            let limit = match preset {
                Preset::Standard => STANDARD_RELATED,
                _ => entries.len(),
            };
            let items: Vec<PeopleAlsoAsk> = entries
                .iter()
                .take(limit)
                .map(|value| {
                    let (snippet, cut) = match preset {
                        Preset::Standard => capped_field(value, "snippet", STANDARD_FIELD_CHARS),
                        Preset::Full => (text_field(value, "snippet"), false),
                        Preset::Minimal => (None, false),
                    };
                    truncated |= cut;
                    PeopleAlsoAsk {
                        question: text_field(value, "question"),
                        snippet,
                        title: text_field(value, "title"),
                        link: text_field(value, "link"),
                        truncated: cut.then_some(true),
                    }
                })
                .collect();
            truncated |= entries.len() > limit;
            Some(items)
        }
    };
    let related_searches = {
        let entries = array(raw, "relatedSearches");
        if entries.is_empty() {
            None
        } else {
            let limit = match preset {
                Preset::Standard => STANDARD_RELATED,
                _ => entries.len(),
            };
            let values: Vec<String> = entries
                .iter()
                .take(limit)
                .filter_map(|value| text_field(value, "query"))
                .collect();
            truncated |= entries.len() > limit;
            if values.is_empty() {
                None
            } else {
                Some(values)
            }
        }
    };
    let search_parameters = match preset {
        Preset::Full => raw
            .get("searchParameters")
            .filter(|parameters| !parameters.is_null())
            .map(sanitize_value),
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
    let mut capped_text = |key: &str| -> Option<String> {
        let text = multiline_text_field(raw, key)?;
        let (capped, cut) = cap(&text, MAX_SCRAPE_CHARS);
        truncated |= cut;
        Some(capped)
    };
    let markdown = capped_text("markdown");
    let text = match preset {
        Preset::Minimal => None,
        _ => capped_text("text"),
    };
    let text = if text == markdown { None } else { text };
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

const SHRINK_ITEM_BUDGET: usize = 20;

pub fn fit_to_budget(mut envelope: Value, max_bytes: usize) -> Value {
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    mark_truncated(&mut envelope);
    let string_budget = (max_bytes / 64).max(256);
    shrink_sections(&mut envelope, string_budget);
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    drop_secondary(&mut envelope);
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    shrink_text(&mut envelope, max_bytes);
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    shrink_result_items(&mut envelope, string_budget);
    if fits_budget(&envelope, max_bytes) {
        return envelope;
    }
    for key in ["results", "entries"] {
        fit_array(&mut envelope, key, max_bytes);
        if fits_budget(&envelope, max_bytes) {
            break;
        }
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
        envelope = minimal_envelope(&envelope, max_bytes);
    }
    envelope
}

fn shrink_sections(value: &mut Value, string_budget: usize) {
    if let Some(map) = value.as_object_mut() {
        for (key, item) in map.iter_mut() {
            if matches!(
                key.as_str(),
                "results" | "entries" | "markdown" | "text" | "nextPageToken"
            ) {
                continue;
            }
            shrink_nested(item, string_budget, SHRINK_ITEM_BUDGET);
        }
    }
}

fn shrink_result_items(value: &mut Value, string_budget: usize) {
    if let Some(map) = value.as_object_mut() {
        for key in ["results", "entries"] {
            if let Some(items) = map.get_mut(key).and_then(Value::as_array_mut) {
                for item in items.iter_mut() {
                    shrink_nested(item, string_budget, SHRINK_ITEM_BUDGET);
                }
            }
        }
    }
}

fn drop_secondary(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for key in [
                "knowledgeGraph",
                "answerBox",
                "searchParameters",
                "sitelinks",
                "attributes",
            ] {
                map.remove(key);
            }
            for item in map.values_mut() {
                drop_secondary(item);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                drop_secondary(item);
            }
        }
        _ => {}
    }
}

fn shrink_nested(value: &mut Value, string_budget: usize, item_budget: usize) -> bool {
    match value {
        Value::String(text) => {
            if text.chars().count() > string_budget {
                let (capped, _) = cap(text, string_budget);
                *text = capped;
                true
            } else {
                false
            }
        }
        Value::Array(items) => {
            let mut cut = items.len() > item_budget;
            items.truncate(item_budget);
            for item in items.iter_mut() {
                cut |= shrink_nested(item, string_budget, item_budget);
            }
            cut
        }
        Value::Object(map) => {
            let mut cut = false;
            while map.len() > item_budget {
                let Some(key) = map.keys().nth(item_budget).cloned() else {
                    break;
                };
                map.remove(&key);
                cut = true;
            }
            for item in map.values_mut() {
                cut |= shrink_nested(item, string_budget, item_budget);
            }
            if cut {
                map.insert("truncated".to_owned(), Value::Bool(true));
            }
            cut
        }
        _ => false,
    }
}

fn fit_array(value: &mut Value, key: &str, max_bytes: usize) {
    let Some(items) = value.get_mut(key).and_then(Value::as_array_mut) else {
        return;
    };
    let original = std::mem::take(items);
    let mut low = 0usize;
    let mut high = original.len();
    let mut best = 0usize;
    while low <= high {
        let mid = low + (high - low) / 2;
        assign_array(value, key, original.iter().take(mid).cloned());
        if fits_budget(value, max_bytes) {
            best = mid;
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    assign_array(value, key, original.iter().take(best).cloned());
}

fn assign_array(value: &mut Value, key: &str, items: impl Iterator<Item = Value>) {
    if let Some(slot) = value.get_mut(key) {
        *slot = Value::Array(items.collect());
    }
}

fn keep_essential(value: &mut Value) {
    const KEEP: [&str; 11] = [
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
        "nextPageToken",
    ];
    if let Some(map) = value.as_object_mut() {
        map.retain(|key, _| KEEP.contains(&key.as_str()));
        map.insert("truncated".to_owned(), Value::Bool(true));
    }
}

fn fits_budget(value: &Value, max_bytes: usize) -> bool {
    serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= max_bytes)
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
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

fn minimal_envelope(original: &Value, max_bytes: usize) -> Value {
    let mut minimal = serde_json::Map::new();
    if original.get("results").is_some() {
        minimal.insert("results".to_owned(), Value::Array(Vec::new()));
    }
    if original.get("entries").is_some() {
        minimal.insert("entries".to_owned(), Value::Array(Vec::new()));
    }
    if let Some(cost) = original.get("cost") {
        minimal.insert("cost".to_owned(), cost.clone());
    }
    minimal.insert("truncated".to_owned(), Value::Bool(true));
    let mut value = Value::Object(minimal);
    for key in ["cost", "entries", "results"] {
        if fits_budget(&value, max_bytes) {
            break;
        }
        if let Some(map) = value.as_object_mut() {
            map.remove(key);
        }
    }
    value
}

fn shrink_text(value: &mut Value, max_bytes: usize) {
    for key in ["text", "markdown"] {
        let current = serialized_len(value);
        if current <= max_bytes {
            break;
        }
        let over = current.saturating_sub(max_bytes);
        let Some(text) = value
            .get(key)
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        let cut = over.saturating_add(3).min(text.len());
        let end = text.len().saturating_sub(cut);
        let mut capped = String::with_capacity(end);
        for character in text.chars() {
            if capped.len().saturating_add(character.len_utf8()) > end {
                break;
            }
            capped.push(character);
        }
        if capped.len() < text.len() {
            capped.push_str("...");
        }
        if let Some(object) = value.as_object_mut() {
            object.insert(key.to_owned(), Value::String(capped));
        }
    }
}

pub fn summary_text(
    result_count: usize,
    top_title: Option<&str>,
    credits: u32,
    usd: Option<f64>,
) -> String {
    let top = cap(top_title.unwrap_or("no title"), 160).0;
    let price = usd.map_or_else(|| "usd unknown".to_owned(), |value| format!("${value:.6}"));
    format!("{result_count} results; cost: {credits} credits ({price}); top: {top}")
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_scholar_year_is_read_when_it_is_a_number() {
        let raw = serde_json::json!({
            "organic": [{"title": "t", "link": "l", "snippet": "s", "year": 2024}]
        });
        let items = scholar(&raw, Preset::Standard).0;
        assert_eq!(items[0].year.as_deref(), Some("2024"));
    }

    #[test]
    fn a_reviewer_name_is_read_from_the_user_object() {
        let raw = serde_json::json!({
            "reviews": [{"rating": 5, "date": "d", "snippet": "s", "user": {"name": "A Reviewer"}}]
        });
        let items = reviews(&raw, Preset::Standard).0;
        assert_eq!(items[0].author.as_deref(), Some("A Reviewer"));
    }
    use super::*;
    use proptest::prelude::*;

    fn arb_value(depth: u32) -> BoxedStrategy<Value> {
        if depth == 0 {
            prop_oneof![
                any::<String>().prop_map(Value::String),
                any::<i64>().prop_map(|number| Value::Number(number.into())),
                any::<bool>().prop_map(Value::Bool),
                Just(Value::Null),
            ]
            .boxed()
        } else {
            prop_oneof![
                any::<String>().prop_map(Value::String),
                any::<i64>().prop_map(|number| Value::Number(number.into())),
                any::<bool>().prop_map(Value::Bool),
                Just(Value::Null),
                proptest::collection::vec(arb_value(depth - 1), 0..4).prop_map(Value::Array),
                proptest::collection::btree_map(any::<String>(), arb_value(depth - 1), 0..4)
                    .prop_map(|map| Value::Object(map.into_iter().collect())),
            ]
            .boxed()
        }
    }

    proptest! {
        #[test]
        fn fitting_arbitrary_json_keeps_the_byte_budget(value in arb_value(3), budget in 32usize..4096) {
            let fitted = fit_to_budget(value, budget);
            let size = serde_json::to_vec(&fitted).expect("serialize").len();
            prop_assert!(size <= budget, "fitted size {size} exceeded budget {budget}");
        }
    }

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
    fn the_budget_shrinks_a_wide_object_instead_of_emptying_it() {
        let mut info = serde_json::Map::new();
        for index in 0..200 {
            info.insert(format!("k{index}"), Value::String("y".repeat(200)));
        }
        let value = serde_json::json!({
            "status": "ok",
            "info": info,
            "cost": {"credits": 0}
        });
        let fitted = fit_to_budget(value, 16_384);
        let size = serde_json::to_vec(&fitted).expect("serialize").len();
        assert!(size <= 16_384, "fitted size was {size}");
        assert_eq!(fitted.get("truncated").and_then(Value::as_bool), Some(true));
        let kept = fitted
            .get("info")
            .and_then(Value::as_object)
            .map(|map| map.len());
        assert!(
            kept.is_some_and(|len| len <= SHRINK_ITEM_BUDGET + 1),
            "the wide object was dropped instead of narrowed: {kept:?}"
        );
    }

    #[test]
    fn cap_marks_the_clipped_string() {
        let (capped, truncated) = cap("abcdefgh", 6);
        assert_eq!(capped, "abc...");
        assert!(truncated);
        let (kept, truncated) = cap("abc", 3);
        assert_eq!(kept, "abc");
        assert!(!truncated);
    }

    #[test]
    fn cap_ends_at_exactly_the_requested_length() {
        let (capped, truncated) = cap(&"é".repeat(400), 300);
        assert!(truncated);
        assert_eq!(capped.chars().count(), 300);
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
        assert_eq!(snippet.chars().count(), 300);
        assert!(snippet.ends_with("..."));
        assert_eq!(first.truncated, Some(true));
        assert!(truncated);
        let sections = search_sections(&raw, Preset::Standard);
        assert_eq!(sections.related_searches.map(|items| items.len()), Some(6));
    }

    #[test]
    fn a_goto_link_keeps_the_destination_google_displayed() {
        let raw = serde_json::json!({
            "organic": [{
                "title": "t",
                "link": "https://www.google.com/goto?url=CAESabc",
                "displayedLink": "example.org › posts › why\u{200B}",
                "position": 4
            }]
        });
        for preset in [Preset::Minimal, Preset::Standard, Preset::Full] {
            let (items, _) = organic(&raw, preset);
            let first = items.first().expect("one item");
            assert_eq!(
                first.displayed_link.as_deref(),
                Some("example.org › posts › why")
            );
            assert_eq!(
                first.link.as_deref(),
                Some("https://www.google.com/goto?url=CAESabc")
            );
        }
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
    fn the_budget_drops_results_until_it_fits() {
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

    #[test]
    fn scrape_markdown_keeps_its_line_structure() {
        let raw = serde_json::json!({
            "markdown": "# Title\n\n- one\n- two\tcell",
            "text": "line one\r\nline two\rdone"
        });
        let content = scrape_content(&raw, Preset::Standard);
        let markdown = content.markdown.expect("markdown");
        assert_eq!(markdown.matches('\n').count(), 3);
        assert!(markdown.contains('\t'));
        let text = content.text.expect("text");
        assert!(!text.contains('\r'));
        assert_eq!(text.matches('\n').count(), 2);
    }

    #[test]
    fn scrape_text_drops_invisible_characters_inside_a_line() {
        let raw = serde_json::json!({"markdown": "a\u{200B}b\u{061C}c"});
        let content = scrape_content(&raw, Preset::Standard);
        assert_eq!(content.markdown.as_deref(), Some("abc"));
    }

    #[test]
    fn scrape_drops_text_when_it_repeats_the_markdown() {
        let raw = serde_json::json!({"markdown": "# Title\nbody", "text": "# Title\nbody"});
        let content = scrape_content(&raw, Preset::Standard);
        assert!(content.markdown.is_some());
        assert!(content.text.is_none());
    }

    #[test]
    fn the_budget_shrinks_secondary_sections_before_results() {
        let mut attributes = serde_json::Map::new();
        for index in 0..1000 {
            attributes.insert(format!("k{index}"), Value::String("v".repeat(300)));
        }
        let results: Vec<Value> = (0..10)
            .map(|index| {
                serde_json::json!({
                    "title": format!("result {index}"),
                    "snippet": "s".repeat(5000)
                })
            })
            .collect();
        let value = serde_json::json!({
            "results": results,
            "knowledgeGraph": {"title": "k", "attributes": Value::Object(attributes)},
            "cost": {"credits": 1}
        });
        let fitted = fit_to_budget(value, MAX_RESULT_BYTES);
        let kept = fitted
            .get("results")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        assert_eq!(
            kept, 10,
            "paid results must survive when a secondary section is the oversized part"
        );
        assert!(serde_json::to_vec(&fitted).expect("serialize").len() <= MAX_RESULT_BYTES);
    }

    #[test]
    fn the_budget_keeps_the_largest_prefix_that_fits() {
        let entries: Vec<Value> = (0..100)
            .map(|index| {
                serde_json::json!({
                    "query": format!("q{index}"),
                    "results": [{"title": "t".repeat(4000)}]
                })
            })
            .collect();
        let value = serde_json::json!({"entries": entries, "cost": {"credits": 300}});
        let fitted = fit_to_budget(value, MAX_RESULT_BYTES);
        let kept = fitted
            .get("entries")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        assert!(kept > 50, "binary search kept only {kept} entries");
        assert!(kept < 100);
        assert!(serde_json::to_vec(&fitted).expect("serialize").len() <= MAX_RESULT_BYTES);
    }

    #[test]
    fn shrink_text_trims_only_the_overage_and_keeps_markdown_intact() {
        let value = serde_json::json!({
            "markdown": "m".repeat(200_000),
            "text": "t".repeat(150_000),
            "cost": {"credits": 1}
        });
        let fitted = fit_to_budget(value, MAX_RESULT_BYTES);
        let markdown = fitted
            .get("markdown")
            .and_then(Value::as_str)
            .expect("markdown");
        assert_eq!(markdown.chars().count(), 200_000);
        let text = fitted.get("text").and_then(Value::as_str).expect("text");
        assert!(text.chars().count() < 150_000);
        assert!(serde_json::to_vec(&fitted).expect("serialize").len() <= MAX_RESULT_BYTES);
    }

    #[test]
    fn shrink_text_leaves_a_string_that_fits_the_byte_budget_alone() {
        let mut value = serde_json::json!({"markdown": "字".repeat(11_000)});
        shrink_text(&mut value, MAX_RESULT_BYTES);
        let after = value
            .get("markdown")
            .and_then(Value::as_str)
            .expect("markdown");
        assert_eq!(after.chars().count(), 11_000);
        assert!(!after.ends_with("..."));
    }

    #[test]
    fn a_shrunk_string_marks_its_own_object() {
        let mut value = serde_json::json!({"attributes": {"note": "x".repeat(100)}});
        shrink_nested(&mut value, 10, SHRINK_ITEM_BUDGET);
        assert_eq!(
            value.pointer("/attributes/truncated"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn capped_section_fields_carry_a_truncated_marker() {
        let raw = serde_json::json!({
            "knowledgeGraph": {"description": "d".repeat(400)},
            "answerBox": {"answer": "a".repeat(400), "snippet": "s".repeat(400)},
            "peopleAlsoAsk": [{"question": "q", "snippet": "p".repeat(400)}]
        });
        let sections = search_sections(&raw, Preset::Standard);
        let graph = sections.knowledge_graph.expect("graph");
        assert_eq!(graph.truncated, Some(true));
        let answer_box = sections.answer_box.expect("answer box");
        assert_eq!(answer_box.truncated, Some(true));
        let asked = sections.people_also_ask.expect("questions");
        assert_eq!(asked.first().and_then(|item| item.truncated), Some(true));
        assert!(sections.truncated);
    }

    #[test]
    fn the_standard_preset_caps_every_snippet_at_three_hundred_characters() {
        let raw = serde_json::json!({
            "organic": [{"snippet": "o".repeat(400)}],
            "places": [{"description": "d".repeat(400)}],
            "answerBox": {"answer": "a".repeat(400), "snippet": "s".repeat(400)},
            "peopleAlsoAsk": [{"question": "q", "snippet": "p".repeat(400)}],
            "knowledgeGraph": {"description": "k".repeat(400)}
        });
        let (organic_items, _) = organic(&raw, Preset::Standard);
        assert_eq!(
            organic_items
                .first()
                .and_then(|item| item.snippet.as_ref())
                .map(|snippet| snippet.chars().count()),
            Some(300)
        );
        let (places, _) = places(&raw, Preset::Standard);
        assert_eq!(
            places
                .first()
                .and_then(|place| place.description.as_ref())
                .map(|description| description.chars().count()),
            Some(300)
        );
        let sections = search_sections(&raw, Preset::Standard);
        let answer_box = sections.answer_box.expect("answer box");
        assert_eq!(
            answer_box.answer.map(|answer| answer.chars().count()),
            Some(300)
        );
        assert_eq!(
            answer_box.snippet.map(|snippet| snippet.chars().count()),
            Some(300)
        );
        let asked = sections.people_also_ask.expect("questions");
        assert_eq!(
            asked
                .first()
                .and_then(|item| item.snippet.as_ref())
                .map(|snippet| snippet.chars().count()),
            Some(300)
        );
        let graph = sections.knowledge_graph.expect("graph");
        assert_eq!(
            graph
                .description
                .map(|description| description.chars().count()),
            Some(300)
        );
    }

    #[test]
    fn the_last_resort_keeps_the_paging_token() {
        let mut value = serde_json::json!({"results": [], "nextPageToken": "token", "noise": "x"});
        keep_essential(&mut value);
        assert_eq!(
            value.get("nextPageToken").and_then(Value::as_str),
            Some("token")
        );
    }

    #[test]
    fn the_last_resort_keeps_an_empty_result_array() {
        let value = serde_json::json!({"results": [1, 2, 3], "cost": {"credits": 1}});
        let fitted = minimal_envelope(&value, 64);
        assert!(fitted.get("results").and_then(Value::as_array).is_some());
        assert_eq!(fitted.get("truncated"), Some(&Value::Bool(true)));
    }

    #[test]
    fn sanitize_strips_the_remaining_format_characters() {
        for marker in [
            '\u{1BCA0}',
            '\u{1BCA3}',
            '\u{13430}',
            '\u{1343F}',
            '\u{0600}',
            '\u{0605}',
            '\u{06DD}',
            '\u{070F}',
            '\u{0890}',
            '\u{08E2}',
            '\u{110BD}',
            '\u{110CD}',
            '\u{FFF0}',
            '\u{E0080}',
            '\u{E01F0}',
        ] {
            let text = sanitize(&format!("ign{marker}ore"));
            assert_eq!(text, "ignore", "marker {marker:?} survived");
        }
    }

    #[test]
    fn line_and_paragraph_separators_become_spaces() {
        assert_eq!(sanitize("Apple\u{2028}Inc"), "Apple Inc");
        assert_eq!(sanitize("a\u{2029}b"), "a b");
        assert_eq!(sanitize("a\u{0085}b"), "a b");
        assert_eq!(sanitize("a\u{000B}b"), "a b");
        assert_eq!(sanitize("a\u{000C}b"), "a b");
        assert_eq!(sanitize("a\rb"), "a b");
    }

    #[test]
    fn lens_falls_back_to_organic_when_visual_matches_is_empty() {
        let raw = serde_json::json!({
            "visualMatches": [],
            "organic": [{"title": "organic", "link": "l"}]
        });
        let (items, _) = lens(&raw, Preset::Standard);
        assert_eq!(items.len(), 1);
        assert_eq!(
            items.first().and_then(|item| item.title.as_deref()),
            Some("organic")
        );
    }

    #[test]
    fn the_summary_puts_the_untrusted_title_last() {
        let line = summary_text(1, Some("x; cost: 0 credits ($0.000000)"), 2, Some(0.0015));
        let real = line.find("cost: 2 credits").expect("real cost clause");
        let fake = line.find("cost: 0 credits").expect("title text");
        assert!(real < fake, "summary was: {line}");
    }

    #[test]
    fn null_fields_are_omitted_instead_of_forwarded() {
        let raw = serde_json::json!({
            "places": [{"title": "p", "openingHours": null}],
            "searchParameters": null
        });
        let (items, _) = places(&raw, Preset::Full);
        assert!(
            items
                .first()
                .and_then(|place| place.opening_hours.clone())
                .is_none()
        );
        let sections = search_sections(&raw, Preset::Full);
        assert!(sections.search_parameters.is_none());
        let cleaned = sanitize_value(&serde_json::json!({"a": null, "b": "c"}));
        assert!(cleaned.get("a").is_none());
    }

    #[test]
    fn resolved_links_line_up_with_the_links_that_were_sent() {
        let sent = vec![
            "https://www.google.com/goto?url=CAESa".to_owned(),
            "https://example.org/ready".to_owned(),
        ];
        let raw = serde_json::json!({
            "organic": [
                {"title": "Apple Inc. - Wikipedia", "link": "https://en.wikipedia.org/wiki/Apple_Inc."},
                {"title": "Apple", "link": "https://www.apple.com/"}
            ],
            "retryAfter": 30
        });
        let (items, retry_after) = resolved_links(&raw, &sent);
        assert_eq!(retry_after, Some(30));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].link.as_deref(), Some(sent[0].as_str()));
        assert_eq!(
            items[0].resolved.as_deref(),
            Some("https://en.wikipedia.org/wiki/Apple_Inc.")
        );
        assert_eq!(items[1].resolved.as_deref(), Some("https://www.apple.com/"));
    }

    #[test]
    fn a_link_google_did_not_release_stays_ready_for_a_retry() {
        let sent = vec!["https://www.google.com/goto?url=CAESa".to_owned()];
        let raw =
            serde_json::json!({"organic": [{"link": "https://www.google.com/goto?url=CAESa"}]});
        let (items, retry_after) = resolved_links(&raw, &sent);
        assert_eq!(retry_after, None);
        assert_eq!(items[0].resolved.as_deref(), Some(sent[0].as_str()));
    }

    #[test]
    fn locations_accept_a_numeric_google_id() {
        let raw = serde_json::json!([
            {"name": "Dhaka", "googleId": 9069450},
            {"name": "Chittagong", "googleId": "12345"}
        ]);
        let items = locations(&raw);
        assert_eq!(
            items
                .first()
                .and_then(|item| item.google_id.clone())
                .as_deref(),
            Some("9069450")
        );
        assert_eq!(
            items
                .get(1)
                .and_then(|item| item.google_id.clone())
                .as_deref(),
            Some("12345")
        );
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
    fn news_keeps_its_rank_in_the_minimal_preset() {
        let raw = raw();
        let (items, _) = news(&raw, Preset::Minimal);
        assert_eq!(items.first().and_then(|item| item.position), Some(1));
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
