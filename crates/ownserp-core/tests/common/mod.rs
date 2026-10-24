#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::atomic::{AtomicU32, Ordering};

use ownserp_core::{Overrides, Settings};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub(crate) fn settings(label: &str) -> Settings {
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ledger_dir = std::env::temp_dir().join(format!(
        "ownserp-test-{}-{label}-{serial}",
        std::process::id()
    ));
    Settings::load(Overrides {
        profile: Some(ledger_dir.join("absent-profile.toml")),
        ledger_dir: Some(ledger_dir),
        cache_ttl_secs: Some(0),
        ..Overrides::default()
    })
    .expect("settings load")
}
