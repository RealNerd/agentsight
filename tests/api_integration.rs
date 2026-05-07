//! Integration tests for the server/API layer.
//!
//! Uses `tower::ServiceExt::oneshot()` to send requests directly to the Router
//! without binding a TCP socket — the standard axum testing pattern.

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::TempDir;
use tower::ServiceExt;

use agentsight::config::Config;
use agentsight::output::json::{
    ConfigJson, ProjectDiagnoseJson, SessionDetailJson, SessionListJson, SummaryJson, TimelineJson,
};
use agentsight::server::cache::SessionCache;
use agentsight::server::state::AppState;
use agentsight::server::{self};

// ── Test harness ──────────────────────────────────────────────────

struct TestHarness {
    router: axum::Router,
    #[allow(dead_code)]
    tmp: TempDir,
}

impl TestHarness {
    /// Build a harness with the default 3-fixture, 2-project layout.
    async fn default() -> Self {
        Self::with_cost(false).await
    }

    /// Build a harness with configurable cost display.
    async fn with_cost(show_cost: bool) -> Self {
        let tmp = TempDir::new().expect("create temp dir");
        let claude_dir = tmp.path().to_path_buf();

        // Create project directories
        // "project-alpha" decodes via decode_project_path to "project/alpha"
        // "project-beta" decodes to "project/beta"
        let alpha_dir = claude_dir.join("projects").join("project-alpha");
        let beta_dir = claude_dir.join("projects").join("project-beta");
        std::fs::create_dir_all(&alpha_dir).unwrap();
        std::fs::create_dir_all(&beta_dir).unwrap();

        let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures");

        // short_session.jsonl → project-alpha, UUID ...0001
        std::fs::copy(
            fixtures.join("short_session.jsonl"),
            alpha_dir.join("00000000-0000-0000-0000-000000000001.jsonl"),
        )
        .unwrap();

        // multi_model.jsonl → project-alpha, UUID ...0004
        std::fs::copy(
            fixtures.join("multi_model.jsonl"),
            alpha_dir.join("00000000-0000-0000-0000-000000000004.jsonl"),
        )
        .unwrap();

        // normal_mixed_tools.jsonl → project-beta, UUID ...0010
        std::fs::copy(
            fixtures.join("normal_mixed_tools.jsonl"),
            beta_dir.join("00000000-0000-0000-0000-000000000010.jsonl"),
        )
        .unwrap();

        Self::build(tmp, claude_dir, show_cost).await
    }

    /// Build a harness with an empty claude dir (no sessions).
    async fn empty() -> Self {
        let tmp = TempDir::new().expect("create temp dir");
        let claude_dir = tmp.path().to_path_buf();
        std::fs::create_dir_all(claude_dir.join("projects")).unwrap();
        Self::build(tmp, claude_dir, false).await
    }

    async fn build(tmp: TempDir, claude_dir: PathBuf, show_cost: bool) -> Self {
        let config = Arc::new(Config::load(None).expect("load config"));
        let cache = Arc::new(SessionCache::new(claude_dir, config.clone()));
        let (watch_tx, _) = tokio::sync::broadcast::channel(16);

        cache.refresh().await;

        let state = AppState {
            config,
            show_cost,
            watch_tx,
            cache,
        };

        let router = server::build_router(state);
        Self { router, tmp }
    }

    /// Send a GET request and return (status, body bytes).
    async fn get(&self, uri: &str) -> (StatusCode, Vec<u8>) {
        let req = Request::builder().uri(uri).body(Body::empty()).unwrap();
        let resp = self.router.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, body)
    }

    /// GET and deserialize JSON.
    async fn get_json<T: serde::de::DeserializeOwned>(&self, uri: &str) -> T {
        let (status, body) = self.get(uri).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "expected 200 for {}, got {} — body: {}",
            uri,
            status,
            String::from_utf8_lossy(&body)
        );
        serde_json::from_slice(&body).unwrap_or_else(|e| {
            panic!(
                "failed to deserialize response for {}: {} — body: {}",
                uri,
                e,
                String::from_utf8_lossy(&body)
            )
        })
    }
}

// ── GET /health ───────────────────────────────────────────────────

#[tokio::test]
async fn health_returns_ok() {
    let h = TestHarness::default().await;
    let (status, body) = h.get("/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(String::from_utf8_lossy(&body), "ok");
}

// ── GET /config ───────────────────────────────────────────────────

#[tokio::test]
async fn config_returns_billing_and_models() {
    let h = TestHarness::default().await;
    let cfg: ConfigJson = h.get_json("/api/v1/config").await;
    // Default billing mode is "max"
    assert_eq!(cfg.billing_mode, "max");
    assert!(!cfg.show_cost);
    assert!(!cfg.models.is_empty(), "should have at least one model");
}

#[tokio::test]
async fn config_reflects_show_cost() {
    let h = TestHarness::with_cost(true).await;
    let cfg: ConfigJson = h.get_json("/api/v1/config").await;
    assert!(cfg.show_cost);
}

// ── GET /sessions ─────────────────────────────────────────────────

#[tokio::test]
async fn sessions_lists_all_with_large_days() {
    let h = TestHarness::default().await;
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=9999").await;
    assert_eq!(list.session_count, 3, "should find 3 fixture sessions");
    assert_eq!(list.sessions.len(), 3);
    assert!(list.total_tokens > 0);
    // Default show_cost=false → no cost
    assert!(list.total_cost.is_none());
}

#[tokio::test]
async fn sessions_filters_by_project() {
    let h = TestHarness::default().await;
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=9999&project=alpha").await;
    assert_eq!(list.session_count, 2, "project-alpha has 2 sessions");
    for s in &list.sessions {
        assert!(
            s.project.contains("alpha"),
            "session project '{}' should contain 'alpha'",
            s.project
        );
    }
}

#[tokio::test]
async fn sessions_respects_limit() {
    let h = TestHarness::default().await;
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=9999&limit=1").await;
    assert_eq!(list.session_count, 1);
    assert_eq!(list.sessions.len(), 1);
}

#[tokio::test]
async fn sessions_date_filter_excludes_old() {
    let h = TestHarness::default().await;
    // Fixtures are from June 2025 — days=1 should exclude them all
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=1").await;
    assert_eq!(list.session_count, 0, "days=1 should exclude 2025 fixtures");
}

#[tokio::test]
async fn sessions_sort_by_tokens() {
    let h = TestHarness::default().await;
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=9999&sort=tokens").await;
    assert!(list.session_count >= 2);
    // Verify descending token order
    for w in list.sessions.windows(2) {
        assert!(
            w[0].tokens.total >= w[1].tokens.total,
            "sessions not sorted by tokens descending: {} < {}",
            w[0].tokens.total,
            w[1].tokens.total
        );
    }
}

#[tokio::test]
async fn sessions_include_cost_when_enabled() {
    let h = TestHarness::with_cost(true).await;
    let list: SessionListJson = h.get_json("/api/v1/sessions?days=9999").await;
    assert!(list.total_cost.is_some(), "should include total_cost");
    for s in &list.sessions {
        assert!(s.cost.is_some(), "each session should have cost");
    }
}

// ── GET /sessions/{id} ───────────────────────────────────────────

#[tokio::test]
async fn session_by_id_returns_detail() {
    let h = TestHarness::default().await;
    let detail: SessionDetailJson = h
        .get_json("/api/v1/sessions/00000000-0000-0000-0000-000000000001")
        .await;
    assert_eq!(
        detail.session.session_id,
        "00000000-0000-0000-0000-000000000001"
    );
    assert_eq!(detail.session.slug.as_deref(), Some("short-session"));
    assert_eq!(detail.turn_details.len(), 2, "short_session has 2 turns");
}

#[tokio::test]
async fn session_by_id_404_for_unknown() {
    let h = TestHarness::default().await;
    let (status, _) = h
        .get("/api/v1/sessions/ffffffff-ffff-ffff-ffff-ffffffffffff")
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── GET /sessions/by-slug/{slug} ─────────────────────────────────

#[tokio::test]
async fn session_by_slug_exact_match() {
    let h = TestHarness::default().await;
    let detail: SessionDetailJson = h.get_json("/api/v1/sessions/by-slug/short-session").await;
    assert_eq!(detail.session.slug.as_deref(), Some("short-session"));
}

#[tokio::test]
async fn session_by_slug_substring_match() {
    let h = TestHarness::default().await;
    let detail: SessionDetailJson = h.get_json("/api/v1/sessions/by-slug/mixed").await;
    assert_eq!(detail.session.slug.as_deref(), Some("mixed-tools"));
}

#[tokio::test]
async fn session_by_slug_404_for_unknown() {
    let h = TestHarness::default().await;
    let (status, _) = h.get("/api/v1/sessions/by-slug/nonexistent-slug").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ── GET /summary ──────────────────────────────────────────────────

#[tokio::test]
async fn summary_aggregates_all_sessions() {
    let h = TestHarness::default().await;
    let summary: SummaryJson = h.get_json("/api/v1/summary?days=9999").await;
    assert_eq!(summary.session_count, 3);
    assert!(summary.total_tokens > 0);
    assert!(summary.total_cost.is_none(), "default show_cost=false");
}

#[tokio::test]
async fn summary_filters_by_project() {
    let h = TestHarness::default().await;
    let summary: SummaryJson = h.get_json("/api/v1/summary?days=9999&project=beta").await;
    assert_eq!(summary.session_count, 1, "project-beta has 1 session");
}

#[tokio::test]
async fn summary_empty_when_no_match() {
    let h = TestHarness::default().await;
    let summary: SummaryJson = h
        .get_json("/api/v1/summary?days=9999&project=nonexistent")
        .await;
    assert_eq!(summary.session_count, 0);
    assert_eq!(summary.total_tokens, 0);
}

// ── GET /projects ─────────────────────────────────────────────────

#[tokio::test]
async fn projects_lists_discovered_sorted() {
    let h = TestHarness::default().await;
    let projects: Vec<String> = h.get_json("/api/v1/projects").await;
    assert_eq!(projects.len(), 2);
    // Should be sorted alphabetically
    assert!(
        projects[0] <= projects[1],
        "projects not sorted: {:?}",
        projects
    );
    // Both shortened project names should be present
    assert!(projects.iter().any(|p| p.contains("alpha")));
    assert!(projects.iter().any(|p| p.contains("beta")));
}

// ── GET /timeline ─────────────────────────────────────────────────

#[tokio::test]
async fn timeline_returns_sessions_and_concurrency() {
    let h = TestHarness::default().await;
    let tl: TimelineJson = h.get_json("/api/v1/timeline?days=9999").await;
    assert_eq!(tl.total_sessions, 3);
    assert!(!tl.sessions.is_empty());
    // Concurrency slots should exist when sessions overlap or span time
    // (at minimum we get slots covering the session range)
    assert!(!tl.concurrency.is_empty());
}

#[tokio::test]
async fn timeline_empty_when_no_match() {
    let h = TestHarness::default().await;
    let tl: TimelineJson = h
        .get_json("/api/v1/timeline?days=9999&project=nonexistent")
        .await;
    assert_eq!(tl.total_sessions, 0);
    assert!(tl.sessions.is_empty());
    assert!(tl.concurrency.is_empty());
}

#[tokio::test]
async fn timeline_filters_by_project() {
    let h = TestHarness::default().await;
    let tl: TimelineJson = h.get_json("/api/v1/timeline?days=9999&project=alpha").await;
    assert_eq!(tl.total_sessions, 2);
    for s in &tl.sessions {
        assert!(
            s.project.contains("alpha"),
            "timeline session '{}' should contain 'alpha'",
            s.project
        );
    }
}

// ── GET /diagnose ─────────────────────────────────────────────────

#[tokio::test]
async fn diagnose_returns_benchmarks() {
    let h = TestHarness::default().await;
    let diag: ProjectDiagnoseJson = h.get_json("/api/v1/diagnose?days=9999").await;
    assert!(
        diag.project_count >= 2,
        "should benchmark at least 2 projects"
    );
    assert!(!diag.benchmarks.is_empty());
    // No project filter → no trend
    assert!(diag.trend.is_none());
}

#[tokio::test]
async fn diagnose_project_filter_includes_trend() {
    let h = TestHarness::default().await;
    let diag: ProjectDiagnoseJson = h.get_json("/api/v1/diagnose?days=9999&project=alpha").await;
    assert_eq!(diag.project_count, 1);
    // Project filter should produce a trend
    assert!(diag.trend.is_some(), "project filter should include trend");
}

#[tokio::test]
async fn diagnose_empty_when_no_match() {
    let h = TestHarness::default().await;
    let diag: ProjectDiagnoseJson = h
        .get_json("/api/v1/diagnose?days=9999&project=nonexistent")
        .await;
    assert_eq!(diag.project_count, 0);
    assert!(diag.benchmarks.is_empty());
}

// ── Edge case: empty claude dir ──────────────────────────────────

#[tokio::test]
async fn empty_claude_dir_returns_empty_results() {
    let h = TestHarness::empty().await;

    let sessions: SessionListJson = h.get_json("/api/v1/sessions?days=9999").await;
    assert_eq!(sessions.session_count, 0);

    let projects: Vec<String> = h.get_json("/api/v1/projects").await;
    assert!(projects.is_empty());

    let summary: SummaryJson = h.get_json("/api/v1/summary?days=9999").await;
    assert_eq!(summary.session_count, 0);

    let tl: TimelineJson = h.get_json("/api/v1/timeline?days=9999").await;
    assert_eq!(tl.total_sessions, 0);

    let diag: ProjectDiagnoseJson = h.get_json("/api/v1/diagnose?days=9999").await;
    assert_eq!(diag.project_count, 0);
}
