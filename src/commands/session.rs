use anyhow::Result;
use std::path::Path;

use crate::config::Config;
use crate::cost::calculate_usage_cost;
use crate::cost::calculator::cache_hit_ratio;
use crate::output;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

pub struct SessionArgs {
    pub identifier: String,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

pub fn run(claude_dir: &Path, config: &Config, args: &SessionArgs) -> Result<()> {
    let session_files = session_index::discover_sessions(claude_dir)?;

    // Try UUID prefix match first
    let sf = session_files
        .iter()
        .find(|sf| sf.session_id.starts_with(&args.identifier));

    let sf = match sf {
        Some(sf) => sf,
        None => {
            // Try slug match by scanning session files
            return find_by_slug(&session_files, &args.identifier, config, args);
        }
    };

    let project_path = decode_project_path(&sf.project_dir_name);
    let entries = reader::parse_session_file(&sf.path, args.verbose)?;
    let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

    let pricing = lookup_pricing(config, summary.model.as_deref());
    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
    let hit = cache_hit_ratio(&summary.total_usage);

    if args.json {
        output::json::print_session_json(&summary, &cost, hit, args.show_cost);
    } else {
        output::table::render_session_detail(&summary, &cost, hit, args.show_cost);
    }

    Ok(())
}

fn find_by_slug(
    session_files: &[session_index::SessionFile],
    slug: &str,
    config: &Config,
    args: &SessionArgs,
) -> Result<()> {
    // Collect all sessions that match by slug (exact, then substring).
    // When multiple sessions share a slug (e.g. plan mode → implementation),
    // pick the most recent one.
    use crate::cost::CostBreakdown;
    use crate::parser::types::SessionSummary;

    let mut best: Option<(SessionSummary, CostBreakdown, f64)> = None;
    let slug_lower = slug.to_lowercase();

    for sf in session_files {
        let project_path = decode_project_path(&sf.project_dir_name);
        let entries = reader::parse_session_file(&sf.path, args.verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        let matches = match summary.slug.as_deref() {
            Some(s) => {
                let s_lower = s.to_lowercase();
                s_lower == slug_lower || s_lower.contains(&slug_lower)
            }
            None => false,
        };

        if !matches {
            continue;
        }

        let is_newer = match (&best, summary.start_time) {
            (None, _) => true,
            (Some((prev, _, _)), Some(new_start)) => match prev.start_time {
                Some(prev_start) => new_start > prev_start,
                None => true,
            },
            _ => false,
        };

        if is_newer {
            let pricing = lookup_pricing(config, summary.model.as_deref());
            let cost = calculate_usage_cost(&summary.total_usage, &pricing);
            let hit = cache_hit_ratio(&summary.total_usage);
            best = Some((summary, cost, hit));
        }
    }

    match best {
        Some((summary, cost, hit)) => {
            if args.json {
                output::json::print_session_json(&summary, &cost, hit, args.show_cost);
            } else {
                output::table::render_session_detail(&summary, &cost, hit, args.show_cost);
            }
            Ok(())
        }
        None => anyhow::bail!("No session found matching '{}'", slug),
    }
}

fn lookup_pricing(config: &Config, model: Option<&str>) -> crate::config::ModelPricing {
    let model_name = model.unwrap_or("claude-opus-4-6");
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
