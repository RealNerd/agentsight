//! Integration tests for per-turn model statistics and model comparison features.

use std::collections::HashMap;
use std::path::PathBuf;

use agentsight::aggregation::{merge_by_family, ModelStats};
use agentsight::output::table::normalize_model_family;
use agentsight::parser::reader::{parse_session_file, summarize_session};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

// ── Per-turn model stats from fixture ────────────────────────────

#[test]
fn multi_model_per_turn_stats() {
    let path = fixture_path("multi_model.jsonl");
    let entries = parse_session_file(&path, true).expect("parse multi_model.jsonl");
    let summary = summarize_session(&entries, "test-session".to_string(), "/project".to_string());

    // Accumulate per-model stats from turns (mirrors summary.rs accumulation)
    let mut model_map: HashMap<String, ModelStats> = HashMap::new();

    for turn in &summary.turns {
        let model = turn.model.as_deref().unwrap_or("unknown").to_string();
        let ms = model_map
            .entry(model.clone())
            .or_insert_with(|| ModelStats {
                model,
                ..Default::default()
            });

        let turn_input = turn.usage.input_tokens
            + turn.usage.cache_creation_input_tokens
            + turn.usage.cache_read_input_tokens;

        ms.turns += 1;
        ms.total_input += turn_input;
        ms.total_output += turn.usage.output_tokens;
        ms.cache_read += turn.usage.cache_read_input_tokens;
        ms.cache_creation += turn.usage.cache_creation_input_tokens;
        ms.total_tokens += turn.usage.total_tokens();
    }

    // multi_model.jsonl has:
    // Turn 0: claude-opus-4-6 (input=4000, output=300, cache_creation=3000, cache_read=0)
    // Turn 1: claude-sonnet-4-20250514 (input=3000, output=200, cache_creation=500, cache_read=2500)
    // Turn 2: claude-haiku-3-5 (input=3500, output=100, cache_creation=200, cache_read=3000)
    // Turn 3: claude-opus-4-6 (input=4500, output=250, cache_creation=100, cache_read=4000)
    // Turn 4: claude-sonnet-4-20250514 (input=5000, output=180, cache_creation=300, cache_read=4200)

    assert_eq!(model_map.len(), 3, "should have 3 distinct models");

    // Opus: 2 turns
    let opus = model_map.get("claude-opus-4-6").expect("opus should exist");
    assert_eq!(opus.turns, 2);
    assert_eq!(opus.total_output, 300 + 250);
    assert_eq!(opus.cache_read, 4000);
    assert_eq!(opus.cache_creation, 3000 + 100);

    // Sonnet: 2 turns
    let sonnet = model_map
        .get("claude-sonnet-4-20250514")
        .expect("sonnet should exist");
    assert_eq!(sonnet.turns, 2);
    assert_eq!(sonnet.cache_read, 2500 + 4200);

    // Haiku: 1 turn
    let haiku = model_map
        .get("claude-haiku-3-5")
        .expect("haiku should exist");
    assert_eq!(haiku.turns, 1);
    assert_eq!(haiku.cache_read, 3000);
}

#[test]
fn multi_model_cache_hit_differs_between_models() {
    let path = fixture_path("multi_model.jsonl");
    let entries = parse_session_file(&path, true).expect("parse multi_model.jsonl");
    let summary = summarize_session(&entries, "test-session".to_string(), "/project".to_string());

    let mut model_map: HashMap<String, ModelStats> = HashMap::new();
    for turn in &summary.turns {
        let model = turn.model.as_deref().unwrap_or("unknown").to_string();
        let ms = model_map
            .entry(model.clone())
            .or_insert_with(|| ModelStats {
                model,
                ..Default::default()
            });

        let turn_input = turn.usage.input_tokens
            + turn.usage.cache_creation_input_tokens
            + turn.usage.cache_read_input_tokens;

        ms.turns += 1;
        ms.total_input += turn_input;
        ms.cache_read += turn.usage.cache_read_input_tokens;
        ms.cache_creation += turn.usage.cache_creation_input_tokens;
        ms.total_tokens += turn.usage.total_tokens();
    }

    let opus = model_map.get("claude-opus-4-6").unwrap();
    let sonnet = model_map.get("claude-sonnet-4-20250514").unwrap();
    let haiku = model_map.get("claude-haiku-3-5").unwrap();

    // Opus has high cache_creation (first turn), lower cache hit
    // Sonnet has high cache reads, higher cache hit
    assert!(
        opus.cache_hit_ratio() < sonnet.cache_hit_ratio(),
        "opus cache_hit ({:.2}) should be lower than sonnet ({:.2}) due to initial cache creation",
        opus.cache_hit_ratio(),
        sonnet.cache_hit_ratio()
    );

    // Haiku also has high cache reads
    assert!(
        haiku.cache_hit_ratio() > opus.cache_hit_ratio(),
        "haiku cache_hit ({:.2}) should be higher than opus ({:.2})",
        haiku.cache_hit_ratio(),
        opus.cache_hit_ratio()
    );
}

#[test]
fn group_family_merges_correctly() {
    let stats = vec![
        ModelStats {
            model: "claude-sonnet-4-20250514".to_string(),
            turns: 10,
            sessions: 2,
            total_tokens: 100_000,
            total_input: 80_000,
            total_output: 20_000,
            cache_read: 60_000,
            cache_creation: 5_000,
            cost: 1.0,
            ..Default::default()
        },
        ModelStats {
            model: "claude-sonnet-4-20250601".to_string(),
            turns: 5,
            sessions: 1,
            total_tokens: 50_000,
            total_input: 40_000,
            total_output: 10_000,
            cache_read: 30_000,
            cache_creation: 2_000,
            cost: 0.5,
            ..Default::default()
        },
    ];

    let merged = merge_by_family(&stats);
    assert_eq!(merged.len(), 1, "two sonnet versions should merge into one");

    let sonnet = &merged[0];
    assert_eq!(sonnet.model, "claude-sonnet-4");
    assert_eq!(sonnet.turns, 15);
    assert_eq!(sonnet.sessions, 3);
    assert_eq!(sonnet.total_tokens, 150_000);
    assert_eq!(sonnet.cache_read, 90_000);
    assert!((sonnet.cost - 1.5).abs() < 0.001);
}

// ── Normalization tests ─────────────────────────────────────────

#[test]
fn normalize_family_various_models() {
    assert_eq!(
        normalize_model_family("claude-opus-4-6-20250514"),
        "claude-opus-4-6"
    );
    assert_eq!(
        normalize_model_family("claude-sonnet-4-20250514"),
        "claude-sonnet-4"
    );
    assert_eq!(
        normalize_model_family("claude-haiku-3-5-20250514"),
        "claude-haiku-3-5"
    );
    // No date suffix → unchanged
    assert_eq!(normalize_model_family("claude-opus-4-6"), "claude-opus-4-6");
    // Short model names
    assert_eq!(normalize_model_family("gpt-4"), "gpt-4");
}

// ── Single-model session: no merge effect ───────────────────────

#[test]
fn single_model_fixture_produces_one_entry() {
    let path = fixture_path("short_session.jsonl");
    let entries = parse_session_file(&path, true).expect("parse short_session.jsonl");
    let summary = summarize_session(&entries, "test-session".to_string(), "/project".to_string());

    let unique_models: std::collections::HashSet<_> = summary
        .turns
        .iter()
        .filter_map(|t| t.model.as_deref())
        .collect();

    // short_session.jsonl should have only one model
    assert!(
        unique_models.len() <= 1,
        "short_session should have at most 1 model, got {:?}",
        unique_models
    );
}
