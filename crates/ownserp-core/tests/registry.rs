#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use std::collections::BTreeSet;

use ownserp_core::{Overrides, OwnserpHandler, Settings};
use serde_json::Value;

#[test]
fn catalog_tools_have_prefixed_unique_names_and_complete_schemas() {
    let handler = OwnserpHandler::new(common::settings("catalog")).expect("handler");
    let tools = handler.tools();
    assert_eq!(tools.len(), 17, "batch stays disabled by default");
    let mut names = BTreeSet::new();
    let mut descriptions = BTreeSet::new();
    for tool in tools {
        assert!(
            tool.name.starts_with("serper_"),
            "{} lacks the prefix",
            tool.name
        );
        assert!(
            tool.name.len() <= 64,
            "{} is longer than 64 characters",
            tool.name
        );
        assert!(
            names.insert(tool.name.to_string()),
            "{} is duplicated",
            tool.name
        );
        let description = tool.description.clone().unwrap_or_default();
        assert!(!description.is_empty(), "{} has no description", tool.name);
        assert!(
            description.len() <= 2048,
            "{} description is {} bytes",
            tool.name,
            description.len()
        );
        assert!(
            descriptions.insert(description.clone()),
            "{} repeats another description",
            tool.name
        );
        assert!(
            tool.output_schema.is_some(),
            "{} has no output schema",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&Value::String("object".to_owned())),
            "{} input schema is not object rooted",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "{} accepts unknown fields",
            tool.name
        );
        let annotations = tool.annotations.as_ref().expect("annotations");
        let free = matches!(
            tool.name.as_ref(),
            "serper_locations" | "serper_session" | "serper_health" | "serper_resolve"
        );
        assert_eq!(
            annotations.read_only_hint,
            Some(free),
            "{} spends credits, so it must not report read-only",
            tool.name
        );
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(
            annotations.idempotent_hint,
            Some(free),
            "{} must report idempotent only when a repeat costs nothing",
            tool.name
        );
        assert_eq!(annotations.open_world_hint, Some(true));
    }
    assert!(names.contains("serper_search"));
    assert!(names.contains("serper_session"));
    assert!(names.contains("serper_locations"));
}

#[test]
fn the_server_info_declares_tools_and_both_protocol_versions() {
    use rmcp::ServerHandler;

    let handler = OwnserpHandler::new(common::settings("info")).expect("handler");
    let info = handler.get_info();
    assert!(info.capabilities.tools.is_some());
    assert!(info.instructions.is_some());
    assert_eq!(info.server_info.name, "OwnSERP");
    let versions = handler.supported_protocol_versions();
    assert_eq!(versions.len(), 2);
    assert!(
        versions
            .iter()
            .any(|version| version.to_string() == "2026-07-28")
    );
    assert!(
        versions
            .iter()
            .any(|version| version.to_string() == "2025-11-25")
    );
}

#[test]
fn catalog_fits_the_measured_budget() {
    let handler = OwnserpHandler::new(common::settings("budget")).expect("handler");
    let bytes = serde_json::to_vec(handler.tools()).expect("serialize catalog");
    assert!(
        bytes.len() < 48_000,
        "default tools list is {} bytes, above the 48 KB budget",
        bytes.len()
    );
}

#[test]
fn the_batch_tool_appears_only_when_enabled() {
    let base = common::settings("gate");
    let handler = OwnserpHandler::new(base).expect("handler");
    assert!(
        !handler
            .tools()
            .iter()
            .any(|tool| tool.name == "serper_search_batch")
    );

    let dir =
        std::env::temp_dir().join(format!("ownserp-test-batch-enabled-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let profile = dir.join("profile.toml");
    std::fs::write(&profile, "").expect("profile");
    let overrides = Overrides {
        enable_batch: Some(true),
        ledger_dir: Some(dir),
        profile: Some(profile),
        cache_ttl_secs: Some(0),
        ..Overrides::default()
    };
    let enabled = Settings::load(overrides).expect("settings");
    let handler = OwnserpHandler::new(enabled).expect("handler");
    assert!(
        handler
            .tools()
            .iter()
            .any(|tool| tool.name == "serper_search_batch")
    );
}
