use anyhow::Result;
use std::path::Path;

use crate::aggregation::{accumulate_session, merge_by_family, SummaryData};
use crate::config::Config;
use crate::output::{format_cost, format_percent, format_tokens};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

pub struct SummaryArgs {
    pub days: u64,
    pub project: Option<String>,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
    pub by_model: bool,
    pub group_family: bool,
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

    let mut data = SummaryData::new(days);

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

    data.finalize();

    Ok(data)
}

pub fn run(claude_dir: &Path, config: &Config, args: &SummaryArgs) -> Result<()> {
    let data = compute_summary(
        claude_dir,
        config,
        args.days,
        args.project.as_deref(),
        args.verbose,
    )?;

    // Prepare model stats (optionally grouped by family)
    let display_stats = if args.group_family {
        merge_by_family(&data.model_stats)
    } else {
        data.model_stats.clone()
    };

    if args.json {
        let json = data.to_summary_json(args.show_cost, args.by_model);
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
    if args.by_model && !display_stats.is_empty() {
        // Detailed model comparison table
        println!(" ── Model Comparison ───────────────────────────────────────────────");
        if args.show_cost {
            println!(
                "  {:<30} {:>5} {:>14} {:>14} {:>7} {:>8} {:>10}",
                "Model", "Turns", "Tok/Turn(in)", "Tok/Turn(out)", "Cache%", "BashL%", "Cost"
            );
        } else {
            println!(
                "  {:<30} {:>5} {:>14} {:>14} {:>7} {:>8}",
                "Model", "Turns", "Tok/Turn(in)", "Tok/Turn(out)", "Cache%", "BashL%"
            );
        }
        for ms in &display_stats {
            if args.show_cost {
                println!(
                    "  {:<30} {:>5} {:>14} {:>14} {:>7} {:>8} {:>10}",
                    ms.model,
                    ms.turns,
                    format_tokens(ms.avg_input_per_turn()),
                    format_tokens(ms.avg_output_per_turn()),
                    format_percent(ms.cache_hit_ratio()),
                    format!("{:.1}", ms.bash_loops_per_100t()),
                    format_cost(ms.cost),
                );
            } else {
                println!(
                    "  {:<30} {:>5} {:>14} {:>14} {:>7} {:>8}",
                    ms.model,
                    ms.turns,
                    format_tokens(ms.avg_input_per_turn()),
                    format_tokens(ms.avg_output_per_turn()),
                    format_percent(ms.cache_hit_ratio()),
                    format!("{:.1}", ms.bash_loops_per_100t()),
                );
            }
        }
        println!();
    } else if !data.tokens_by_model.is_empty() {
        // Compact model breakdown (existing behavior, now from per-turn data)
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
