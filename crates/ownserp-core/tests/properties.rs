#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use ownserp_core::{CallRecord, Ledger, sanitize, verify};
use proptest::prelude::*;

fn record(seq: u32, tool: &str) -> CallRecord {
    CallRecord {
        tool: tool.to_owned(),
        endpoint: format!("/{tool}"),
        args_hash: format!("{seq:064x}"),
        credits_estimated: seq,
        credits_actual: None,
        batch_multiplier: 1,
        duration_ms: u64::from(seq),
        status: "ok".to_owned(),
        response_bytes: u64::from(seq) * 10,
        tokens: u64::from(seq) * 2,
        tokens_saved: u64::from(seq),
        cache_hit: seq.is_multiple_of(2),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn sanitize_strips_invisible_characters(input in ".*") {
        let cleaned = sanitize(&input);
        let has_invisible = cleaned.chars().any(|character| matches!(character,
            '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}' | '\u{FEFF}' | '\u{FE00}'..='\u{FE0F}'
            | '\u{E0000}'..='\u{E007F}'));
        prop_assert!(!has_invisible, "sanitized text kept an invisible character");
        prop_assert!(
            !cleaned.chars().any(|character| character.is_control()),
            "sanitized text kept a control character"
        );
        prop_assert!(cleaned.chars().count() <= input.chars().count());
    }

    #[test]
    fn sanitize_is_stable_under_a_second_pass(input in ".*") {
        let once = sanitize(&input);
        prop_assert_eq!(sanitize(&once), once);
    }
}

#[test]
fn ledger_chain_accepts_appends_and_rejects_a_tampered_line() {
    let path = std::env::temp_dir().join(format!(
        "ownserp-property-{}-ledger.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    {
        let ledger = Ledger::open(&path).expect("open ledger");
        for seq in 1..=5 {
            ledger.append(record(seq, "serper_search")).expect("append");
        }
        let totals = ledger.totals().expect("totals");
        assert_eq!(totals.calls, 5);
        assert_eq!(totals.credits, 15);
        assert_eq!(totals.cache_hits, 2);
    }
    assert_eq!(verify(&path).expect("chain verifies"), 5);

    let original = std::fs::read_to_string(&path).expect("read ledger");
    let tampered = original.replacen("\"credits_estimated\":3", "\"credits_estimated\":9", 1);
    assert_ne!(
        original, tampered,
        "the fixture line must contain the field"
    );
    std::fs::write(&path, tampered).expect("write tampered ledger");
    assert!(verify(&path).is_err(), "a changed row must break the chain");

    std::fs::write(&path, original).expect("restore ledger");
    assert_eq!(verify(&path).expect("chain verifies again"), 5);
    std::fs::remove_file(&path).expect("remove test ledger");
}

#[test]
fn rotation_archives_a_verified_ledger_and_starts_a_fresh_chain() {
    let dir = std::env::temp_dir().join(format!("ownserp-rotate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("calls.jsonl");
    let _ = std::fs::remove_file(&path);
    {
        let ledger = Ledger::open(&path).expect("open ledger");
        ledger.append(record(1, "serper_search")).expect("append");
        ledger.append(record(2, "serper_news")).expect("append");
    }
    let (rows, archive) = ownserp_core::rotate(&path).expect("rotate");
    assert_eq!(rows, 2);
    assert!(archive.exists(), "archive must exist");
    assert!(!path.exists(), "live ledger must be gone");
    assert_eq!(verify(&archive).expect("archive verifies"), 2);

    let fresh = Ledger::open(&path).expect("reopen");
    let row = fresh.append(record(1, "serper_locations")).expect("append");
    assert_eq!(row.seq, 1);
    assert_eq!(row.prev, "genesis");
    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn rotation_refuses_when_there_is_no_ledger() {
    let path = std::env::temp_dir().join(format!(
        "ownserp-rotate-missing-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    assert!(ownserp_core::rotate(&path).is_err());
}

#[test]
fn sanitize_strips_every_added_format_range() {
    let samples = [
        '\u{1BCA0}',
        '\u{1BCA3}',
        '\u{13430}',
        '\u{1343F}',
        '\u{0600}',
        '\u{0605}',
        '\u{06DD}',
        '\u{070F}',
        '\u{0890}',
        '\u{0891}',
        '\u{08E2}',
        '\u{110BD}',
        '\u{110CD}',
        '\u{FFF0}',
        '\u{FFF8}',
        '\u{E0080}',
        '\u{E00FF}',
        '\u{E01F0}',
        '\u{E0FFF}',
    ];
    for sample in samples {
        let text = format!("ign{sample}ore");
        let cleaned = sanitize(&text);
        assert!(
            !cleaned.contains(sample),
            "U+{:04X} survived sanitizing: {cleaned:?}",
            sample as u32
        );
    }
}
