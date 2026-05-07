use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::cost::{calculate_usage_cost, CostBreakdown};
use crate::output::table::shorten_project;
use crate::output::{format_cost, format_percent, format_tokens};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::SessionSummary;

pub struct SummaryArgs {
    pub days: u64,
    pub project: Option<String>,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

/// Aggregated summary data, reusable by both CLI and dashboard API.
#[allow(dead_code)]
pub struct SummaryData {
    pub days: u64,
    pub total_cost: CostBreakdown,
    pub total_tokens: u64,
    pub session_count: u64,
    pub tokens_by_project: HashMap<String, u64>,
    pub cost_by_project: HashMap<String, f64>,
    pub sessions_by_project: HashMap<String, u64>,
    pub tokens_by_model: HashMap<String, u64>,
    pub cost_by_model: HashMap<String, f64>,
    pub total_cache_reads: u64,
    pub total_input_tokens: u64,
    /// Sessions bucketed by date (YYYY-MM-DD) for daily trends.
    pub by_day: HashMap<String, DayBucket>,
    /// Tokens bucketed by hour (YYYY-MM-DD HH) for burn rate analysis.
    pub by_hour: HashMap<String, u64>,
    /// Total active hours (hours with any token activity).
    pub active_hours: u64,
}

#[derive(Default)]
pub struct DayBucket {
    pub tokens: u64,
    pub cost: f64,
    pub sessions: u64,
}

/// Compute aggregated summary from all sessions matching the given criteria.
pub fn compute_summary(
    claude_dir: &Path,
    config: &Config,
    days: u64,
    project_filter: Option<&str>,
    verbose: bool,
) -> Result<SummaryData> {
    let session_files = session_index::discover_sessions(claude_dir)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);

    let mut data = SummaryData {
        days,
        total_cost: CostBreakdown::default(),
        total_tokens: 0,
        session_count: 0,
        tokens_by_project: HashMap::new(),
        cost_by_project: HashMap::new(),
        sessions_by_project: HashMap::new(),
        tokens_by_model: HashMap::new(),
        cost_by_model: HashMap::new(),
        total_cache_reads: 0,
        total_input_tokens: 0,
        by_day: HashMap::new(),
        by_hour: HashMap::new(),
        active_hours: 0,
    };

    for sf in &session_files {
        let project_path = decode_project_path(&sf.project_dir_name);

        if let Some(filter) = project_filter {
            if !project_path.contains(filter) {
                continue;
            }
        }

        let entries = reader::parse_session_file(&sf.path, verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        if let Some(start) = summary.start_time {
            if start < cutoff {
                continue;
            }
        } else {
            continue;
        }

        accumulate_session(&mut data, &summary, config);
    }

    data.active_hours = data.by_hour.len() as u64;

    Ok(data)
}

fn accumulate_session(data: &mut SummaryData, summary: &SessionSummary, config: &Config) {
    let model_name = summary
        .model
        .as_deref()
        .unwrap_or("claude-opus-4-6")
        .to_string();
    let pricing =
        config
            .pricing_for_model(&model_name)
            .cloned()
            .unwrap_or(crate::config::ModelPricing {
                input_per_million: 5.0,
                output_per_million: 25.0,
                cache_creation_per_million: 6.25,
                cache_read_per_million: 0.5,
            });

    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
    let cost_total = cost.total();
    let session_tokens = summary.total_usage.total_tokens();

    let short_project = shorten_project(&summary.project_path);
    *data
        .tokens_by_project
        .entry(short_project.clone())
        .or_default() += session_tokens;
    *data
        .cost_by_project
        .entry(short_project.clone())
        .or_default() += cost_total;
    *data.sessions_by_project.entry(short_project).or_default() += 1;

    *data.tokens_by_model.entry(model_name.clone()).or_default() += session_tokens;
    *data.cost_by_model.entry(model_name).or_default() += cost_total;

    data.total_cache_reads += summary.total_usage.cache_read_input_tokens;
    data.total_input_tokens += summary.total_usage.input_tokens
        + summary.total_usage.cache_creation_input_tokens
        + summary.total_usage.cache_read_input_tokens;

    // Bucket by day
    if let Some(start) = summary.start_time {
        let date_key = start.format("%Y-%m-%d").to_string();
        let bucket = data.by_day.entry(date_key).or_default();
        bucket.tokens += session_tokens;
        bucket.cost += cost_total;
        bucket.sessions += 1;
    }

    // Bucket turns by hour for burn rate
    for turn in &summary.turns {
        if let Some(ts) = turn.timestamp {
            let hour_key = ts.format("%Y-%m-%d %H").to_string();
            *data.by_hour.entry(hour_key).or_default() += turn.usage.total_tokens();
        }
    }

    data.total_tokens += session_tokens;
    data.total_cost += cost;
    data.session_count += 1;
}

pub fn run(claude_dir: &Path, config: &Config, args: &SummaryArgs) -> Result<()> {
    let data = compute_summary(
        claude_dir,
        config,
        args.days,
        args.project.as_deref(),
        args.verbose,
    )?;

    if args.json {
        let peak_hour = data
            .by_hour
            .iter()
            .max_by_key(|(_, v)| *v)
            .map(|(h, t)| serde_json::json!({ "hour": h, "tokens": t }));

        let json = serde_json::json!({
            "sessions": data.session_count,
            "total_tokens": data.total_tokens,
            "total_cost": data.total_cost.total(),
            "active_hours": data.active_hours,
            "avg_tokens_per_hour": data.total_tokens / data.active_hours.max(1),
            "peak_hour": peak_hour,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&json).unwrap_or_default()
        );
        return Ok(());
    }

    // Header
    println!(
        " ── Last {} Days ──────────────────────────────────",
        args.days
    );
    println!("  Sessions:            {}", data.session_count);
    println!(
        "  Total tokens:        {}",
        format_tokens(data.total_tokens)
    );
    if args.show_cost {
        println!(
            "  Total cost:          {}",
            format_cost(data.total_cost.total())
        );
    }
    if let Some(avg) = data.total_tokens.checked_div(data.session_count) {
        println!("  Avg tokens/session:  {}", format_tokens(avg));
        if args.show_cost {
            println!(
                "  Avg cost/session:    {}",
                format_cost(data.total_cost.total() / data.session_count as f64)
            );
        }
    }
    println!();

    // By project
    if !data.tokens_by_project.is_empty() {
        println!(" ── Usage by Project ───────────────────────────────");
        let mut projects: Vec<_> = data.tokens_by_project.into_iter().collect();
        projects.sort_by_key(|p| std::cmp::Reverse(p.1));
        for (project, tokens) in &projects {
            let pct = if data.total_tokens > 0 {
                *tokens as f64 / data.total_tokens as f64 * 100.0
            } else {
                0.0
            };
            let bar = "█".repeat((pct / 5.0) as usize);
            if args.show_cost {
                let cost = data.cost_by_project.get(project).copied().unwrap_or(0.0);
                println!(
                    "  {:<24} {} ({:.1}%) {} {}",
                    project,
                    format_tokens(*tokens),
                    pct,
                    format_cost(cost),
                    bar
                );
            } else {
                println!(
                    "  {:<24} {} ({:.1}%) {}",
                    project,
                    format_tokens(*tokens),
                    pct,
                    bar
                );
            }
        }
        println!();
    }

    // By model
    if !data.tokens_by_model.is_empty() {
        println!(" ── Usage by Model ─────────────────────────────────");
        let mut models: Vec<_> = data.tokens_by_model.into_iter().collect();
        models.sort_by_key(|m| std::cmp::Reverse(m.1));
        for (model, tokens) in &models {
            let pct = if data.total_tokens > 0 {
                *tokens as f64 / data.total_tokens as f64 * 100.0
            } else {
                0.0
            };
            if args.show_cost {
                let cost = data.cost_by_model.get(model).copied().unwrap_or(0.0);
                println!(
                    "  {:<28} {} ({:.1}%) {}",
                    model,
                    format_tokens(*tokens),
                    pct,
                    format_cost(cost)
                );
            } else {
                println!("  {:<28} {} ({:.1}%)", model, format_tokens(*tokens), pct);
            }
        }
        println!();
    }

    // Cache performance
    println!(" ── Cache Performance ──────────────────────────────");
    if data.total_input_tokens > 0 {
        let ratio = data.total_cache_reads as f64 / data.total_input_tokens as f64;
        println!("  Avg cache hit ratio:  {}", format_percent(ratio));
        println!(
            "  Total cache reads:    {}",
            format_tokens(data.total_cache_reads)
        );
    }
    println!();

    // Burn rate
    if !data.by_hour.is_empty() {
        println!(" ── Burn Rate ─────────────────────────────────────");
        let active_hours = data.active_hours.max(1);
        let avg_per_hour = data.total_tokens / active_hours;
        let peak_hour = data.by_hour.iter().max_by_key(|(_, v)| *v);
        println!("  Active hours:         {}", active_hours);
        println!("  Avg tokens/hour:      {}", format_tokens(avg_per_hour));
        if let Some((hour, tokens)) = peak_hour {
            // Extract just the HH part for display
            let hour_label = hour.split(' ').next_back().unwrap_or(hour);
            println!(
                "  Peak hour:            {} ({}:00)",
                format_tokens(*tokens),
                hour_label
            );
        }
        if args.show_cost {
            let avg_cost_per_hour = data.total_cost.total() / active_hours as f64;
            println!("  Avg cost/hour:        {}", format_cost(avg_cost_per_hour));
        }
        println!();
    }

    Ok(())
}
