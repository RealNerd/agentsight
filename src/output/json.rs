use chrono::Utc;
use serde::Serialize;

use crate::cost::CostBreakdown;
use crate::parser::types::SessionSummary;
use std::collections::HashMap;

#[derive(Serialize, Clone)]
pub struct SessionListJson {
    pub sessions: Vec<SessionJson>,
    pub total_tokens: u64,
    pub session_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_cost: Option<f64>,
}

#[derive(Serialize, Clone)]
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

#[derive(Serialize, Clone)]
pub struct CostJson {
    pub input: f64,
    pub cache_creation: f64,
    pub cache_read: f64,
    pub output: f64,
    pub total: f64,
}

#[derive(Serialize, Clone)]
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
    println!(
        "{}",
        serde_json::to_string(&snapshot).unwrap_or_default()
    );
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

#[derive(Serialize, Clone)]
pub struct TurnDetailJson {
    pub index: usize,
    pub timestamp: Option<String>,
    pub tokens: TokensJson,
    pub tools: Vec<String>,
    pub model: Option<String>,
}

#[derive(Serialize, Clone)]
pub struct SessionDetailJson {
    #[serde(flatten)]
    pub session: SessionJson,
    pub git_branch: Option<String>,
    pub turn_details: Vec<TurnDetailJson>,
}

#[derive(Serialize, Clone)]
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
}

#[derive(Serialize, Clone)]
pub struct HourBurnJson {
    pub hour: String,
    pub tokens: u64,
}

#[derive(Serialize, Clone)]
pub struct ProjectBreakdownJson {
    pub project: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub sessions: u64,
    pub pct: f64,
}

#[derive(Serialize, Clone)]
pub struct ModelBreakdownJson {
    pub model: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub pct: f64,
}

#[derive(Serialize, Clone)]
pub struct DayBreakdownJson {
    pub date: String,
    pub tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    pub sessions: u64,
}

// ── Timeline types ────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct TimelineJson {
    pub period_start: String,
    pub period_end: String,
    pub period_days: u64,
    pub sessions: Vec<TimelineSessionJson>,
    pub concurrency: Vec<ConcurrencySlotJson>,
    pub peak_concurrent: u64,
    pub total_sessions: usize,
}

#[derive(Serialize, Clone)]
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

#[derive(Serialize, Clone)]
pub struct ConcurrencySlotJson {
    pub time: String,
    pub count: u64,
    pub tokens: u64,
}

#[derive(Serialize, Clone)]
pub struct ConfigJson {
    pub billing_mode: String,
    pub show_cost: bool,
    pub models: Vec<String>,
}
