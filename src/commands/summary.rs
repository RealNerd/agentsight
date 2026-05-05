use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::cost::{calculate_usage_cost, CostBreakdown};
use crate::output::{format_cost, format_percent, format_tokens};
use crate::output::table::shorten_project;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

pub struct SummaryArgs {
    pub days: u64,
    pub project: Option<String>,
    pub json: bool,
    pub show_cost: bool,
}

pub fn run(claude_dir: &Path, config: &Config, args: &SummaryArgs) -> Result<()> {
    let session_files = session_index::discover_sessions(claude_dir)?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(args.days as i64);

    let mut total_cost = CostBreakdown::default();
    let mut total_tokens = 0u64;
    let mut session_count = 0u64;
    let mut tokens_by_project: HashMap<String, u64> = HashMap::new();
    let mut cost_by_project: HashMap<String, f64> = HashMap::new();
    let mut tokens_by_model: HashMap<String, u64> = HashMap::new();
    let mut cost_by_model: HashMap<String, f64> = HashMap::new();
    let mut total_cache_reads = 0u64;
    let mut total_input_tokens = 0u64;

    for sf in &session_files {
        let project_path = decode_project_path(&sf.project_dir_name);

        if let Some(ref filter) = args.project {
            if !project_path.contains(filter.as_str()) {
                continue;
            }
        }

        let entries = reader::parse_session_file(&sf.path)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        if let Some(start) = summary.start_time {
            if start < cutoff {
                continue;
            }
        } else {
            continue;
        }

        let model_name = summary
            .model
            .as_deref()
            .unwrap_or("claude-opus-4-6")
            .to_string();
        let pricing = config
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
        *tokens_by_project.entry(short_project.clone()).or_default() += session_tokens;
        *cost_by_project.entry(short_project).or_default() += cost_total;

        *tokens_by_model.entry(model_name.clone()).or_default() += session_tokens;
        *cost_by_model.entry(model_name).or_default() += cost_total;

        total_cache_reads += summary.total_usage.cache_read_input_tokens;
        total_input_tokens += summary.total_usage.input_tokens
            + summary.total_usage.cache_creation_input_tokens
            + summary.total_usage.cache_read_input_tokens;

        total_tokens += session_tokens;
        total_cost += cost;
        session_count += 1;
    }

    if args.json {
        println!(
            "{{\"sessions\": {}, \"total_tokens\": {}, \"total_cost\": {:.2}}}",
            session_count,
            total_tokens,
            total_cost.total()
        );
        return Ok(());
    }

    // Header
    println!(
        " ── Last {} Days ──────────────────────────────────",
        args.days
    );
    println!("  Sessions:            {}", session_count);
    println!("  Total tokens:        {}", format_tokens(total_tokens));
    if args.show_cost {
        println!(
            "  Total cost:          {}",
            format_cost(total_cost.total())
        );
    }
    if session_count > 0 {
        println!(
            "  Avg tokens/session:  {}",
            format_tokens(total_tokens / session_count)
        );
        if args.show_cost {
            println!(
                "  Avg cost/session:    {}",
                format_cost(total_cost.total() / session_count as f64)
            );
        }
    }
    println!();

    // By project
    if !tokens_by_project.is_empty() {
        println!(" ── Usage by Project ───────────────────────────────");
        let mut projects: Vec<_> = tokens_by_project.into_iter().collect();
        projects.sort_by(|a, b| b.1.cmp(&a.1));
        for (project, tokens) in &projects {
            let pct = if total_tokens > 0 {
                *tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            };
            let bar = "█".repeat((pct / 5.0) as usize);
            if args.show_cost {
                let cost = cost_by_project.get(project).copied().unwrap_or(0.0);
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
    if !tokens_by_model.is_empty() {
        println!(" ── Usage by Model ─────────────────────────────────");
        let mut models: Vec<_> = tokens_by_model.into_iter().collect();
        models.sort_by(|a, b| b.1.cmp(&a.1));
        for (model, tokens) in &models {
            let pct = if total_tokens > 0 {
                *tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            };
            if args.show_cost {
                let cost = cost_by_model.get(model).copied().unwrap_or(0.0);
                println!(
                    "  {:<28} {} ({:.1}%) {}",
                    model,
                    format_tokens(*tokens),
                    pct,
                    format_cost(cost)
                );
            } else {
                println!(
                    "  {:<28} {} ({:.1}%)",
                    model,
                    format_tokens(*tokens),
                    pct
                );
            }
        }
        println!();
    }

    // Cache performance
    println!(" ── Cache Performance ──────────────────────────────");
    if total_input_tokens > 0 {
        let ratio = total_cache_reads as f64 / total_input_tokens as f64;
        println!("  Avg cache hit ratio:  {}", format_percent(ratio));
        println!(
            "  Total cache reads:    {}",
            format_tokens(total_cache_reads)
        );
    }
    println!();

    Ok(())
}
