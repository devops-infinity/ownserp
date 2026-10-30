use crate::config::Settings;
use crate::cost::Endpoint;

#[derive(Debug, Clone, Copy)]
pub struct ToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub endpoint: Option<Endpoint>,
}

pub const SPECS: &[ToolSpec] = &[
    ToolSpec {
        name: "serper_search",
        title: "Google web search",
        endpoint: Some(Endpoint::Search),
        description: "Google web results with knowledge graph, answer box, and related searches. Use it for general research; use serper_news for dated stories and serper_scholar for papers.",
    },
    ToolSpec {
        name: "serper_images",
        title: "Google image search",
        endpoint: Some(Endpoint::Images),
        description: "Google image results with dimensions, source, and thumbnail links. Use it when the answer needs a picture, not prose.",
    },
    ToolSpec {
        name: "serper_news",
        title: "Google news search",
        endpoint: Some(Endpoint::News),
        description: "Google news results with source and publication date, verbatim including relative dates. Use it for current events; serper_search covers evergreen pages.",
    },
    ToolSpec {
        name: "serper_places",
        title: "Google places search",
        endpoint: Some(Endpoint::Places),
        description: "Local business results with address, rating, phone, and website, filtered by a Serper location name. Use serper_maps when a map viewport matters.",
    },
    ToolSpec {
        name: "serper_maps",
        title: "Google maps search",
        endpoint: Some(Endpoint::Maps),
        description: "Map-anchored place results using an ll viewport in @lat,lng,zoomz form. Use it when results must sit near a coordinate; serper_places takes a named location instead.",
    },
    ToolSpec {
        name: "serper_videos",
        title: "Google video search",
        endpoint: Some(Endpoint::Videos),
        description: "Google video results with duration, channel, and publish date. Use it to find talks or tutorials rather than written pages.",
    },
    ToolSpec {
        name: "serper_shopping",
        title: "Google shopping search",
        endpoint: Some(Endpoint::Shopping),
        description: "Product listings with price, seller, and delivery text. Use it for prices and availability; serper_search returns editorial pages.",
    },
    ToolSpec {
        name: "serper_scholar",
        title: "Google scholar search",
        endpoint: Some(Endpoint::Scholar),
        description: "Academic papers with publication info, year, and citation counts. Use it for research literature; serper_search returns pages about the topic.",
    },
    ToolSpec {
        name: "serper_patents",
        title: "Google patents search",
        endpoint: Some(Endpoint::Patents),
        description: "Patent records with assignee, publication number, and filing dates. Use it for prior-art checks.",
    },
    ToolSpec {
        name: "serper_autocomplete",
        title: "Google autocomplete",
        endpoint: Some(Endpoint::Autocomplete),
        description: "Query suggestions for a partial phrase, taking only q, gl, and hl. Use it to widen or correct a query before a paid search. Batch is not supported.",
    },
    ToolSpec {
        name: "serper_reviews",
        title: "Place reviews",
        endpoint: Some(Endpoint::Reviews),
        description: "Reviews for one place, selected by cid, fid, or placeId, with cursor pagination through nextPageToken. Use it after serper_places returns a place identifier. Batch is not supported.",
    },
    ToolSpec {
        name: "serper_lens",
        title: "Reverse image search",
        endpoint: Some(Endpoint::Lens),
        description: "Reverse image search for a public image URL, returning visual matches. Use it when a picture itself is the query. Batch is not supported.",
    },
    ToolSpec {
        name: "serper_scrape",
        title: "Extract page content",
        endpoint: Some(Endpoint::Scrape),
        description: "Extract readable content from one public page as Markdown, plus text when it differs from the Markdown, optionally with images, links, and videos. Use it after a search returns a URL worth reading.",
    },
    ToolSpec {
        name: "serper_locations",
        title: "Look up a Serper location",
        endpoint: Some(Endpoint::Locations),
        description: "Canonical Serper location names for a place term, to pass as the location argument elsewhere. Call it before a location-filtered search.",
    },
    ToolSpec {
        name: "serper_resolve",
        title: "Resolve Google redirect links",
        endpoint: Some(Endpoint::Resolve),
        description: "Real destination URLs for the google.com/goto links a search returns. Send the links from a recent search. It waits for Google, so it answers in about 30 seconds right after a search; when links are not ready, the response carries retryAfter and the same links are sent again after that many seconds.",
    },
    ToolSpec {
        name: "serper_search_batch",
        title: "Batch search",
        endpoint: None,
        description: "Run up to 100 queries against one search family in a single call, reporting progress per query. Disabled unless enable_batch is set in the profile.",
    },
    ToolSpec {
        name: "serper_health",
        title: "Serper service health",
        description: "Health of the Serper API from api.serper.dev, unauthenticated and free. Call it before a long run, or when another tool reports an upstream failure.",
        endpoint: None,
    },
    ToolSpec {
        name: "serper_session",
        title: "Session spend and stats",
        endpoint: None,
        description: "Call count, credits, dollars, tokens served and saved, cache hits, and the budget left for this ledger, read locally with no network call. Set live to also probe the account balance.",
    },
];

pub(crate) fn switch(spec: &ToolSpec) -> Option<&'static str> {
    (spec.name == "serper_search_batch").then_some("enable_batch")
}

pub(crate) fn enabled(spec: &ToolSpec, settings: &Settings) -> bool {
    switch(spec).is_none_or(|_| settings.enable_batch())
}

pub(crate) fn free(spec: &ToolSpec) -> bool {
    spec.endpoint
        .is_none_or(|endpoint| endpoint.base_cost().get() == 0)
}

pub(crate) fn enabled_specs(settings: &Settings) -> Vec<&'static ToolSpec> {
    SPECS
        .iter()
        .filter(|spec| enabled(spec, settings))
        .collect()
}

pub(crate) fn cost_sentence(spec: &ToolSpec) -> String {
    match spec.name {
        "serper_images" => "Costs 1 credit, doubling to 2 when num exceeds 10.".to_owned(),
        "serper_scrape" => {
            "Costs 2, 6, or 10 credits by page difficulty, while the response reports the actual figure. The session budget reserves 10 credits before the call.".to_owned()
        }
        "serper_search_batch" => "Every query costs three times its single price.".to_owned(),
        "serper_locations" | "serper_resolve" | "serper_session" | "serper_health" => {
            "Free.".to_owned()
        }
        _ => {
            let credits = spec
                .endpoint
                .map_or(0, |endpoint| endpoint.base_cost().get());
            match credits {
                0 => "Free.".to_owned(),
                1 => "Costs 1 credit.".to_owned(),
                other => format!("Costs {other} credits."),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_description_states_its_price_once() {
        let spec = SPECS
            .iter()
            .find(|spec| spec.name == "serper_lens")
            .expect("lens spec");
        assert!(!spec.description.contains("Costs 3 credits"));
        assert!(cost_sentence(spec).contains("Costs 3 credits"));
    }
}
