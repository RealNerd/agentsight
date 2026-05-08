//! CLI binary integration tests using assert_cmd.
//!
//! Tests the actual `agentsight` binary against fixture data to verify
//! main.rs dispatch logic, argument parsing, and end-to-end output.

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

// ── Test harness ──────────────────────────────────────────────────

fn agentsight() -> Command {
    Command::cargo_bin("agentsight").expect("binary must exist")
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

/// Set up a temp claude dir with multiple fixtures across two projects.
/// Returns (TempDir guard, claude_dir path string).
fn setup_fixtures() -> (TempDir, String) {
    let tmp = TempDir::new().expect("create temp dir");
    let claude_dir = tmp.path().to_path_buf();

    let alpha_dir = claude_dir.join("projects").join("project-alpha");
    let beta_dir = claude_dir.join("projects").join("project-beta");
    std::fs::create_dir_all(&alpha_dir).unwrap();
    std::fs::create_dir_all(&beta_dir).unwrap();

    let fixtures = fixtures_dir();

    // short_session.jsonl -> project-alpha
    std::fs::copy(
        fixtures.join("short_session.jsonl"),
        alpha_dir.join("00000000-0000-0000-0000-000000000001.jsonl"),
    )
    .unwrap();

    // multi_model.jsonl -> project-alpha
    std::fs::copy(
        fixtures.join("multi_model.jsonl"),
        alpha_dir.join("00000000-0000-0000-0000-000000000004.jsonl"),
    )
    .unwrap();

    // normal_mixed_tools.jsonl -> project-beta
    std::fs::copy(
        fixtures.join("normal_mixed_tools.jsonl"),
        beta_dir.join("00000000-0000-0000-0000-000000000010.jsonl"),
    )
    .unwrap();

    // bash_heavy.jsonl -> project-beta
    std::fs::copy(
        fixtures.join("bash_heavy.jsonl"),
        beta_dir.join("00000000-0000-0000-0000-000000000011.jsonl"),
    )
    .unwrap();

    // cache_churning.jsonl -> project-alpha
    std::fs::copy(
        fixtures.join("cache_churning.jsonl"),
        alpha_dir.join("00000000-0000-0000-0000-000000000005.jsonl"),
    )
    .unwrap();

    let dir_str = claude_dir.to_string_lossy().to_string();
    (tmp, dir_str)
}

/// Set up a temp claude dir with an empty projects directory.
fn setup_empty() -> (TempDir, String) {
    let tmp = TempDir::new().expect("create temp dir");
    let claude_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
    let dir_str = claude_dir.to_string_lossy().to_string();
    (tmp, dir_str)
}

/// Set up a temp claude dir with only a malformed fixture.
fn setup_malformed() -> (TempDir, String) {
    let tmp = TempDir::new().expect("create temp dir");
    let claude_dir = tmp.path().to_path_buf();
    let project_dir = claude_dir.join("projects").join("project-bad");
    std::fs::create_dir_all(&project_dir).unwrap();

    std::fs::copy(
        fixtures_dir().join("malformed.jsonl"),
        project_dir.join("00000000-0000-0000-0000-000000000099.jsonl"),
    )
    .unwrap();

    let dir_str = claude_dir.to_string_lossy().to_string();
    (tmp, dir_str)
}

// ── sessions ──────────────────────────────────────────────────────

#[test]
fn sessions_table_output() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args(["--claude-dir", &dir, "sessions", "--days", "9999"])
        .assert()
        .success();
}

#[test]
fn sessions_json_valid() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args(["--claude-dir", &dir, "--json", "sessions", "--days", "9999"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("session_count").is_some());
    assert!(json.get("sessions").is_some());
    assert!(json["sessions"].is_array());
}

#[test]
fn sessions_filter_by_project() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args([
            "--claude-dir",
            &dir,
            "--json",
            "sessions",
            "--days",
            "9999",
            "--project",
            "alpha",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let sessions = json["sessions"].as_array().expect("sessions array");
    // All sessions should be from project-alpha
    for s in sessions {
        let project = s["project"].as_str().unwrap_or("");
        assert!(
            project.contains("alpha"),
            "expected alpha project, got: {}",
            project
        );
    }
}

#[test]
fn sessions_limit() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args([
            "--claude-dir",
            &dir,
            "--json",
            "sessions",
            "--days",
            "9999",
            "--limit",
            "1",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let sessions = json["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 1);
}

#[test]
fn sessions_empty_dir() {
    let (_tmp, dir) = setup_empty();
    agentsight()
        .args(["--claude-dir", &dir, "sessions", "--days", "9999"])
        .assert()
        .success();
}

// ── session ───────────────────────────────────────────────────────

#[test]
fn session_by_uuid() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args([
            "--claude-dir",
            &dir,
            "session",
            "00000000-0000-0000-0000-000000000001",
        ])
        .assert()
        .success();
}

#[test]
fn session_json_valid() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args([
            "--claude-dir",
            &dir,
            "--json",
            "session",
            "00000000-0000-0000-0000-000000000001",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("session_id").is_some());
    assert!(json.get("tokens").is_some());
}

#[test]
fn session_not_found() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args([
            "--claude-dir",
            &dir,
            "session",
            "nonexistent-session-id-that-does-not-exist",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("No session")));
}

// ── summary ───────────────────────────────────────────────────────

#[test]
fn summary_json_valid() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args(["--claude-dir", &dir, "--json", "summary", "--days", "9999"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("period_days").is_some());
    assert!(json.get("total_tokens").is_some());
}

#[test]
fn summary_text_output() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args(["--claude-dir", &dir, "summary", "--days", "9999"])
        .assert()
        .success();
}

// ── timeline ──────────────────────────────────────────────────────

#[test]
fn timeline_json_valid() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args(["--claude-dir", &dir, "--json", "timeline", "--days", "9999"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("sessions").is_some());
    assert!(json["sessions"].is_array());
}

#[test]
fn timeline_empty() {
    let (_tmp, dir) = setup_empty();
    agentsight()
        .args(["--claude-dir", &dir, "--json", "timeline", "--days", "9999"])
        .assert()
        .success();
}

// ── diagnose ──────────────────────────────────────────────────────

#[test]
fn diagnose_session_json() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args([
            "--claude-dir",
            &dir,
            "--json",
            "diagnose",
            "00000000-0000-0000-0000-000000000001",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("cache_stability").is_some());
    assert!(json.get("recommendations").is_some());
}

#[test]
fn diagnose_project_level_json() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args(["--claude-dir", &dir, "--json", "diagnose", "--days", "9999"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("benchmarks").is_some());
    assert!(json.get("global_avg_cache_hit").is_some());
}

#[test]
fn diagnose_session_text() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args([
            "--claude-dir",
            &dir,
            "diagnose",
            "00000000-0000-0000-0000-000000000001",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cache Stability"));
}

#[test]
fn diagnose_project_text() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args(["--claude-dir", &dir, "diagnose", "--days", "9999"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Project Diagnostics"));
}

// ── health ────────────────────────────────────────────────────────

#[test]
fn health_quick() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args(["--claude-dir", &dir, "health", "--quick"])
        .assert()
        .success();
}

#[test]
fn health_json() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args(["--claude-dir", &dir, "--json", "health", "--quick"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.get("environment").is_some());
}

// ── install-skill ─────────────────────────────────────────────────

#[test]
fn install_skill_list() {
    agentsight()
        .args(["install-skill", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("diagnose"));
}

#[test]
fn install_skill_list_json() {
    let output = agentsight()
        .args(["--json", "install-skill", "--list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    assert!(json.is_array() || json.get("skills").is_some());
}

// ── sanitize ──────────────────────────────────────────────────────

#[test]
fn sanitize_basic() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args([
            "--claude-dir",
            &dir,
            "sanitize",
            "00000000-0000-0000-0000-000000000001",
        ])
        .assert()
        .success();
}

#[test]
fn sanitize_max_lines() {
    let (_tmp, dir) = setup_fixtures();
    agentsight()
        .args([
            "--claude-dir",
            &dir,
            "sanitize",
            "00000000-0000-0000-0000-000000000001",
            "--max-lines",
            "3",
        ])
        .assert()
        .success();
}

// ── Cross-cutting ─────────────────────────────────────────────────

#[test]
fn cost_flag_adds_cost() {
    let (_tmp, dir) = setup_fixtures();
    let output = agentsight()
        .args([
            "--claude-dir",
            &dir,
            "--json",
            "--cost",
            "sessions",
            "--days",
            "9999",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).expect("valid JSON");
    let sessions = json["sessions"].as_array().expect("sessions array");
    // With --cost, session entries should have cost fields
    if let Some(first) = sessions.first() {
        assert!(
            first.get("cost").is_some(),
            "expected cost field in session JSON when --cost is used"
        );
    }
}

#[test]
fn verbose_malformed_warnings() {
    let (_tmp, dir) = setup_malformed();
    agentsight()
        .args(["--claude-dir", &dir, "-v", "sessions", "--days", "9999"])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("malformed").or(predicate::str::contains("skip")
                .or(predicate::str::contains("warn").or(predicate::str::is_empty().not()))),
        );
}

#[test]
fn no_subcommand_shows_help() {
    agentsight()
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage"));
}

#[test]
fn version_flag() {
    agentsight()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("agentsight"))
        .stdout(predicate::str::is_match(r"\d+\.\d+\.\d+ \(").unwrap());
}

// ── completions ──────────────────────────────────────────────────

#[test]
fn completions_bash() {
    agentsight()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agentsight"));
}

#[test]
fn completions_zsh() {
    agentsight()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agentsight"));
}

#[test]
fn completions_fish() {
    agentsight()
        .args(["completions", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("agentsight"));
}

#[test]
fn completions_hidden_from_help() {
    agentsight()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("completions").not());
}

#[test]
fn completions_invalid_shell() {
    agentsight()
        .args(["completions", "nushell"])
        .assert()
        .failure();
}

// ── dashboard ────────────────────────────────────────────────────

#[test]
fn dashboard_replace_flag_exists() {
    agentsight()
        .args(["dashboard", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--replace"));
}

// ── help examples ────────────────────────────────────────────────

#[test]
fn sessions_help_shows_examples() {
    agentsight()
        .args(["sessions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"));
}

#[test]
fn session_help_shows_examples() {
    agentsight()
        .args(["session", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"));
}

#[test]
fn diagnose_help_shows_examples() {
    agentsight()
        .args(["diagnose", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"))
        .stdout(predicate::str::contains("--with-context"));
}

#[test]
fn health_help_shows_examples() {
    agentsight()
        .args(["health", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Examples:"));
}
