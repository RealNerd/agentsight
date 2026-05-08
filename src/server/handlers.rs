use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use std::collections::HashMap;

use crate::aggregation::{accumulate_session, SummaryData};
use crate::commands::diagnose::{
    analyze_claude_md, analyze_project_trend, compute_project_benchmark, rank_benchmarks,
    CacheClassification, TrendDirection,
};
use crate::commands::timeline::{compute_concurrency, TimelineSession};
use crate::cost::calculator::cache_hit_ratio;
use crate::output::json::{
    session_to_json, ClaudeMdJson, ConcurrencySlotJson, ConfigJson, ProjectBenchmarkJson,
    ProjectDiagnoseJson, ProjectTrendJson, SessionDetailJson, SessionJson, SessionListJson,
    SummaryJson, TimelineJson, TimelineSessionJson, TokensJson, TrendPointJson, TurnDetailJson,
};
use crate::output::table::shorten_project;

use super::cache::CachedSession;
use super::state::AppState;

const MAX_DAYS: u64 = 365;
const MAX_LIMIT: usize = 500;

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

    let days = query.days.unwrap_or(7).min(MAX_DAYS);
    let limit = query.limit.unwrap_or(50).min(MAX_LIMIT);
    let sort = query.sort.as_deref().unwrap_or("date");
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    let mut items: Vec<&CachedSession> = all
        .iter()
        .map(|arc| arc.as_ref())
        .filter(|cs| {
            // Date filter
            cs.summary.start_time.is_some_and(|start| start >= cutoff)
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
        "tokens" => {
            items.sort_by_key(|cs| std::cmp::Reverse(cs.summary.total_usage.total_tokens()))
        }
        "turns" => items.sort_by_key(|cs| std::cmp::Reverse(cs.summary.turns.len())),
        "cost" => items.sort_by(|a, b| {
            b.cost
                .total()
                .partial_cmp(&a.cost.total())
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        "project" => items.sort_by_key(|cs| cs.project_path.clone()),
        _ => items.sort_by_key(|cs| std::cmp::Reverse(cs.summary.start_time)),
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

    let days = query.days.unwrap_or(7).min(MAX_DAYS);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    let matching: Vec<&CachedSession> = all
        .iter()
        .map(|arc| arc.as_ref())
        .filter(|cs| cs.summary.start_time.is_some_and(|start| start >= cutoff))
        .filter(|cs| match &query.project {
            Some(filter) => cs.project_path.contains(filter.as_str()),
            None => true,
        })
        .collect();

    let mut data = SummaryData::new(days);
    for cs in &matching {
        accumulate_session(&mut data, &cs.summary, &state.config);
    }
    data.finalize();

    Ok(Json(data.to_summary_json(state.show_cost, false)))
}

pub async fn get_config(State(state): State<AppState>) -> Json<ConfigJson> {
    let models: Vec<String> = state.config.models.keys().cloned().collect();

    Json(ConfigJson {
        billing_mode: format!("{:?}", state.config.billing_mode()).to_lowercase(),
        show_cost: state.show_cost,
        models,
    })
}

pub async fn list_projects(State(state): State<AppState>) -> Result<Json<Vec<String>>, StatusCode> {
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

    let days = query.days.unwrap_or(1).min(MAX_DAYS);
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

#[derive(Deserialize)]
pub struct DiagnoseQuery {
    pub days: Option<u64>,
    pub project: Option<String>,
    pub with_context: Option<bool>,
}

pub async fn get_diagnose(
    State(state): State<AppState>,
    Query(query): Query<DiagnoseQuery>,
) -> Result<Json<ProjectDiagnoseJson>, StatusCode> {
    state.cache.refresh().await;

    let days = query.days.unwrap_or(7).min(MAX_DAYS);
    let with_context = query.with_context.unwrap_or(false);
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let all = state.cache.get_all().await;

    // Group sessions by project, applying filters
    let mut by_project: HashMap<String, Vec<&CachedSession>> = HashMap::new();
    let mut project_paths: HashMap<String, String> = HashMap::new();

    for cs in &all {
        // Date filter
        let in_range = cs.summary.start_time.is_some_and(|start| start >= cutoff);
        if !in_range {
            continue;
        }

        // Project filter
        if let Some(ref filter) = query.project {
            if !cs.project_path.contains(filter.as_str()) {
                continue;
            }
        }

        let short = shorten_project(&cs.project_path);
        project_paths
            .entry(short.clone())
            .or_insert_with(|| cs.project_path.clone());
        by_project.entry(short).or_default().push(cs.as_ref());
    }

    // Compute benchmarks using the per-session summaries
    let mut benchmarks: Vec<crate::commands::diagnose::ProjectBenchmark> = by_project
        .iter()
        .map(|(project, sessions)| {
            let summaries: Vec<_> = sessions.iter().map(|cs| cs.summary.clone()).collect();
            compute_project_benchmark(project, &summaries)
        })
        .collect();
    rank_benchmarks(&mut benchmarks);

    // Global averages
    let total_sessions: usize = benchmarks.iter().map(|b| b.session_count).sum();
    let global_avg_cache_hit = if total_sessions > 0 {
        benchmarks
            .iter()
            .map(|b| b.avg_cache_hit * b.session_count as f64)
            .sum::<f64>()
            / total_sessions as f64
    } else {
        0.0
    };
    let global_avg_tokens = if total_sessions > 0 {
        benchmarks
            .iter()
            .map(|b| b.avg_tokens_per_session * b.session_count as u64)
            .sum::<u64>()
            / total_sessions as u64
    } else {
        0
    };

    // Trend (if specific project requested)
    let trend = query.project.as_ref().and_then(|filter| {
        let matching_key = by_project.keys().find(|k| k.contains(filter.as_str()));
        matching_key.and_then(|key| {
            let sessions = by_project.get(key)?;
            let mut summaries: Vec<_> = sessions.iter().map(|cs| cs.summary.clone()).collect();
            summaries.sort_by_key(|s| s.start_time);
            let t = analyze_project_trend(&summaries);
            Some(ProjectTrendJson {
                direction: match t.direction {
                    TrendDirection::Improving => "improving".to_string(),
                    TrendDirection::Declining => "declining".to_string(),
                    TrendDirection::Stable => "stable".to_string(),
                },
                recent_avg_cache_hit: t.recent_avg_cache_hit,
                overall_avg_cache_hit: t.overall_avg_cache_hit,
                points: t
                    .points
                    .iter()
                    .map(|p| {
                        let class_str = match p.classification {
                            CacheClassification::Stable => "stable",
                            CacheClassification::Churning => "churning",
                            CacheClassification::Degrading => "degrading",
                        };
                        TrendPointJson {
                            session_id: p.session_id.clone(),
                            slug: p.slug.clone(),
                            date: p.date.clone(),
                            tokens: p.tokens,
                            cache_hit: p.cache_hit,
                            classification: class_str.to_string(),
                        }
                    })
                    .collect(),
            })
        })
    });

    // CLAUDE.md analysis (if with_context and project specified)
    let claude_md = if with_context {
        query.project.as_ref().and_then(|filter| {
            let matching_key = project_paths.keys().find(|k| k.contains(filter.as_str()));
            matching_key.and_then(|key| {
                let decoded = project_paths.get(key)?;
                // Try to find CLAUDE.md by decoding the raw project path
                let md = analyze_claude_md(decoded, true);
                Some(ClaudeMdJson {
                    exists: md.exists,
                    path: md.path.as_ref().map(|p| p.display().to_string()),
                    size_bytes: md.size_bytes,
                    estimated_tokens: md.estimated_tokens,
                    oversized: md.oversized,
                    content: md.content,
                    recommendations: md.recommendations,
                })
            })
        })
    } else {
        None
    };

    // Recommendations
    let mut recommendations = Vec::new();
    for b in &benchmarks {
        if b.avg_cache_hit < global_avg_cache_hit - 0.1 && b.session_count >= 2 {
            recommendations.push(format!(
                "Project \"{}\" has {:.1}% cache hit vs global {:.1}%.",
                b.project,
                b.avg_cache_hit * 100.0,
                global_avg_cache_hit * 100.0
            ));
        }
    }
    if global_avg_cache_hit < 0.7 {
        recommendations.push(
            "Global cache hit is below 70%. Consider shorter, more focused sessions.".to_string(),
        );
    }

    let benchmark_json: Vec<ProjectBenchmarkJson> = benchmarks
        .iter()
        .map(|b| {
            let class_str = match b.dominant_classification {
                CacheClassification::Stable => "stable",
                CacheClassification::Churning => "churning",
                CacheClassification::Degrading => "degrading",
            };
            ProjectBenchmarkJson {
                project: b.project.clone(),
                session_count: b.session_count,
                avg_tokens_per_session: b.avg_tokens_per_session,
                avg_cache_hit: b.avg_cache_hit,
                dominant_classification: class_str.to_string(),
                bash_loop_count: b.bash_loop_count,
                bash_retry_count: b.bash_retry_count,
                exploration_count: b.exploration_count,
                efficiency_score: b.efficiency_score,
            }
        })
        .collect();

    Ok(Json(ProjectDiagnoseJson {
        period_days: days,
        project_count: benchmark_json.len(),
        global_avg_cache_hit,
        global_avg_tokens,
        benchmarks: benchmark_json,
        trend,
        claude_md,
        recommendations,
    }))
}

pub async fn health() -> &'static str {
    "ok"
}
