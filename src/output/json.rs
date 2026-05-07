use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cost::CostBreakdown;
use crate::parser::types::SessionSummary;
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionListJson {
    pub sessions: Vec<SessionJson>,
    pub total_tokens: u64,
    pub session_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionJson {
    pub session_id: String,
    pub slug: Option<String>,
    pub project: String,
    pub model: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub turns: usize,
    pub tokens: TokensJson,
    pub cache_hit_ratio: f64,
    pub tool_calls: HashMap<String, u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<CostJson>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CostJson {
    pub input: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
    pub output: f64,
    pub total: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TokensJson {
    pub input: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub output: u64,
    pub total: u64,
}

impl From<&CostBreakdown> for CostJson {
    fn from(c: &CostBreakdown) -> Self {
        CostJson {
            input: c.input_cost,
            cache_creation: c.cache_creation_cost,
            cache_read: c.cache_read_cost,
            output: c.output_cost,
            total: c.total(),
        }
    }
}

pub fn session_to_json(
    summary: &SessionSummary,
    cost: &CostBreakdown,
    cache_hit: f64,
    show_cost: bool,
) -> SessionJson {
    SessionJson {
        session_id: summary.session_id.clone(),
        slug: summary.slug.clone(),
        project: summary.project_path.clone(),
        model: summary.model.clone(),
        start_time: summary.start_time.map(|t| t.to_rfc3339()),
        end_time: summary.end_time.map(|t| t.to_rfc3339()),
        turns: summary.turns.len(),
        tokens: TokensJson {
            input: summary.total_usage.input_tokens,
            cache_creation: summary.total_usage.cache_creation_input_tokens,
            cache_read: summary.total_usage.cache_read_input_tokens,
            output: summary.total_usage.output_tokens,
            total: summary.total_usage.total_tokens(),
        },
        cache_hit_ratio: cache_hit,
        tool_calls: summary.tool_calls.clone(),
        cost: if show_cost {
            Some(CostJson::from(cost))
        } else {
            None
        },
    }
}

/// Print a single session as JSON to stdout.
pub fn print_session_json(
    summary: &SessionSummary,
    cost: &CostBreakdown,
    cache_hit: f64,
    show_cost: bool,
) {
    let json = session_to_json(summary, cost, cache_hit, show_cost);
    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

// ── Watch snapshot types ──────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct WatchSnapshotJson {
    pub timestamp: String,
    pub active_sessions: Vec<SessionJson>,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
}

/// Print a watch snapshot as a single NDJSON line to stdout.
pub fn print_watch_snapshot_json(
    sessions: &[(SessionSummary, CostBreakdown, f64)],
    show_cost: bool,
) {
    let active: Vec<SessionJson> = sessions
        .iter()
        .map(|(s, c, hit)| session_to_json(s, c, *hit, show_cost))
        .collect();

    let total_tokens: u64 = sessions
        .iter()
        .map(|(s, _, _)| s.total_usage.total_tokens())
        .sum();

    let total_cost = if show_cost {
        Some(sessions.iter().map(|(_, c, _)| c.total()).sum())
    } else {
        None
    };

    let snapshot = WatchSnapshotJson {
        timestamp: Utc::now().to_rfc3339(),
        active_sessions: active,
        total_tokens,
        total_cost,
    };

    // NDJSON: one compact JSON object per line
    println!("{}", serde_json::to_string(&snapshot).unwrap_or_default());
}

/// Print a list of sessions as JSON to stdout.
pub fn print_sessions_json(items: &[(SessionSummary, CostBreakdown, f64)], show_cost: bool) {
    let sessions: Vec<SessionJson> = items
        .iter()
        .map(|(s, c, hit)| session_to_json(s, c, *hit, show_cost))
        .collect();

    let total_tokens: u64 = items
        .iter()
        .map(|(s, _, _)| s.total_usage.total_tokens())
        .sum();

    let total_cost = if show_cost {
        Some(items.iter().map(|(_, c, _)| c.total()).sum())
    } else {
        None
    };

    let list = SessionListJson {
        session_count: sessions.len(),
        total_tokens,
        total_cost,
        sessions,
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&list).unwrap_or_default()
    );
}

// ── Dashboard API types ───────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct TurnDetailJson {
    pub index: usize,
    pub timestamp: Option<String>,
    pub tokens: TokensJson,
    pub tools: Vec<String>,
    pub model: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SessionDetailJson {
    #[serde(flatten)]
    pub session: SessionJson,
    pub git_branch: Option<String>,
    pub turn_details: Vec<TurnDetailJson>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SummaryJson {
    pub period_days: u64,
    pub session_count: u64,
    pub total_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
    pub avg_tokens_per_session: u64,
    pub cache_hit_ratio: f64,
    pub active_hours: u64,
    pub avg_tokens_per_hour: u64,
    pub peak_hour: Option<HourBurnJson>,
    pub by_project: Vec<ProjectBreakdownJson>,
    pub by_model: Vec<ModelBreakdownJson>,
    pub by_day: Vec<DayBreakdownJson>,
    pub by_hour: Vec<HourBurnJson>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct HourBurnJson {
    pub hour: String,
    pub tokens: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectBreakdownJson {
    pub project: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub sessions: u64,
    pub pct: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ModelBreakdownJson {
    pub model: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sessions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_input_per_turn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_output_per_turn: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_hit_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bash_loops_per_100t: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exploration_ratio: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent_count: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DayBreakdownJson {
    pub date: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub sessions: u64,
}

// ── Timeline types ────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct TimelineJson {
    pub period_start: String,
    pub period_end: String,
    pub period_days: u64,
    pub sessions: Vec<TimelineSessionJson>,
    pub concurrency: Vec<ConcurrencySlotJson>,
    pub peak_concurrent: u64,
    pub total_sessions: usize,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TimelineSessionJson {
    pub session_id: String,
    pub slug: Option<String>,
    pub project: String,
    pub model: Option<String>,
    pub start_time: String,
    pub end_time: String,
    pub duration_minutes: i64,
    pub tokens: u64,
    pub turns: usize,
    pub cache_hit_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConcurrencySlotJson {
    pub time: String,
    pub count: u64,
    pub tokens: u64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ConfigJson {
    pub billing_mode: String,
    pub show_cost: bool,
    pub models: Vec<String>,
}

// ── Diagnose types ────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ModelDistributionJson {
    pub model: String,
    pub turns: usize,
    pub pct: f64,
}

#[derive(Serialize, Clone)]
pub struct DiagnoseJson {
    pub session: SessionJson,
    pub cache_stability: CacheStabilityJson,
    pub context_growth: ContextGrowthJson,
    pub tool_patterns: ToolPatternsJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub same_error_retries: Option<Vec<BashRetryJson>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_distribution: Option<Vec<ModelDistributionJson>>,
    pub recommendations: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct CacheStabilityJson {
    pub classification: String,
    pub turns_above_threshold: usize,
    pub total_turns: usize,
    pub avg_cache_creation_pct: f64,
    pub per_turn_ratios: Vec<f64>,
}

#[derive(Serialize, Clone)]
pub struct ContextGrowthJson {
    pub growth_factor: f64,
    pub flagged: bool,
    pub per_turn_input: Vec<u64>,
}

#[derive(Serialize, Clone)]
pub struct ToolPatternsJson {
    pub bash_loops: Vec<BashLoopJson>,
    pub bash_retries: Vec<BashRetryJson>,
    pub read_edit_ratio: f64,
    pub exploration_flagged: bool,
    pub subagent_count: usize,
    pub subagent_flagged: bool,
}

#[derive(Serialize, Clone)]
pub struct BashLoopJson {
    pub start_turn: usize,
    pub length: usize,
}

#[derive(Serialize, Clone)]
pub struct BashRetryJson {
    pub pattern: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_snippet: Option<String>,
    pub start_turn: usize,
    pub length: usize,
}

// ── Project-level Diagnose types ──────────────────────────────────

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectDiagnoseJson {
    pub period_days: u64,
    pub project_count: usize,
    pub global_avg_cache_hit: f64,
    pub global_avg_tokens: u64,
    pub benchmarks: Vec<ProjectBenchmarkJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trend: Option<ProjectTrendJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude_md: Option<ClaudeMdJson>,
    pub recommendations: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectBenchmarkJson {
    pub project: String,
    pub session_count: usize,
    pub avg_tokens_per_session: u64,
    pub avg_cache_hit: f64,
    pub dominant_classification: String,
    pub bash_loop_count: usize,
    pub bash_retry_count: usize,
    pub exploration_count: usize,
    pub efficiency_score: f64,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectTrendJson {
    pub direction: String,
    pub recent_avg_cache_hit: f64,
    pub overall_avg_cache_hit: f64,
    pub points: Vec<TrendPointJson>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TrendPointJson {
    pub session_id: String,
    pub slug: Option<String>,
    pub date: Option<String>,
    pub tokens: u64,
    pub cache_hit: f64,
    pub classification: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct ClaudeMdJson {
    pub exists: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub size_bytes: u64,
    pub estimated_tokens: u64,
    pub oversized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub recommendations: Vec<String>,
}

// ── Health types ──────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct HealthJson {
    pub environment: EnvironmentCheckJson,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub baseline: Option<BaselineReportJson>,
    pub next_steps: Vec<NextStepJson>,
}

#[derive(Serialize, Clone)]
pub struct EnvironmentCheckJson {
    pub grade: String,
    pub items: Vec<CheckItemJson>,
}

#[derive(Serialize, Clone)]
pub struct CheckItemJson {
    pub name: String,
    pub status: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recommendation: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct BaselineReportJson {
    pub session_count: u64,
    pub total_tokens: u64,
    pub project_count: usize,
    pub global_avg_cache_hit: f64,
    pub benchmarks: Vec<ProjectBenchmarkJson>,
    pub top_recommendations: Vec<String>,
}

#[derive(Serialize, Clone)]
pub struct NextStepJson {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}
