use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::cost::calculate_usage_cost;
use crate::cost::calculator::cache_hit_ratio;
use crate::output::table::shorten_project;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::{SessionEntry, SessionSummary};

use super::benchmark::{compute_project_benchmark, rank_benchmarks};
use super::claude_md::analyze_claude_md;
use super::recommendations::generate_project_recommendations;
use super::render::{render_json, render_project_json, render_project_text, render_text};
use super::trend::analyze_project_trend;
use super::types::{DiagnoseArgs, ProjectDiagnoseData};

pub fn run(claude_dir: &Path, config: &Config, args: &DiagnoseArgs) -> Result<()> {
    let session_files = session_index::discover_sessions(claude_dir)?;

    // Filter by project if specified (substring match on decoded project path)
    let filtered: Vec<&session_index::SessionFile> = if let Some(ref filter) = args.project {
        session_files
            .iter()
            .filter(|sf| decode_project_path(&sf.project_dir_name).contains(filter.as_str()))
            .collect()
    } else {
        session_files.iter().collect()
    };

    // If an identifier is given, run single-session mode
    if let Some(ref id) = args.identifier {
        let (summary, entries) = resolve_session(&filtered, id, args.verbose)?;
        return run_single_session(config, args, &summary, &entries);
    }

    // No identifier: project-level overview
    run_project_level(claude_dir, config, args, &filtered)
}

fn run_single_session(
    config: &Config,
    args: &DiagnoseArgs,
    summary: &SessionSummary,
    entries: &[SessionEntry],
) -> Result<()> {
    let pricing = lookup_pricing(config, summary.model.as_deref());
    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
    let hit = cache_hit_ratio(&summary.total_usage);

    let diag = super::run_diagnose_with_entries(summary, Some(entries));

    if args.json {
        render_json(summary, &diag, &cost, hit, args.show_cost);
    } else {
        render_text(summary, &diag, &cost, hit, args.show_cost);
    }

    Ok(())
}

fn run_project_level(
    _claude_dir: &Path,
    _config: &Config,
    args: &DiagnoseArgs,
    session_files: &[&session_index::SessionFile],
) -> Result<()> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(args.days as i64);

    // Parse all sessions and group by project
    let mut by_project: HashMap<String, Vec<SessionSummary>> = HashMap::new();
    // Track the raw decoded path for each project key (for CLAUDE.md lookup)
    let mut project_paths: HashMap<String, String> = HashMap::new();

    for sf in session_files {
        let decoded = decode_project_path(&sf.project_dir_name);
        let entries = match reader::parse_session_file(&sf.path, args.verbose) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), decoded.clone());

        // Date filter
        if let Some(start) = summary.start_time {
            if start < cutoff {
                continue;
            }
        }

        let short = shorten_project(&decoded);
        project_paths.entry(short.clone()).or_insert(decoded);
        by_project.entry(short).or_default().push(summary);
    }

    // Compute benchmarks
    let mut benchmarks: Vec<_> = by_project
        .iter()
        .map(|(project, summaries)| compute_project_benchmark(project, summaries))
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

    // Trend (only if a specific project is selected)
    let trend = args.project.as_ref().and_then(|filter| {
        let matching_key = by_project.keys().find(|k| k.contains(filter.as_str()));
        matching_key.and_then(|key| {
            let summaries = by_project.get(key)?;
            let mut sorted = summaries.clone();
            sorted.sort_by_key(|s| s.start_time);
            Some(analyze_project_trend(&sorted))
        })
    });

    // CLAUDE.md analysis (only if --with-context and --project specified)
    let claude_md = if args.with_context {
        args.project.as_ref().and_then(|filter| {
            let matching_key = project_paths.keys().find(|k| k.contains(filter.as_str()));
            matching_key.and_then(|key| {
                let decoded = project_paths.get(key)?;
                Some(analyze_claude_md(decoded, true))
            })
        })
    } else {
        None
    };

    let recommendations = generate_project_recommendations(&benchmarks, global_avg_cache_hit);

    let data = ProjectDiagnoseData {
        benchmarks,
        global_avg_cache_hit,
        global_avg_tokens,
        trend,
        claude_md,
        recommendations,
    };

    if args.json {
        render_project_json(&data, args.days);
    } else {
        render_project_text(&data, args.days);
    }

    Ok(())
}

fn resolve_session(
    session_files: &[&session_index::SessionFile],
    identifier: &str,
    verbose: bool,
) -> Result<(SessionSummary, Vec<SessionEntry>)> {
    // Try UUID prefix match
    if let Some(sf) = session_files
        .iter()
        .find(|sf| sf.session_id.starts_with(identifier))
    {
        let project_path = decode_project_path(&sf.project_dir_name);
        let entries = reader::parse_session_file(&sf.path, verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);
        return Ok((summary, entries));
    }

    // Try slug match
    let id_lower = identifier.to_lowercase();
    let mut best: Option<(SessionSummary, Vec<SessionEntry>)> = None;

    for sf in session_files {
        let project_path = decode_project_path(&sf.project_dir_name);
        let entries = reader::parse_session_file(&sf.path, verbose)?;
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), project_path);

        let matches = match summary.slug.as_deref() {
            Some(s) => {
                let s_lower = s.to_lowercase();
                s_lower == id_lower || s_lower.contains(&id_lower)
            }
            None => false,
        };

        if !matches {
            continue;
        }

        let is_newer = match (&best, summary.start_time) {
            (None, _) => true,
            (Some((prev, _)), Some(new_start)) => match prev.start_time {
                Some(prev_start) => new_start > prev_start,
                None => true,
            },
            _ => false,
        };

        if is_newer {
            best = Some((summary, entries));
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", identifier))
}

fn lookup_pricing(config: &Config, model: Option<&str>) -> crate::config::ModelPricing {
    let model_name = model.unwrap_or("claude-opus-4-6");
    config
        .pricing_for_model(model_name)
        .cloned()
        .unwrap_or_default()
}
