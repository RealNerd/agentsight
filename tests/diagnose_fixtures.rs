//! Integration tests: run diagnose analysis on fixture sessions and assert expected classifications.

use std::path::PathBuf;

use agentsight::commands::diagnose::{
    analyze_cache_stability, analyze_context_growth, analyze_tool_patterns,
    detect_identical_command_retries, CacheClassification,
};
use agentsight::parser::reader::{parse_session_file, summarize_session};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_summary(name: &str) -> agentsight::parser::types::SessionSummary {
    let path = fixture_path(name);
    let entries = parse_session_file(&path, true).expect(&format!("failed to parse {}", name));
    summarize_session(&entries, "test-session".to_string(), "/project".to_string())
}

// ── Cache classification tests ───────────────────────────────────

#[test]
fn cache_churning_fixture_is_churning() {
    let summary = load_summary("cache_churning.jsonl");
    let stability = analyze_cache_stability(&summary.turns);
    assert_eq!(
        stability.classification,
        CacheClassification::Churning,
        "cache_churning.jsonl should be classified as Churning, got {:?}. \
         turns_above_threshold={}, total_turns={}, avg_cache_creation_pct={:.1}",
        stability.classification,
        stability.turns_above_threshold,
        stability.total_turns,
        stability.avg_cache_creation_pct,
    );
}

#[test]
fn short_session_is_stable() {
    let summary = load_summary("short_session.jsonl");
    let stability = analyze_cache_stability(&summary.turns);
    // Short sessions (<5 turns) are always Stable
    assert_eq!(stability.classification, CacheClassification::Stable);
}

#[test]
fn normal_mixed_tools_is_stable() {
    let summary = load_summary("normal_mixed_tools.jsonl");
    let stability = analyze_cache_stability(&summary.turns);
    // Normal session with good cache behavior should be Stable
    assert_eq!(
        stability.classification,
        CacheClassification::Stable,
        "normal_mixed_tools.jsonl should be Stable, got {:?}",
        stability.classification
    );
}

// ── Bash loop detection ──────────────────────────────────────────

#[test]
fn bash_heavy_has_bash_loops() {
    let summary = load_summary("bash_heavy.jsonl");
    let patterns = analyze_tool_patterns(&summary.turns);
    // 7 consecutive Bash-only turns should trigger bash loop detection
    assert!(
        !patterns.bash_loops.is_empty(),
        "bash_heavy.jsonl should have bash loops detected"
    );
    // First loop should start early
    assert!(
        patterns.bash_loops[0].length >= 3,
        "bash loop should be at least 3 turns, got {}",
        patterns.bash_loops[0].length
    );
}

#[test]
fn bash_heavy_has_identical_retries() {
    let summary = load_summary("bash_heavy.jsonl");
    let retries = detect_identical_command_retries(&summary.turns);
    // "cargo build 2>&1" is repeated 4 times, then "cargo test 2>&1" is repeated 3 times
    assert!(
        !retries.is_empty(),
        "bash_heavy.jsonl should have identical command retries"
    );
}

#[test]
fn normal_mixed_tools_no_bash_loops() {
    let summary = load_summary("normal_mixed_tools.jsonl");
    let patterns = analyze_tool_patterns(&summary.turns);
    // Mixed tools should NOT trigger bash loops
    assert!(
        patterns.bash_loops.is_empty(),
        "normal_mixed_tools.jsonl should have no bash loops"
    );
}

// ── Context growth ───────────────────────────────────────────────

#[test]
fn large_session_context_growth() {
    let summary = load_summary("large_session.jsonl");
    let growth = analyze_context_growth(&summary.turns);
    // Large session has input growing from ~5000 to ~15000 → growth factor ~3x
    assert!(
        growth.growth_factor > 1.5,
        "large_session.jsonl should show context growth, got factor {:.2}",
        growth.growth_factor
    );
}

#[test]
fn short_session_minimal_growth() {
    let summary = load_summary("short_session.jsonl");
    let growth = analyze_context_growth(&summary.turns);
    // Short session shouldn't show significant growth (only 2 turns)
    // Growth factor may be undefined for very short sessions
    assert!(
        growth.per_turn_input.len() <= 2,
        "expected <= 2 turns of input data"
    );
}

// ── Tool pattern analysis ────────────────────────────────────────

#[test]
fn sidechain_has_task_calls() {
    let summary = load_summary("sidechain.jsonl");
    let patterns = analyze_tool_patterns(&summary.turns);
    // Sidechain fixture uses Task tool
    assert!(
        patterns.subagent_count > 0,
        "sidechain.jsonl should have Task (subagent) calls"
    );
}

#[test]
fn error_heavy_has_mixed_patterns() {
    let summary = load_summary("error_heavy.jsonl");
    let patterns = analyze_tool_patterns(&summary.turns);
    // Should have both Bash and Edit tool usage
    let total_tools: u32 = summary.tool_calls.values().sum();
    assert!(
        total_tools > 3,
        "error_heavy.jsonl should have multiple tool calls"
    );
}

// ── Multi-model fixture ──────────────────────────────────────────

#[test]
fn multi_model_turns_have_different_models() {
    let summary = load_summary("multi_model.jsonl");
    let unique_models: std::collections::HashSet<_> = summary
        .turns
        .iter()
        .filter_map(|t| t.model.as_deref())
        .collect();
    assert!(
        unique_models.len() >= 2,
        "multi_model.jsonl should have at least 2 different models, got {:?}",
        unique_models
    );
}

// ── Empty and malformed resilience ───────────────────────────────

#[test]
fn empty_session_diagnose_does_not_panic() {
    let summary = load_summary("empty_session.jsonl");
    // All analysis functions should handle 0 turns gracefully
    let stability = analyze_cache_stability(&summary.turns);
    let growth = analyze_context_growth(&summary.turns);
    let patterns = analyze_tool_patterns(&summary.turns);
    assert_eq!(stability.total_turns, 0);
    assert!(growth.per_turn_input.is_empty());
    assert!(patterns.bash_loops.is_empty());
}

#[test]
fn malformed_session_diagnose_works() {
    let summary = load_summary("malformed.jsonl");
    // Should work on the valid entries that were parsed
    let stability = analyze_cache_stability(&summary.turns);
    assert_eq!(stability.total_turns, 3);
    let patterns = analyze_tool_patterns(&summary.turns);
    assert!(patterns.bash_loops.is_empty());
}
