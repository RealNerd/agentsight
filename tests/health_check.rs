//! Integration tests: health command environment check with temp dir mocks.

use std::fs;
use tempfile::TempDir;

use agentsight::commands::health::{
    compute_grade, run_environment_check, CheckStatus, OverallGrade,
};

/// Create a temp dir that mimics ~/.claude/ with projects and sessions.
fn setup_populated_claude_dir() -> TempDir {
    let tmp = TempDir::new().unwrap();

    // Create projects dir with a fake project and session
    let project_dir = tmp
        .path()
        .join("projects")
        .join("-Users-test-myproject");
    fs::create_dir_all(&project_dir).unwrap();

    // Write a minimal valid JSONL session file
    let session_path = project_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl");
    fs::write(
        &session_path,
        r#"{"type":"assistant","message":{"model":"claude-opus-4-6","content":[],"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":0,"cache_read_input_tokens":0}},"timestamp":"2025-05-01T00:00:00Z"}
"#,
    )
    .unwrap();

    // Create settings.json
    fs::write(tmp.path().join("settings.json"), "{}").unwrap();

    // Create CLAUDE.md
    fs::write(tmp.path().join("CLAUDE.md"), "# Global rules\n").unwrap();

    tmp
}

#[test]
fn empty_dir_needs_work() {
    let tmp = TempDir::new().unwrap();
    // Point at a nonexistent subdir
    let fake_claude_dir = tmp.path().join("nonexistent");

    let check = run_environment_check(&fake_claude_dir);
    assert_eq!(check.grade, OverallGrade::NeedsWork);

    // claude dir should be missing
    let claude_item = check
        .items
        .iter()
        .find(|i| i.name.contains("~/.claude/ directory"))
        .unwrap();
    assert_eq!(claude_item.status, CheckStatus::Missing);
}

#[test]
fn populated_dir_good_or_fair() {
    let tmp = setup_populated_claude_dir();

    let check = run_environment_check(tmp.path());

    // Core items should pass
    let claude_item = check
        .items
        .iter()
        .find(|i| i.name.contains("~/.claude/ directory"))
        .unwrap();
    assert_eq!(claude_item.status, CheckStatus::Pass);

    let projects_item = check
        .items
        .iter()
        .find(|i| i.name.contains("~/.claude/projects/"))
        .unwrap();
    assert_eq!(projects_item.status, CheckStatus::Pass);
    assert!(projects_item.detail.contains("sessions found"));

    let global_md = check
        .items
        .iter()
        .find(|i| i.name == "~/.claude/CLAUDE.md")
        .unwrap();
    assert_eq!(global_md.status, CheckStatus::Pass);

    // Grade should be Good or Fair (Fair if cwd has no CLAUDE.md)
    assert!(
        check.grade == OverallGrade::Good || check.grade == OverallGrade::Fair,
        "expected Good or Fair, got {:?}",
        check.grade
    );
}

#[test]
fn claude_dir_exists_but_no_projects() {
    let tmp = TempDir::new().unwrap();
    // Create the claude dir but not the projects subdir
    fs::create_dir_all(tmp.path()).unwrap();

    let check = run_environment_check(tmp.path());

    let projects_item = check
        .items
        .iter()
        .find(|i| i.name.contains("~/.claude/projects/"))
        .unwrap();
    assert_eq!(projects_item.status, CheckStatus::Missing);
    assert_eq!(check.grade, OverallGrade::NeedsWork);
}

#[test]
fn projects_empty_is_warn() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("projects")).unwrap();

    let check = run_environment_check(tmp.path());

    let projects_item = check
        .items
        .iter()
        .find(|i| i.name.contains("~/.claude/projects/"))
        .unwrap();
    assert_eq!(projects_item.status, CheckStatus::Warn);
    // Grade should be Fair (warn present)
    assert_eq!(check.grade, OverallGrade::Fair);
}

#[test]
fn oversized_claude_md_warns() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("projects")).unwrap();

    // Create an oversized CLAUDE.md (> 32KB = ~8K tokens)
    let big_content = "x".repeat(40_000);
    fs::write(tmp.path().join("CLAUDE.md"), &big_content).unwrap();

    let check = run_environment_check(tmp.path());

    let md_item = check
        .items
        .iter()
        .find(|i| i.name == "~/.claude/CLAUDE.md")
        .unwrap();
    assert_eq!(md_item.status, CheckStatus::Warn);
    assert!(md_item.detail.contains("oversized"));
}

#[test]
fn compute_grade_all_pass() {
    use agentsight::commands::health::CheckItem;

    let items = vec![
        CheckItem {
            name: "~/.claude/ directory".to_string(),
            status: CheckStatus::Pass,
            detail: "".to_string(),
            recommendation: None,
        },
        CheckItem {
            name: "~/.claude/projects/".to_string(),
            status: CheckStatus::Pass,
            detail: "".to_string(),
            recommendation: None,
        },
    ];
    assert_eq!(compute_grade(&items), OverallGrade::Good);
}

#[test]
fn compute_grade_missing_core() {
    use agentsight::commands::health::CheckItem;

    let items = vec![
        CheckItem {
            name: "~/.claude/ directory".to_string(),
            status: CheckStatus::Missing,
            detail: "".to_string(),
            recommendation: None,
        },
    ];
    assert_eq!(compute_grade(&items), OverallGrade::NeedsWork);
}
