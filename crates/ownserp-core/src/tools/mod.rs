pub mod args;
mod call;
pub(crate) mod specs;

pub use call::{ToolContext, call};

use std::sync::Arc;

use rmcp::handler::server::tool::{schema_for_input, schema_for_output};
use rmcp::model::{JsonObject, Tool, ToolAnnotations};
use schemars::JsonSchema;
use serde_json::Value;

use crate::config::Settings;
use crate::error::{Error, Result};
use crate::response::{
    BatchOutput, Envelope, ImageItem, LensItem, ListOutput, LocationItem, NewsItem, OrganicItem,
    PatentItem, PlaceItem, ReviewItem, ScholarItem, ScrapeOutput, SessionOutput, ShoppingItem,
    Suggestion, VideoItem,
};
use crate::tools::args::{
    BatchArgs, LensArgs, LocationsArgs, ReviewsArgs, ScrapeArgs, SearchArgs, SessionArgs,
};
use crate::tools::specs::ToolSpec;

pub(crate) fn definitions(settings: &Settings) -> Result<Vec<Tool>> {
    specs::enabled_specs(settings)
        .into_iter()
        .map(build_tool)
        .collect()
}

fn build_tool(spec: &'static ToolSpec) -> Result<Tool> {
    let schema = match spec.name {
        "serper_search"
        | "serper_images"
        | "serper_news"
        | "serper_places"
        | "serper_maps"
        | "serper_videos"
        | "serper_shopping"
        | "serper_scholar"
        | "serper_patents"
        | "serper_autocomplete" => input_schema::<SearchArgs>()?,
        "serper_reviews" => input_schema::<ReviewsArgs>()?,
        "serper_lens" => input_schema::<LensArgs>()?,
        "serper_scrape" => input_schema::<ScrapeArgs>()?,
        "serper_locations" => input_schema::<LocationsArgs>()?,
        "serper_search_batch" => input_schema::<BatchArgs>()?,
        "serper_session" => input_schema::<SessionArgs>()?,
        other => {
            return Err(Error::Config(format!(
                "tool {other} has no argument schema"
            )));
        }
    };
    let description = format!("{} {}", spec.description, specs::cost_sentence(spec));
    let tool = Tool::new(spec.name, description, schema)
        .with_title(spec.title)
        .annotate(
            ToolAnnotations::new()
                .read_only(true)
                .destructive(false)
                .idempotent(matches!(spec.name, "serper_locations" | "serper_session"))
                .open_world(true),
        );
    Ok(match spec.name {
        "serper_search" => attach_output_schema::<Envelope<OrganicItem>>(tool),
        "serper_images" => attach_output_schema::<ListOutput<ImageItem>>(tool),
        "serper_news" => attach_output_schema::<ListOutput<NewsItem>>(tool),
        "serper_places" | "serper_maps" => attach_output_schema::<ListOutput<PlaceItem>>(tool),
        "serper_videos" => attach_output_schema::<ListOutput<VideoItem>>(tool),
        "serper_shopping" => attach_output_schema::<ListOutput<ShoppingItem>>(tool),
        "serper_scholar" => attach_output_schema::<ListOutput<ScholarItem>>(tool),
        "serper_patents" => attach_output_schema::<ListOutput<PatentItem>>(tool),
        "serper_autocomplete" => attach_output_schema::<ListOutput<Suggestion>>(tool),
        "serper_reviews" => attach_output_schema::<ListOutput<ReviewItem>>(tool),
        "serper_lens" => attach_output_schema::<ListOutput<LensItem>>(tool),
        "serper_locations" => attach_output_schema::<ListOutput<LocationItem>>(tool),
        "serper_scrape" => attach_output_schema::<ScrapeOutput>(tool),
        "serper_search_batch" => attach_output_schema::<BatchOutput>(tool),
        "serper_session" => attach_output_schema::<SessionOutput>(tool),
        _ => tool,
    })
}

fn attach_output_schema<T: JsonSchema + 'static>(tool: Tool) -> Tool {
    let schema = strip_meta(schema_for_output::<T>());
    tool.with_raw_output_schema(schema)
}

fn strip_meta(schema: Arc<JsonObject>) -> Arc<JsonObject> {
    let mut object = schema.as_ref().clone();
    object.remove("$schema");
    Arc::new(object)
}

fn input_schema<T: JsonSchema + 'static>() -> Result<Arc<JsonObject>> {
    let schema =
        schema_for_input::<T>().map_err(|error| Error::Config(format!("input schema: {error}")))?;
    let mut object = schema.as_ref().clone();
    object.remove("$schema");
    object.insert("additionalProperties".to_owned(), Value::Bool(false));
    Ok(Arc::new(object))
}
