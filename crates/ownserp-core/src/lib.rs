#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

mod cache;
mod client;
mod config;
mod cost;
mod error;
mod ledger;
mod projection;
mod response;
mod server;
mod tools;
mod types;

pub use cache::ResponseCache;
pub use client::SerperClient;
pub use config::{
    KEYRING_SERVICE, KEYRING_USER, Overrides, Settings, default_config_dir, default_data_dir,
    keychain_delete, keychain_get, keychain_set, mask,
};
pub use cost::{BATCH_MULTIPLIER, Endpoint, Host, usd_for_call, usd_for_credits};
pub use error::{Error, Result};
pub use ledger::{CallRecord, Ledger, LedgerRow, Reservation, ToolTotals, Totals, rotate, verify};
pub use projection::{
    MAX_RESULT_BYTES, ScrapeContent, SearchSections, fit_to_budget, sanitize, search_sections,
    summary_text,
};
pub use response::{
    AnswerBox, BatchEntry, BatchOutput, CostInfo, Envelope, HealthOutput, ImageItem,
    KnowledgeGraph, LensItem, ListOutput, LocationItem, NewsItem, OrganicItem, PatentItem,
    PeopleAlsoAsk, PlaceItem, ReviewItem, ScholarItem, ScrapeOutput, SessionOutput, ShoppingItem,
    Suggestion, ToolUsage, VideoItem,
};
pub use secrecy::SecretString;
pub use server::{Metrics, OwnserpHandler, serve_http, serve_stdio};
pub use tools::specs::{SPECS, ToolSpec};
pub use tools::{ToolContext, args, call as call_tool};
pub use types::{
    ApiKeySource, CreditCost, LocationQuery, MapViewport, PackTier, Preset, TextMode, TimeFilter,
};

pub const APP_NAME: &str = "OwnSERP";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
