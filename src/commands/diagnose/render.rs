use crate::output;
use crate::output::json::{
    BashLoopJson, BashRetryJson, CacheStabilityJson, ClaudeMdJson, ContextGrowthJson, DiagnoseJson,
    ProjectBenchmarkJson, ProjectDiagnoseJson, ProjectTrendJson, ToolPatternsJson, TrendPointJson,
};
use crate::output::table::shorten_project;
use crate::parser::types::SessionSummary;

use super::models::collect_model_distribution;
use super::types::{
    BashRetryPattern, CacheClassification, DiagnoseData, ProjectDiagnoseData, TrendDirection,
};
use super::ClearUrgency;

pub fn classification_str(c: &CacheClassification) -> &'static str {
    match c {
        CacheClassification::Stable => "stable",
        CacheClassification::Churning => "churning",
        CacheClassification::Degrading => "degrading",
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}

fn bash_retry_to_json(retry: &super::types::BashRetry) -> BashRetryJson {
    match &retry.pattern {
        BashRetryPattern::IdenticalCommand { command } => BashRetryJson {
            pattern: "identical_command".to_string(),
            command: Some(command.clone()),
            error_snippet: None,
            start_turn: retry.start_turn,
            length: retry.length,
        },
        BashRetryPattern::SameError {
            command,
            error_snippet,
        } => BashRetryJson {
            pattern: "same_error".to_string(),
            command: Some(command.clone()),
            error_snippet: Some(error_snippet.clone()),
            start_turn: retry.start_turn,
            length: retry.length,
        },
    }
}

// ── Text rendering ────────────────────────────────────────────────

pub(super) fn render_text(
    summary: &SessionSummary,
    diag: &DiagnoseData,
    cost: &crate::cost::CostBreakdown,
    hit: f64,
    show_cost: bool,
) {
    let slug_display = summary.slug.as_deref().unwrap_or(&summary.session_id[..8]);
    let duration = match (summary.start_time, summary.end_time) {
        (Some(start), Some(end)) => {
            let dur = end - start;
            let mins = dur.num_minutes();
            if mins >= 60 {
                format!("{}h {}m", mins / 60, mins % 60)
            } else {
                format!("{}m", mins)
            }
        }
        _ => "unknown".to_string(),
    };

    println!();
    println!(
        " ── Diagnose: {} ─────────────────────────────────",
        slug_display
    );
    println!(" Project:  {}", shorten_project(&summary.project_path));
    println!(
        " Tokens:   {} across {} turns ({})",
        output::format_tokens(summary.total_usage.total_tokens()),
        summary.turns.len(),
        duration
    );
    println!(" Cache:    {} hit ratio", output::format_percent(hit));
    if show_cost {
        println!(" Cost:     {}", output::format_cost(cost.total()));
    }

    // Model Distribution (only if multiple models used)
    let model_distribution = collect_model_distribution(&summary.turns);
    if model_distribution.len() > 1 {
        println!();
        println!(" ── Model Distribution ────────────────────────────────────");
        for (model, count) in &model_distribution {
            let pct = *count as f64 / summary.turns.len() as f64 * 100.0;
            println!("  {:<30} {} turns ({:.0}%)", model, count, pct);
        }
    }

    // Cache Stability
    println!();
    println!(" ── Cache Stability ───────────────────────────────────────");
    let class_str = match diag.cache_stability.classification {
        CacheClassification::Stable => "STABLE",
        CacheClassification::Churning => "CHURNING",
        CacheClassification::Degrading => "DEGRADING",
    };
    println!(" Classification: {}", class_str);
    match diag.cache_stability.classification {
        CacheClassification::Churning => {
            println!(
                " Cache creation stayed above 30% on {} of {} turns.",
                diag.cache_stability.turns_above_threshold, diag.cache_stability.total_turns
            );
        }
        CacheClassification::Degrading => {
            println!(" Cache creation ratio increased over the session.");
        }
        CacheClassification::Stable => {
            println!(" Cache front-loading looks healthy.");
        }
    }

    // Context Growth
    println!();
    println!(" ── Context Growth ────────────────────────────────────────");
    if diag.context_growth.flagged {
        println!(
            " Input per turn grew {:.1}x over this session (turns 5→{}).",
            diag.context_growth.growth_factor,
            summary.turns.len()
        );
    } else {
        println!(" Context size remained stable across the session.");
    }

    // /clear Advisor
    println!();
    println!(" ── /clear Advisor ────────────────────────────────────────");
    let advice = &diag.clear_advice;
    let badge = match advice.urgency {
        ClearUrgency::Healthy => "[ok]",
        ClearUrgency::Consider => "[~]",
        ClearUrgency::Recommend => "[!]",
    };
    println!(" {} {}", badge, advice.headline());
    println!(
        " Context now: {} carried/turn ({:.0}% of ~{} window, peak {})",
        output::format_tokens(advice.current_context_tokens),
        advice.context_fraction * 100.0,
        output::format_tokens(advice.context_window),
        output::format_tokens(advice.peak_context_tokens),
    );
    for reason in &advice.reasons {
        println!("     - {}", reason);
    }

    // Tool Patterns
    let has_same_error = diag
        .same_error_retries
        .as_ref()
        .is_some_and(|v| !v.is_empty());

    println!();
    println!(" ── Tool Patterns ─────────────────────────────────────────");
    if diag.tool_patterns.bash_loops.is_empty()
        && diag.tool_patterns.bash_retries.is_empty()
        && !has_same_error
        && !diag.tool_patterns.exploration_flagged
        && !diag.tool_patterns.subagent_flagged
    {
        println!(" No concerning tool patterns detected.");
    } else {
        if !diag.tool_patterns.bash_loops.is_empty() {
            let total_turns: usize = diag.tool_patterns.bash_loops.iter().map(|l| l.length).sum();
            println!(
                " [!] Bash loops: {} sequence{} ({} turns total)",
                diag.tool_patterns.bash_loops.len(),
                if diag.tool_patterns.bash_loops.len() > 1 {
                    "s"
                } else {
                    ""
                },
                total_turns
            );
        }
        if !diag.tool_patterns.bash_retries.is_empty() {
            let total_turns: usize = diag
                .tool_patterns
                .bash_retries
                .iter()
                .map(|r| r.length)
                .sum();
            println!(
                " [!] Identical command retries: {} sequence{} ({} turns)",
                diag.tool_patterns.bash_retries.len(),
                if diag.tool_patterns.bash_retries.len() > 1 {
                    "s"
                } else {
                    ""
                },
                total_turns
            );
            for retry in &diag.tool_patterns.bash_retries {
                if let BashRetryPattern::IdenticalCommand { ref command } = retry.pattern {
                    let display_cmd: String = command.chars().take(60).collect();
                    println!(
                        "     Turn {}-{}: `{}` ({}x)",
                        retry.start_turn,
                        retry.start_turn + retry.length - 1,
                        display_cmd,
                        retry.length
                    );
                }
            }
        }
        if let Some(ref error_retries) = diag.same_error_retries {
            if !error_retries.is_empty() {
                let total_turns: usize = error_retries.iter().map(|r| r.length).sum();
                println!(
                    " [!] Same-error retries: {} sequence{} ({} turns)",
                    error_retries.len(),
                    if error_retries.len() > 1 { "s" } else { "" },
                    total_turns
                );
                for retry in error_retries {
                    println!(
                        "     Turn {}-{}: same error repeated {}x",
                        retry.start_turn,
                        retry.start_turn + retry.length - 1,
                        retry.length
                    );
                }
            }
        }
        if diag.tool_patterns.exploration_flagged {
            println!(
                " [!] Exploration heavy: Read:Edit ratio is {:.0}:1",
                diag.tool_patterns.read_edit_ratio
            );
        }
        if diag.tool_patterns.subagent_flagged {
            println!(
                " [!] Subagent overhead: {} Task calls",
                diag.tool_patterns.subagent_count
            );
        }
    }

    // Recommendations
    if !diag.recommendations.is_empty() {
        println!();
        println!(" ── Recommendations ───────────────────────────────────────");
        for (i, rec) in diag.recommendations.iter().enumerate() {
            println!(" {}. {}", i + 1, rec);
        }
    }

    println!();
}

// ── JSON rendering ────────────────────────────────────────────────

pub(super) fn render_json(
    summary: &SessionSummary,
    diag: &DiagnoseData,
    cost: &crate::cost::CostBreakdown,
    hit: f64,
    show_cost: bool,
) {
    use crate::output::json::ModelDistributionJson;

    let session = output::json::session_to_json(summary, cost, hit, show_cost);

    let model_dist = collect_model_distribution(&summary.turns);
    let model_distribution = if model_dist.len() > 1 {
        let total = summary.turns.len() as f64;
        Some(
            model_dist
                .into_iter()
                .map(|(model, count)| ModelDistributionJson {
                    model,
                    turns: count,
                    pct: if total > 0.0 {
                        (count as f64 / total * 1000.0).round() / 10.0
                    } else {
                        0.0
                    },
                })
                .collect(),
        )
    } else {
        None
    };

    let json = DiagnoseJson {
        session,
        cache_stability: CacheStabilityJson {
            classification: match diag.cache_stability.classification {
                CacheClassification::Stable => "stable".to_string(),
                CacheClassification::Churning => "churning".to_string(),
                CacheClassification::Degrading => "degrading".to_string(),
            },
            turns_above_threshold: diag.cache_stability.turns_above_threshold,
            total_turns: diag.cache_stability.total_turns,
            avg_cache_creation_pct: diag.cache_stability.avg_cache_creation_pct,
            per_turn_ratios: diag.cache_stability.per_turn_ratios.clone(),
        },
        context_growth: ContextGrowthJson {
            growth_factor: diag.context_growth.growth_factor,
            flagged: diag.context_growth.flagged,
            per_turn_input: diag.context_growth.per_turn_input.clone(),
        },
        tool_patterns: ToolPatternsJson {
            bash_loops: diag
                .tool_patterns
                .bash_loops
                .iter()
                .map(|l| BashLoopJson {
                    start_turn: l.start_turn,
                    length: l.length,
                })
                .collect(),
            bash_retries: diag
                .tool_patterns
                .bash_retries
                .iter()
                .map(bash_retry_to_json)
                .collect(),
            read_edit_ratio: diag.tool_patterns.read_edit_ratio,
            exploration_flagged: diag.tool_patterns.exploration_flagged,
            subagent_count: diag.tool_patterns.subagent_count,
            subagent_flagged: diag.tool_patterns.subagent_flagged,
        },
        same_error_retries: diag
            .same_error_retries
            .as_ref()
            .map(|retries| retries.iter().map(bash_retry_to_json).collect()),
        model_distribution,
        clear_advice: super::clear_advice_to_json(&diag.clear_advice),
        recommendations: diag.recommendations.clone(),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

// ── Project-level text rendering ──────────────────────────────────

pub(super) fn render_project_text(data: &ProjectDiagnoseData, days: u64) {
    println!();
    println!(
        " ── Project Diagnostics ({} days) ─────────────────────",
        days
    );

    if data.benchmarks.is_empty() {
        println!(" No sessions found for this period.");
        println!();
        return;
    }

    // Ranking table
    println!(
        " {:<3} {:<30} {:>8} {:>12} {:>9} {:>6}",
        "#", "Project", "Sessions", "Tokens/Sess", "Cache Hit", "Score"
    );
    for (i, b) in data.benchmarks.iter().enumerate() {
        println!(
            " {:<3} {:<30} {:>8} {:>12} {:>9} {:>6}",
            i + 1,
            truncate_str(&b.project, 30),
            b.session_count,
            output::format_tokens(b.avg_tokens_per_session),
            output::format_percent(b.avg_cache_hit),
            format!("{:.2}", b.efficiency_score),
        );
    }
    println!(
        " Global Avg Cache Hit: {}",
        output::format_percent(data.global_avg_cache_hit)
    );

    // Trend (if present)
    if let Some(ref trend) = data.trend {
        println!();
        println!(" ── Project Trend ─────────────────────────────────────────");
        let dir_str = match trend.direction {
            TrendDirection::Improving => "IMPROVING",
            TrendDirection::Declining => "DECLINING",
            TrendDirection::Stable => "STABLE",
        };
        println!(" Direction: {}", dir_str);
        println!(
            " Recent (last 5): {} -> Overall: {}",
            output::format_percent(trend.recent_avg_cache_hit),
            output::format_percent(trend.overall_avg_cache_hit)
        );

        if !trend.points.is_empty() {
            println!();
            println!(
                " {:<20} {:>12} {:>9} {:>10}",
                "Date", "Tokens", "Cache Hit", "Class"
            );
            for p in &trend.points {
                println!(
                    " {:<20} {:>12} {:>9} {:>10}",
                    p.date.as_deref().unwrap_or("—"),
                    output::format_tokens(p.tokens),
                    output::format_percent(p.cache_hit),
                    classification_str(&p.classification),
                );
            }
        }
    }

    // CLAUDE.md analysis
    if let Some(ref md) = data.claude_md {
        println!();
        println!(" ── CLAUDE.md Analysis ────────────────────────────────────");
        if md.exists {
            println!(
                " Path: {}",
                md.path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
            println!(
                " Size: ~{} tokens ({} bytes) — {}",
                md.estimated_tokens,
                md.size_bytes,
                if md.oversized { "OVERSIZED" } else { "Healthy" }
            );
        } else {
            println!(" Not found.");
        }

        for rec in &md.recommendations {
            println!(" [!] {}", rec);
        }
    }

    // Recommendations
    if !data.recommendations.is_empty() {
        println!();
        println!(" ── Recommendations ───────────────────────────────────────");
        for (i, rec) in data.recommendations.iter().enumerate() {
            println!(" {}. {}", i + 1, rec);
        }
    }

    println!();
}

// ── Project-level JSON rendering ─────────────────────────────────

pub(super) fn render_project_json(data: &ProjectDiagnoseData, days: u64) {
    let benchmarks: Vec<ProjectBenchmarkJson> = data
        .benchmarks
        .iter()
        .map(|b| ProjectBenchmarkJson {
            project: b.project.clone(),
            session_count: b.session_count,
            avg_tokens_per_session: b.avg_tokens_per_session,
            avg_cache_hit: b.avg_cache_hit,
            dominant_classification: classification_str(&b.dominant_classification).to_string(),
            bash_loop_count: b.bash_loop_count,
            bash_retry_count: b.bash_retry_count,
            exploration_count: b.exploration_count,
            efficiency_score: b.efficiency_score,
        })
        .collect();

    let trend = data.trend.as_ref().map(|t| {
        let dir = match t.direction {
            TrendDirection::Improving => "improving",
            TrendDirection::Declining => "declining",
            TrendDirection::Stable => "stable",
        };
        ProjectTrendJson {
            direction: dir.to_string(),
            recent_avg_cache_hit: t.recent_avg_cache_hit,
            overall_avg_cache_hit: t.overall_avg_cache_hit,
            points: t
                .points
                .iter()
                .map(|p| TrendPointJson {
                    session_id: p.session_id.clone(),
                    slug: p.slug.clone(),
                    date: p.date.clone(),
                    tokens: p.tokens,
                    cache_hit: p.cache_hit,
                    classification: classification_str(&p.classification).to_string(),
                })
                .collect(),
        }
    });

    let claude_md = data.claude_md.as_ref().map(|md| ClaudeMdJson {
        exists: md.exists,
        path: md.path.as_ref().map(|p| p.display().to_string()),
        size_bytes: md.size_bytes,
        estimated_tokens: md.estimated_tokens,
        oversized: md.oversized,
        content: md.content.clone(),
        recommendations: md.recommendations.clone(),
    });

    let json = ProjectDiagnoseJson {
        period_days: days,
        project_count: data.benchmarks.len(),
        global_avg_cache_hit: data.global_avg_cache_hit,
        global_avg_tokens: data.global_avg_tokens,
        benchmarks,
        trend,
        claude_md,
        recommendations: data.recommendations.clone(),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}
