use anyhow::Result;
use std::path::Path;

use crate::config::Config;
use crate::cost::calculator::cache_hit_ratio;
use crate::cost::{calculate_usage_cost, CostBreakdown};
use crate::output;
use crate::output::table::SessionRow;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::SessionSummary;

pub struct SessionsArgs {
    pub days: u64,
    pub project: Option<String>,
    pub sort_by: SortField,
    pub limit: usize,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

pub enum SortField {
    Cost,
    Tokens,
    Date,
    Turns,
    Project,
}

pub fn run(claude_dir: &Path, config: &Config, args: &SessionsArgs) -> Result<()> {
    let session_files = session_index::discover_sessions(claude_dir)?;

    let cutoff = chrono::Utc::now() - chrono::Duration::days(args.days as i64);

    let mut results: Vec<(SessionSummary, CostBreakdown, f64)> = Vec::new();

    for sf in &session_files {
        let project_path = decode_project_path(&sf.project_dir_name);

        if let Some(ref filter) = args.project {
            if !project_path.contains(filter.as_str()) {
                continue;
            }
        }

        let entries = reader::parse_session_file(&sf.path, args.verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        if let Some(start) = summary.start_time {
            if start < cutoff {
                continue;
            }
        } else {
            continue;
        }

        let pricing = lookup_pricing(config, &summary);
        let cost = calculate_usage_cost(&summary.total_usage, &pricing);
        let hit = cache_hit_ratio(&summary.total_usage);

        results.push((summary, cost, hit));
    }

    // Sort
    match args.sort_by {
        SortField::Cost => results.sort_by(|a, b| b.1.total().partial_cmp(&a.1.total()).unwrap()),
        SortField::Tokens => {
            results.sort_by_key(|r| std::cmp::Reverse(r.0.total_usage.total_tokens()))
        }
        SortField::Date => results.sort_by_key(|r| std::cmp::Reverse(r.0.start_time)),
        SortField::Turns => results.sort_by_key(|r| std::cmp::Reverse(r.0.turns.len())),
        SortField::Project => results.sort_by_key(|r| r.0.project_path.clone()),
    }

    results.truncate(args.limit);

    if args.json {
        output::json::print_sessions_json(&results, args.show_cost);
    } else {
        let total_tokens: u64 = results
            .iter()
            .map(|(s, _, _)| s.total_usage.total_tokens())
            .sum();
        let total_cost: f64 = results.iter().map(|(_, c, _)| c.total()).sum();

        let rows: Vec<SessionRow> = results
            .iter()
            .map(|(s, c, hit)| SessionRow {
                summary: s,
                cost: c,
                cache_hit: *hit,
            })
            .collect();

        output::table::render_sessions_table(&rows, args.show_cost);

        if args.show_cost {
            println!(
                "\n Total ({} days): {} tokens, {} across {} sessions",
                args.days,
                output::format_tokens(total_tokens),
                output::format_cost(total_cost),
                results.len()
            );
        } else {
            println!(
                "\n Total ({} days): {} tokens across {} sessions",
                args.days,
                output::format_tokens(total_tokens),
                results.len()
            );
        }
    }

    Ok(())
}

fn lookup_pricing(config: &Config, summary: &SessionSummary) -> crate::config::ModelPricing {
    let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
    config
        .pricing_for_model(model_name)
        .cloned()
        .unwrap_or(crate::config::ModelPricing {
            input_per_million: 5.0,
            output_per_million: 25.0,
            cache_creation_per_million: 6.25,
            cache_read_per_million: 0.5,
        })
}
