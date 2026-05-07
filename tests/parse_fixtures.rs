//! Integration tests: every fixture file parses without panic, and no sensitive content leaks.

use std::path::PathBuf;

use agentsight::parser::reader::{parse_session_file, summarize_session};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

/// Parse a fixture and return its summary.
fn parse_fixture(
    name: &str,
) -> (
    Vec<agentsight::parser::types::SessionEntry>,
    agentsight::parser::types::SessionSummary,
) {
    let path = fixture_path(name);
    let entries = parse_session_file(&path, true)
        .unwrap_or_else(|e| panic!("failed to parse {}: {}", name, e));
    let summary = summarize_session(&entries, "test-session".to_string(), "/project".to_string());
    (entries, summary)
}

// ── Every fixture parses without panic ───────────────────────────

#[test]
fn parse_short_session() {
    let (entries, summary) = parse_fixture("short_session.jsonl");
    // 5 lines: system, user, assistant, user, assistant → 2 assistant turns
    assert_eq!(summary.turns.len(), 2);
    assert!(entries.len() >= 4);
    assert_eq!(summary.slug.as_deref(), Some("short-session"));
}

#[test]
fn parse_empty_session() {
    let (entries, summary) = parse_fixture("empty_session.jsonl");
    // No assistant entries → 0 turns
    assert_eq!(summary.turns.len(), 0);
    assert!(entries.len() >= 2); // file-history-snapshot + system
}

#[test]
fn parse_malformed() {
    let (entries, summary) = parse_fixture("malformed.jsonl");
    // 14 lines total, 5 are bad JSON/empty → 9 valid JSON lines
    // system + 3 user + 3 assistant + 1 progress + 1 "null" (Unknown) = 8-9 entries
    // (null parses as valid JSON but won't match any SessionEntry variant tag → Unknown)
    assert!(
        entries.len() >= 7,
        "expected at least 7 parsed entries from valid JSON lines, got {}",
        entries.len()
    );
    assert_eq!(summary.turns.len(), 3);
    assert_eq!(summary.slug.as_deref(), Some("malformed-test"));
}

#[test]
fn parse_multi_model() {
    let (_, summary) = parse_fixture("multi_model.jsonl");
    // 5 assistant turns with different models
    assert_eq!(summary.turns.len(), 5);
    // First model should be opus
    assert_eq!(summary.model.as_deref(), Some("claude-opus-4-6"));
    // Individual turns have different models
    let models: Vec<Option<&str>> = summary.turns.iter().map(|t| t.model.as_deref()).collect();
    assert!(models.contains(&Some("claude-sonnet-4-20250514")));
    assert!(models.contains(&Some("claude-haiku-3-5")));
}

#[test]
fn parse_normal_mixed_tools() {
    let (_, summary) = parse_fixture("normal_mixed_tools.jsonl");
    // 11 assistant entries → 11 turns
    assert_eq!(summary.turns.len(), 11);
    assert_eq!(summary.slug.as_deref(), Some("mixed-tools"));
    // Should have diverse tool usage
    assert!(summary.tool_calls.contains_key("Read"));
    assert!(summary.tool_calls.contains_key("Edit"));
    assert!(summary.tool_calls.contains_key("Bash"));
    assert!(summary.tool_calls.contains_key("Grep"));
    assert!(summary.tool_calls.contains_key("Glob"));
    assert!(summary.tool_calls.contains_key("Write"));
}

#[test]
fn parse_bash_heavy() {
    let (_, summary) = parse_fixture("bash_heavy.jsonl");
    assert_eq!(summary.turns.len(), 8);
    assert_eq!(summary.slug.as_deref(), Some("bash-heavy"));
    // Should be mostly Bash calls
    let bash_count = summary.tool_calls.get("Bash").copied().unwrap_or(0);
    assert!(
        bash_count >= 7,
        "expected at least 7 Bash calls, got {}",
        bash_count
    );
}

#[test]
fn parse_error_heavy() {
    let (_, summary) = parse_fixture("error_heavy.jsonl");
    assert_eq!(summary.turns.len(), 8);
    assert_eq!(summary.slug.as_deref(), Some("error-heavy"));
    // Should have both Bash and Edit calls
    assert!(summary.tool_calls.contains_key("Bash"));
    assert!(summary.tool_calls.contains_key("Edit"));
    assert!(summary.tool_calls.contains_key("Read"));
    // Bash is the dominant tool
    let bash_count = summary.tool_calls.get("Bash").copied().unwrap_or(0);
    assert!(
        bash_count >= 4,
        "expected at least 4 Bash calls in error_heavy, got {}",
        bash_count
    );
}

#[test]
fn parse_cache_churning() {
    let (_, summary) = parse_fixture("cache_churning.jsonl");
    assert_eq!(summary.turns.len(), 10);
    assert_eq!(summary.slug.as_deref(), Some("cache-churn"));
    // Every turn should have substantial cache_creation_input_tokens
    for (i, turn) in summary.turns.iter().enumerate() {
        assert!(
            turn.usage.cache_creation_input_tokens > 1000,
            "turn {} should have high cache creation, got {}",
            i,
            turn.usage.cache_creation_input_tokens
        );
    }
}

#[test]
fn parse_sidechain() {
    let (_, summary) = parse_fixture("sidechain.jsonl");
    // Has both main and sidechain assistant entries
    assert!(
        summary.turns.len() >= 5,
        "expected at least 5 turns, got {}",
        summary.turns.len()
    );
    assert_eq!(summary.slug.as_deref(), Some("sidechain-test"));
    // Should have Task tool calls
    assert!(summary.tool_calls.contains_key("Task"));
}

#[test]
fn parse_large_session() {
    let (_, summary) = parse_fixture("large_session.jsonl");
    // 99 assistant turns expected
    assert_eq!(summary.turns.len(), 99);
    assert_eq!(summary.slug.as_deref(), Some("large-session"));
    // Should have diverse tools
    assert!(summary.tool_calls.contains_key("Bash"));
    assert!(summary.tool_calls.contains_key("Read"));
    assert!(summary.tool_calls.contains_key("Edit"));
}

// ── No sensitive content leaks ───────────────────────────────────

#[test]
fn no_real_home_dirs_in_fixtures() {
    let fixture_names = [
        "short_session.jsonl",
        "empty_session.jsonl",
        "malformed.jsonl",
        "multi_model.jsonl",
        "normal_mixed_tools.jsonl",
        "bash_heavy.jsonl",
        "error_heavy.jsonl",
        "cache_churning.jsonl",
        "sidechain.jsonl",
        "large_session.jsonl",
    ];

    for name in &fixture_names {
        let path = fixture_path(name);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {}", name, e));

        // Scan for any /Users/<name>/ or /home/<name>/ pattern (real home dirs)
        for line in content.lines() {
            for prefix in &["/Users/", "/home/"] {
                if let Some(start) = line.find(prefix) {
                    let after = &line[start + prefix.len()..];
                    // Extract the username component (up to / or end)
                    let username: String = after.chars().take_while(|c| *c != '/').collect();
                    // "user" is our sanitized placeholder — anything else is a leak
                    assert!(
                        username.is_empty() || username == "user",
                        "fixture {} contains real home dir: {}{}",
                        name,
                        prefix,
                        username
                    );
                }
            }
        }

        // Should not contain known real usernames
        assert!(
            !content.contains("sandvault"),
            "fixture {} contains real username 'sandvault'",
            name
        );
    }
}

// ── Token usage is preserved ─────────────────────────────────────

#[test]
fn token_usage_nonzero() {
    let fixtures_with_turns = [
        "short_session.jsonl",
        "normal_mixed_tools.jsonl",
        "bash_heavy.jsonl",
        "error_heavy.jsonl",
        "cache_churning.jsonl",
        "sidechain.jsonl",
        "large_session.jsonl",
    ];

    for name in &fixtures_with_turns {
        let (_, summary) = parse_fixture(name);
        assert!(
            summary.total_usage.total_tokens() > 0,
            "fixture {} has zero total tokens",
            name
        );
        assert!(
            summary.total_usage.output_tokens > 0,
            "fixture {} has zero output tokens",
            name
        );
    }
}
