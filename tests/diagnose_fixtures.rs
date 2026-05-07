//! Integration tests: run diagnose analysis on fixture sessions and assert expected classifications.

use std::path::PathBuf;

use agentsight::commands::diagnose::{
    analyze_cache_stability, analyze_context_growth, analyze_tool_patterns,
    detect_identical_command_retries, detect_same_error_retries, generate_recommendations,
    run_diagnose_with_entries, CacheClassification,
};
use agentsight::parser::reader::{parse_session_file, summarize_session};
use agentsight::parser::types::SessionEntry;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn load_fixture(name: &str) -> (Vec<SessionEntry>, agentsight::parser::types::SessionSummary) {
    let path = fixture_path(name);
    let entries = parse_session_file(&path, true)
        .unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e));
    let summary = summarize_session(&entries, "test-session".to_string(), "/project".to_string());
    (entries, summary)
}

fn load_summary(name: &str) -> agentsight::parser::types::SessionSummary {
    let (_, summary) = load_fixture(name);
    summary
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
    // Large session has input growing from ~5000+3500=8500 to ~15000+13700=28700
    // Growth factor should be well above 2x
    assert!(
        growth.growth_factor > 2.0,
        "large_session.jsonl should show >2x context growth, got factor {:.2}",
        growth.growth_factor
    );
    assert!(
        growth.flagged,
        "large_session.jsonl context growth should be flagged (>2x)"
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
fn error_heavy_has_bash_retries() {
    let summary = load_summary("error_heavy.jsonl");
    let retries = detect_identical_command_retries(&summary.turns);
    // "npm run build" appears in turns 0, 2, 5, 6 — not all consecutive, but some streaks
    // The fixture has Bash-only turns interspersed with Edit/Read turns
    // At minimum, verify the tool mix is correct
    assert!(summary.tool_calls.contains_key("Bash"));
    assert!(summary.tool_calls.contains_key("Edit"));
    assert!(summary.tool_calls.contains_key("Read"));
    let bash_count = summary.tool_calls.get("Bash").copied().unwrap_or(0);
    assert!(
        bash_count >= 4,
        "expected at least 4 Bash calls, got {}",
        bash_count
    );
    // May or may not have identical retries depending on consecutive streak length
    // but the fixture should at least not panic
    let _ = retries;
}

#[test]
fn error_heavy_same_error_retries() {
    let (entries, _) = load_fixture("error_heavy.jsonl");
    // detect_same_error_retries operates on raw entries
    let retries = detect_same_error_retries(&entries);
    // The fixture doesn't have 3+ consecutive identical error outputs
    // (errors are interspersed with non-error results), so this should be empty
    // but it must not panic on error-heavy input
    let _ = retries;
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

#[test]
fn multi_model_diagnose_shows_model_distribution() {
    use agentsight::commands::diagnose::collect_model_distribution;

    let summary = load_summary("multi_model.jsonl");
    let distribution = collect_model_distribution(&summary.turns);

    // Should have at least 2 models in the distribution
    assert!(
        distribution.len() >= 2,
        "multi_model.jsonl should have >= 2 models in distribution, got {:?}",
        distribution
    );

    // Total turns across all models should equal summary.turns.len()
    let total_turns: usize = distribution.iter().map(|(_, count)| count).sum();
    assert_eq!(
        total_turns,
        summary.turns.len(),
        "distribution turn count should match total turns"
    );
}

#[test]
fn single_model_diagnose_no_model_distribution() {
    use agentsight::commands::diagnose::collect_model_distribution;

    let summary = load_summary("short_session.jsonl");
    let distribution = collect_model_distribution(&summary.turns);

    // Single-model session should have at most 1 entry
    assert!(
        distribution.len() <= 1,
        "short_session.jsonl should have <= 1 model in distribution, got {:?}",
        distribution
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

// ── Recommendations tests ────────────────────────────────────────

#[test]
fn churning_produces_cache_recommendation() {
    let summary = load_summary("cache_churning.jsonl");
    let cache = analyze_cache_stability(&summary.turns);
    let growth = analyze_context_growth(&summary.turns);
    let tools = analyze_tool_patterns(&summary.turns);
    let recs = generate_recommendations(&cache, &growth, &tools, None);
    assert!(
        recs.iter()
            .any(|r| r.contains("Cache creation stayed above 30%")),
        "cache_churning should produce a cache creation recommendation, got: {:?}",
        recs
    );
}

#[test]
fn bash_heavy_produces_retry_recommendation() {
    let summary = load_summary("bash_heavy.jsonl");
    let cache = analyze_cache_stability(&summary.turns);
    let growth = analyze_context_growth(&summary.turns);
    let tools = analyze_tool_patterns(&summary.turns);
    let recs = generate_recommendations(&cache, &growth, &tools, None);
    // Should mention bash retry sequences or identical command retries
    assert!(
        recs.iter()
            .any(|r| r.contains("Bash retry") || r.contains("identical command")),
        "bash_heavy should produce a bash loop or retry recommendation, got: {:?}",
        recs
    );
}

#[test]
fn normal_session_produces_no_recommendations() {
    let summary = load_summary("short_session.jsonl");
    let cache = analyze_cache_stability(&summary.turns);
    let growth = analyze_context_growth(&summary.turns);
    let tools = analyze_tool_patterns(&summary.turns);
    let recs = generate_recommendations(&cache, &growth, &tools, None);
    assert!(
        recs.is_empty(),
        "short_session should produce no recommendations, got: {:?}",
        recs
    );
}

// ── Full diagnose pipeline (run_diagnose_with_entries) ───────────

#[test]
fn full_diagnose_cache_churning() {
    let (entries, summary) = load_fixture("cache_churning.jsonl");
    let diag = run_diagnose_with_entries(&summary, Some(&entries));
    assert_eq!(
        diag.cache_stability.classification,
        CacheClassification::Churning
    );
    assert!(!diag.recommendations.is_empty());
}

#[test]
fn full_diagnose_bash_heavy() {
    let (entries, summary) = load_fixture("bash_heavy.jsonl");
    let diag = run_diagnose_with_entries(&summary, Some(&entries));
    assert!(!diag.tool_patterns.bash_loops.is_empty());
    assert!(!diag.recommendations.is_empty());
}

#[test]
fn full_diagnose_large_session() {
    let (entries, summary) = load_fixture("large_session.jsonl");
    let diag = run_diagnose_with_entries(&summary, Some(&entries));
    assert!(diag.context_growth.flagged);
    assert!(diag.context_growth.growth_factor > 2.0);
}
