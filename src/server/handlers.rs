use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;

use crate::commands::timeline::{compute_concurrency, TimelineSession};
use crate::cost::calculator::cache_hit_ratio;
use crate::output::json::{
    session_to_json, ConcurrencySlotJson, ConfigJson, DayBreakdownJson, ModelBreakdownJson,
    ProjectBreakdownJson, SessionDetailJson, SessionJson, SessionListJson, SummaryJson,
    TimelineJson, TimelineSessionJson, TokensJson, TurnDetailJson,
};
use crate::output::table::shorten_project;

use super::cache::CachedSession;
use super::state::AppState;

#[derive(Deserialize)]
pub struct SessionsQuery {
    pub days: Option<u64>,
    pub project: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<usize>,
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionsQuery>,
) -> Result<Json<SessionListJson>, StatusCode> {
    state.cache.refresh().await;

    let days = query.days.unwrap_or(7);
    let limit = query.limit.unwrap_or(50);
    let sort = query.sort.as_deref().unwrap_or("date");
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    let mut items: Vec<&CachedSession> = all
        .iter()
        .map(|arc| arc.as_ref())
        .filter(|cs| {
            // Date filter
            cs.summary
                .start_time
                .is_some_and(|start| start >= cutoff)
        })
        .filter(|cs| {
            // Project filter
            match &query.project {
                Some(filter) => cs.project_path.contains(filter.as_str()),
                None => true,
            }
        })
        .collect();

    // Sort
    match sort {
        "tokens" => items.sort_by_key(|cs| std::cmp::Reverse(cs.summary.total_usage.total_tokens())),
        "turns" => items.sort_by_key(|cs| std::cmp::Reverse(cs.summary.turns.len())),
        "cost" => items.sort_by(|a, b| {
            b.cost
                .total()
                .partial_cmp(&a.cost.total())
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "project" => items.sort_by(|a, b| a.project_path.cmp(&b.project_path)),
        _ => items.sort_by(|a, b| b.summary.start_time.cmp(&a.summary.start_time)),
    }

    items.truncate(limit);

    let sessions: Vec<SessionJson> = items
        .iter()
        .map(|cs| session_to_json(&cs.summary, &cs.cost, cs.cache_hit, state.show_cost))
        .collect();

    let total_tokens: u64 = items
        .iter()
        .map(|cs| cs.summary.total_usage.total_tokens())
        .sum();

    let total_cost = if state.show_cost {
        Some(items.iter().map(|cs| cs.cost.total()).sum())
    } else {
        None
    };

    Ok(Json(SessionListJson {
        session_count: sessions.len(),
        total_tokens,
        total_cost,
        sessions,
    }))
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionDetailJson>, StatusCode> {
    state.cache.refresh().await;

    let cs = state
        .cache
        .get_by_id(&id)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(build_session_detail(&cs, state.show_cost)))
}

pub async fn get_session_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<SessionDetailJson>, StatusCode> {
    state.cache.refresh().await;

    // Try exact match first, then substring. When multiple match, pick most recent.
    let cs = state
        .cache
        .get_by_slug_best(&slug)
        .await
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(build_session_detail(&cs, state.show_cost)))
}

fn build_session_detail(cs: &CachedSession, show_cost: bool) -> SessionDetailJson {
    let turn_details: Vec<TurnDetailJson> = cs
        .summary
        .turns
        .iter()
        .map(|t| TurnDetailJson {
            index: t.index,
            timestamp: t.timestamp.map(|ts| ts.to_rfc3339()),
            tokens: TokensJson {
                input: t.usage.input_tokens,
                cache_creation: t.usage.cache_creation_input_tokens,
                cache_read: t.usage.cache_read_input_tokens,
                output: t.usage.output_tokens,
                total: t.usage.total_tokens(),
            },
            tools: t.tools.clone(),
            model: t.model.clone(),
        })
        .collect();

    let session_json = session_to_json(&cs.summary, &cs.cost, cs.cache_hit, show_cost);

    SessionDetailJson {
        git_branch: cs.summary.git_branch.clone(),
        turn_details,
        session: session_json,
    }
}

#[derive(Deserialize)]
pub struct SummaryQuery {
    pub days: Option<u64>,
    pub project: Option<String>,
}

pub async fn get_summary(
    State(state): State<AppState>,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<SummaryJson>, StatusCode> {
    state.cache.refresh().await;

    let days = query.days.unwrap_or(7);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    // Filter to matching sessions
    let matching: Vec<&CachedSession> = all
        .iter()
        .map(|arc| arc.as_ref())
        .filter(|cs| {
            cs.summary
                .start_time
                .is_some_and(|start| start >= cutoff)
        })
        .filter(|cs| match &query.project {
            Some(filter) => cs.project_path.contains(filter.as_str()),
            None => true,
        })
        .collect();

    // Aggregate
    let mut total_tokens: u64 = 0;
    let mut total_cost_val: f64 = 0.0;
    let mut session_count: u64 = 0;
    let mut tokens_by_project: HashMap<String, u64> = HashMap::new();
    let mut cost_by_project: HashMap<String, f64> = HashMap::new();
    let mut sessions_by_project: HashMap<String, u64> = HashMap::new();
    let mut tokens_by_model: HashMap<String, u64> = HashMap::new();
    let mut cost_by_model: HashMap<String, f64> = HashMap::new();
    let mut total_cache_reads: u64 = 0;
    let mut total_input_tokens: u64 = 0;
    let mut by_day: HashMap<String, (u64, f64, u64)> = HashMap::new(); // (tokens, cost, sessions)

    for cs in &matching {
        let s = &cs.summary;
        let tok = s.total_usage.total_tokens();
        let cost_total = cs.cost.total();

        let short = shorten_project(&cs.project_path);
        *tokens_by_project.entry(short.clone()).or_default() += tok;
        *cost_by_project.entry(short.clone()).or_default() += cost_total;
        *sessions_by_project.entry(short).or_default() += 1;

        let model = s.model.as_deref().unwrap_or("unknown").to_string();
        *tokens_by_model.entry(model.clone()).or_default() += tok;
        *cost_by_model.entry(model).or_default() += cost_total;

        total_cache_reads += s.total_usage.cache_read_input_tokens;
        total_input_tokens += s.total_usage.input_tokens
            + s.total_usage.cache_creation_input_tokens
            + s.total_usage.cache_read_input_tokens;

        if let Some(start) = s.start_time {
            let key = start.format("%Y-%m-%d").to_string();
            let entry = by_day.entry(key).or_default();
            entry.0 += tok;
            entry.1 += cost_total;
            entry.2 += 1;
        }

        total_tokens += tok;
        total_cost_val += cost_total;
        session_count += 1;
    }

    let cache_hit_ratio = if total_input_tokens > 0 {
        total_cache_reads as f64 / total_input_tokens as f64
    } else {
        0.0
    };

    let avg_tokens = total_tokens.checked_div(session_count).unwrap_or(0);

    let show_cost = state.show_cost;

    let mut by_project: Vec<ProjectBreakdownJson> = tokens_by_project
        .iter()
        .map(|(project, tokens)| {
            let pct = if total_tokens > 0 {
                *tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            };
            ProjectBreakdownJson {
                project: project.clone(),
                tokens: *tokens,
                cost: if show_cost {
                    Some(cost_by_project.get(project).copied().unwrap_or(0.0))
                } else {
                    None
                },
                sessions: sessions_by_project.get(project).copied().unwrap_or(0),
                pct,
            }
        })
        .collect();
    by_project.sort_by_key(|p| std::cmp::Reverse(p.tokens));

    let mut by_model: Vec<ModelBreakdownJson> = tokens_by_model
        .iter()
        .map(|(model, tokens)| {
            let pct = if total_tokens > 0 {
                *tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            };
            ModelBreakdownJson {
                model: model.clone(),
                tokens: *tokens,
                cost: if show_cost {
                    Some(cost_by_model.get(model).copied().unwrap_or(0.0))
                } else {
                    None
                },
                pct,
            }
        })
        .collect();
    by_model.sort_by_key(|m| std::cmp::Reverse(m.tokens));

    let mut by_day_json: Vec<DayBreakdownJson> = by_day
        .into_iter()
        .map(|(date, (tokens, cost, sessions))| DayBreakdownJson {
            date,
            tokens,
            cost: if show_cost { Some(cost) } else { None },
            sessions,
        })
        .collect();
    by_day_json.sort_by_key(|d| d.date.clone());

    Ok(Json(SummaryJson {
        period_days: days,
        session_count,
        total_tokens,
        total_cost: if show_cost {
            Some(total_cost_val)
        } else {
            None
        },
        avg_tokens_per_session: avg_tokens,
        cache_hit_ratio,
        by_project,
        by_model,
        by_day: by_day_json,
    }))
}

pub async fn get_config(State(state): State<AppState>) -> Json<ConfigJson> {
    let models: Vec<String> = state.config.models.keys().cloned().collect();

    Json(ConfigJson {
        billing_mode: format!("{:?}", state.config.billing_mode()).to_lowercase(),
        show_cost: state.show_cost,
        models,
    })
}

pub async fn list_projects(
    State(state): State<AppState>,
) -> Result<Json<Vec<String>>, StatusCode> {
    state.cache.refresh().await;
    Ok(Json(state.cache.get_projects().await))
}

#[derive(Deserialize)]
pub struct TimelineQuery {
    pub days: Option<u64>,
    pub project: Option<String>,
}

pub async fn get_timeline(
    State(state): State<AppState>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<TimelineJson>, StatusCode> {
    state.cache.refresh().await;

    let days = query.days.unwrap_or(1);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    let mut sessions: Vec<TimelineSession> = Vec::new();

    for cs in &all {
        // Date filter
        let (start, end) = match (cs.summary.start_time, cs.summary.end_time) {
            (Some(s), Some(e)) if s >= cutoff => (s, e),
            _ => continue,
        };

        // Project filter
        if let Some(ref filter) = query.project {
            if !cs.project_path.contains(filter.as_str()) {
                continue;
            }
        }

        let hit = cache_hit_ratio(&cs.summary.total_usage);

        let turn_activity: Vec<(chrono::DateTime<chrono::Utc>, u64)> = cs
            .summary
            .turns
            .iter()
            .filter_map(|t| t.timestamp.map(|ts| (ts, t.usage.total_tokens())))
            .collect();

        sessions.push(TimelineSession {
            session_id: cs.session_id.clone(),
            slug: cs.summary.slug.clone(),
            project: shorten_project(&cs.project_path),
            model: cs.summary.model.clone(),
            start,
            end,
            duration_minutes: (end - start).num_minutes(),
            tokens: cs.summary.total_usage.total_tokens(),
            turns: cs.summary.turns.len(),
            cache_hit: hit,
            cost: if state.show_cost {
                Some(cs.cost.total())
            } else {
                None
            },
            turn_activity,
        });
    }

    sessions.sort_by_key(|s| s.start);

    if sessions.is_empty() {
        let now = chrono::Utc::now();
        return Ok(Json(TimelineJson {
            period_start: now.to_rfc3339(),
            period_end: now.to_rfc3339(),
            period_days: days,
            sessions: Vec::new(),
            concurrency: Vec::new(),
            peak_concurrent: 0,
            total_sessions: 0,
        }));
    }

    let axis_start = sessions.iter().map(|s| s.start).min().unwrap();
    let axis_end = sessions.iter().map(|s| s.end).max().unwrap();

    let granularity = match days {
        0..=1 => chrono::Duration::minutes(30),
        2..=3 => chrono::Duration::hours(1),
        4..=14 => chrono::Duration::hours(4),
        _ => chrono::Duration::days(1),
    };

    let concurrency = compute_concurrency(&sessions, axis_start, axis_end, granularity);
    let peak = concurrency.iter().map(|s| s.count).max().unwrap_or(0);

    let timeline_sessions: Vec<TimelineSessionJson> = sessions
        .iter()
        .map(|s| TimelineSessionJson {
            session_id: s.session_id.clone(),
            slug: s.slug.clone(),
            project: s.project.clone(),
            model: s.model.clone(),
            start_time: s.start.to_rfc3339(),
            end_time: s.end.to_rfc3339(),
            duration_minutes: s.duration_minutes,
            tokens: s.tokens,
            turns: s.turns,
            cache_hit_ratio: s.cache_hit,
            cost: s.cost,
        })
        .collect();

    let concurrency_json: Vec<ConcurrencySlotJson> = concurrency
        .iter()
        .map(|c| ConcurrencySlotJson {
            time: c.time.to_rfc3339(),
            count: c.count,
            tokens: c.tokens,
        })
        .collect();

    Ok(Json(TimelineJson {
        period_start: axis_start.to_rfc3339(),
        period_end: axis_end.to_rfc3339(),
        period_days: days,
        sessions: timeline_sessions,
        concurrency: concurrency_json,
        peak_concurrent: peak,
        total_sessions: sessions.len(),
    }))
}

pub async fn health() -> &'static str {
    "ok"
}
