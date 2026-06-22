mod benchmark;
mod cache;
mod claude_md;
mod clear_advisor;
mod cli;
mod context;
mod errors;
mod models;
mod patterns;
mod recommendations;
mod render;
mod trend;
pub mod types;

#[cfg(test)]
pub(crate) mod test_helpers;

use crate::parser::types::{SessionEntry, SessionSummary};

use self::types::DiagnoseData;

// ── Re-exports (preserve all external import paths) ───────────────

// Used by server/handlers.rs
pub use self::benchmark::{compute_project_benchmark, rank_benchmarks};
pub use self::claude_md::analyze_claude_md;
pub use self::trend::analyze_project_trend;
pub use self::types::{CacheClassification, ProjectBenchmark, TrendDirection};

// Used by watch (live) and diagnose (post-hoc) for the /clear advisor
pub use self::clear_advisor::{
    advise_clear, clear_advice_to_json, detect_context_window, ClearAdvice, ClearUrgency,
};

// Used by tests/diagnose_fixtures.rs
pub use self::cache::analyze_cache_stability;
pub use self::context::analyze_context_growth;
pub use self::errors::detect_same_error_retries;
pub use self::models::collect_model_distribution;
pub use self::patterns::{analyze_tool_patterns, detect_identical_command_retries};
pub use self::recommendations::generate_recommendations;

// Used by main.rs
pub use self::cli::run;
pub use self::types::DiagnoseArgs;

// Used by commands/health.rs
pub use self::render::classification_str;

// ── Orchestration ─────────────────────────────────────────────────

/// Run all diagnostic analyses on a session summary.
pub fn run_diagnose(summary: &SessionSummary) -> DiagnoseData {
    run_diagnose_with_entries(summary, None)
}

/// Run all diagnostic analyses, optionally with entry-level same-error detection.
pub fn run_diagnose_with_entries(
    summary: &SessionSummary,
    entries: Option<&[SessionEntry]>,
) -> DiagnoseData {
    let cache_stability = analyze_cache_stability(&summary.turns);
    let context_growth = analyze_context_growth(&summary.turns);
    let tool_patterns = analyze_tool_patterns(&summary.turns);
    let same_error_retries = entries.map(detect_same_error_retries);
    let clear_advice = advise_clear(
        &summary.turns,
        detect_context_window(summary.model.as_deref()),
    );
    let recommendations = generate_recommendations(
        &cache_stability,
        &context_growth,
        &tool_patterns,
        same_error_retries.as_deref(),
    );

    DiagnoseData {
        cache_stability,
        context_growth,
        tool_patterns,
        same_error_retries,
        clear_advice,
        recommendations,
    }
}
