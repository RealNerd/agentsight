use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::cost::{calculate_usage_cost, CostBreakdown};
use crate::output::table::{normalize_model_family, shorten_project};
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
    pub by_model: bool,
    pub group_family: bool,
}

/// Per-model aggregated statistics, computed from per-turn accumulation.
#[derive(Debug, Clone, Default)]
pub struct ModelStats {
    pub model: String,
    pub turns: u64,
    pub sessions: u64,
    pub total_input: u64,
    pub total_output: u64,
    pub cache_read: u64,
    pub cache_creation: u64,
    pub total_tokens: u64,
    pub cost: f64,
    // Adherence counters
    pub bash_loop_turns: u64,
    pub bash_retry_count: u64,
    pub read_count: u64,
    pub edit_count: u64,
    pub subagent_count: u64,
}

impl ModelStats {
    pub fn avg_input_per_turn(&self) -> u64 {
        self.total_input.checked_div(self.turns).unwrap_or(0)
    }

    pub fn avg_output_per_turn(&self) -> u64 {
        self.total_output.checked_div(self.turns).unwrap_or(0)
    }

    pub fn cache_hit_ratio(&self) -> f64 {
        if self.total_input == 0 {
            0.0
        } else {
            self.cache_read as f64 / self.total_input as f64
        }
    }

    pub fn cache_creation_ratio(&self) -> f64 {
        if self.total_input == 0 {
            0.0
        } else {
            self.cache_creation as f64 / self.total_input as f64
        }
    }

    pub fn exploration_ratio(&self) -> f64 {
        if self.edit_count == 0 {
            0.0
        } else {
            self.read_count as f64 / self.edit_count as f64
        }
    }

    pub fn bash_loops_per_100t(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.bash_loop_turns as f64 * 100.0 / self.turns as f64
        }
    }

    /// Merge another ModelStats into this one (for --group-family).
    pub fn merge(&mut self, other: &ModelStats) {
        self.turns += other.turns;
        self.sessions += other.sessions;
        self.total_input += other.total_input;
        self.total_output += other.total_output;
        self.cache_read += other.cache_read;
        self.cache_creation += other.cache_creation;
        self.total_tokens += other.total_tokens;
        self.cost += other.cost;
        self.bash_loop_turns += other.bash_loop_turns;
        self.bash_retry_count += other.bash_retry_count;
        self.read_count += other.read_count;
        self.edit_count += other.edit_count;
        self.subagent_count += other.subagent_count;
    }
}

/// Merge ModelStats entries by model family (stripping date suffixes).
pub fn merge_by_family(stats: &[ModelStats]) -> Vec<ModelStats> {
    let mut family_map: HashMap<String, ModelStats> = HashMap::new();
    for s in stats {
        let family = normalize_model_family(&s.model).to_string();
        let entry = family_map
            .entry(family.clone())
            .or_insert_with(|| ModelStats {
                model: family,
                ..Default::default()
            });
        entry.merge(s);
    }
    let mut result: Vec<ModelStats> = family_map.into_values().collect();
    result.sort_by_key(|m| std::cmp::Reverse(m.total_tokens));
    result
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
    /// Per-model aggregated stats from per-turn accumulation.
    pub model_stats: Vec<ModelStats>,
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
        model_stats: Vec::new(),
    };
    // Temporary map for accumulating per-model stats during processing.
    let mut model_stats_map: HashMap<String, ModelStats> = HashMap::new();

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

        accumulate_session(&mut data, &mut model_stats_map, &summary, config);
    }

    data.active_hours = data.by_hour.len() as u64;

    // Finalize model_stats from the accumulated map
    let mut stats: Vec<ModelStats> = model_stats_map.into_values().collect();
    stats.sort_by_key(|m| std::cmp::Reverse(m.total_tokens));
    data.model_stats = stats;

    Ok(data)
}

fn accumulate_session(
    data: &mut SummaryData,
    model_stats_map: &mut HashMap<String, ModelStats>,
    summary: &SessionSummary,
    config: &Config,
) {
    let default_pricing = crate::config::ModelPricing {
        input_per_million: 5.0,
        output_per_million: 25.0,
        cache_creation_per_million: 6.25,
        cache_read_per_million: 0.5,
    };

    let session_tokens = summary.total_usage.total_tokens();

    // Per-project accumulation (unchanged)
    let short_project = shorten_project(&summary.project_path);
    *data
        .tokens_by_project
        .entry(short_project.clone())
        .or_default() += session_tokens;
    *data
        .sessions_by_project
        .entry(short_project.clone())
        .or_default() += 1;

    data.total_cache_reads += summary.total_usage.cache_read_input_tokens;
    data.total_input_tokens += summary.total_usage.input_tokens
        + summary.total_usage.cache_creation_input_tokens
        + summary.total_usage.cache_read_input_tokens;

    // Per-turn model accumulation: attribute each turn's tokens to its model
    let session_model = summary.model.as_deref().unwrap_or("claude-opus-4-6");

    // Track which models appeared in this session (for session count)
    let mut models_in_session: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Detect bash loops per model: track consecutive bash-only streaks
    let mut prev_model: Option<&str> = None;
    let mut bash_streak = 0u64;

    for turn in &summary.turns {
        let turn_model = turn.model.as_deref().unwrap_or(session_model);
        let turn_model_str = turn_model.to_string();
        models_in_session.insert(turn_model_str.clone());

        let turn_input = turn.usage.input_tokens
            + turn.usage.cache_creation_input_tokens
            + turn.usage.cache_read_input_tokens;
        let turn_tokens = turn.usage.total_tokens();

        let pricing = config
            .pricing_for_model(turn_model)
            .cloned()
            .unwrap_or(default_pricing.clone());
        let turn_cost = calculate_usage_cost(&turn.usage, &pricing);

        let ms = model_stats_map
            .entry(turn_model_str.clone())
            .or_insert_with(|| ModelStats {
                model: turn_model_str.clone(),
                ..Default::default()
            });

        ms.turns += 1;
        ms.total_input += turn_input;
        ms.total_output += turn.usage.output_tokens;
        ms.cache_read += turn.usage.cache_read_input_tokens;
        ms.cache_creation += turn.usage.cache_creation_input_tokens;
        ms.total_tokens += turn_tokens;
        ms.cost += turn_cost.total();

        // Tool adherence counters
        for tool in &turn.tools {
            match tool.as_str() {
                "Read" | "Glob" | "Grep" => ms.read_count += 1,
                "Edit" | "Write" => ms.edit_count += 1,
                "Task" => ms.subagent_count += 1,
                _ => {}
            }
        }

        // Bash loop tracking per model
        let is_bash_only = !turn.tools.is_empty() && turn.tools.iter().all(|t| t == "Bash");
        if is_bash_only && prev_model == Some(turn_model) {
            bash_streak += 1;
        } else {
            // Flush previous streak
            if bash_streak >= 3 {
                if let Some(pm) = prev_model {
                    if let Some(prev_ms) = model_stats_map.get_mut(pm) {
                        prev_ms.bash_loop_turns += bash_streak;
                    }
                }
            }
            bash_streak = if is_bash_only { 1 } else { 0 };
        }
        prev_model = if is_bash_only { Some(turn_model) } else { None };

        // Also accumulate into tokens_by_model/cost_by_model for backward compat
        *data
            .tokens_by_model
            .entry(turn_model.to_string())
            .or_default() += turn_tokens;
        *data
            .cost_by_model
            .entry(turn_model.to_string())
            .or_default() += turn_cost.total();

        // Bucket by hour for burn rate
        if let Some(ts) = turn.timestamp {
            let hour_key = ts.format("%Y-%m-%d %H").to_string();
            *data.by_hour.entry(hour_key).or_default() += turn_tokens;
        }
    }

    // Flush trailing bash streak
    if bash_streak >= 3 {
        if let Some(pm) = prev_model {
            if let Some(prev_ms) = model_stats_map.get_mut(pm) {
                prev_ms.bash_loop_turns += bash_streak;
            }
        }
    }

    // Increment session count per model
    for model_name in &models_in_session {
        if let Some(ms) = model_stats_map.get_mut(model_name) {
            ms.sessions += 1;
        }
    }

    // Compute per-bucket session cost from per-turn costs
    let mut session_cost = CostBreakdown::default();
    for turn in &summary.turns {
        let turn_model = turn.model.as_deref().unwrap_or(session_model);
        let pricing = config
            .pricing_for_model(turn_model)
            .cloned()
            .unwrap_or(default_pricing.clone());
        session_cost += calculate_usage_cost(&turn.usage, &pricing);
    }
    let session_cost_total = session_cost.total();

    *data.cost_by_project.entry(short_project).or_default() += session_cost_total;

    // Bucket by day
    if let Some(start) = summary.start_time {
        let date_key = start.format("%Y-%m-%d").to_string();
        let bucket = data.by_day.entry(date_key).or_default();
        bucket.tokens += session_tokens;
        bucket.cost += session_cost_total;
        bucket.sessions += 1;
    }

    data.total_tokens += session_tokens;
    data.total_cost += session_cost;
    data.session_count += 1;
}

/// Build model breakdown JSON entries from model_stats.
fn build_model_breakdown_json(
    stats: &[ModelStats],
    total_tokens: u64,
    show_cost: bool,
    by_model: bool,
) -> Vec<crate::output::json::ModelBreakdownJson> {
    stats
        .iter()
        .map(|ms| {
            let pct = if total_tokens > 0 {
                ms.total_tokens as f64 / total_tokens as f64 * 100.0
            } else {
                0.0
            };
            crate::output::json::ModelBreakdownJson {
                model: ms.model.clone(),
                tokens: ms.total_tokens,
                cost: if show_cost { Some(ms.cost) } else { None },
                pct,
                turns: if by_model { Some(ms.turns) } else { None },
                sessions: if by_model { Some(ms.sessions) } else { None },
                avg_input_per_turn: if by_model {
                    Some(ms.avg_input_per_turn())
                } else {
                    None
                },
                avg_output_per_turn: if by_model {
                    Some(ms.avg_output_per_turn())
                } else {
                    None
                },
                cache_hit_pct: if by_model {
                    Some((ms.cache_hit_ratio() * 1000.0).round() / 10.0)
                } else {
                    None
                },
                bash_loops_per_100t: if by_model {
                    Some((ms.bash_loops_per_100t() * 10.0).round() / 10.0)
                } else {
                    None
                },
                exploration_ratio: if by_model {
                    Some((ms.exploration_ratio() * 10.0).round() / 10.0)
                } else {
                    None
                },
                subagent_count: if by_model {
                    Some(ms.subagent_count)
                } else {
                    None
                },
            }
        })
        .collect()
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
        let peak_hour = data
            .by_hour
            .iter()
            .max_by_key(|(_, v)| *v)
            .map(|(h, t)| serde_json::json!({ "hour": h, "tokens": t }));

        let by_model = build_model_breakdown_json(
            &display_stats,
            data.total_tokens,
            args.show_cost,
            args.by_model,
        );

        let json = serde_json::json!({
            "sessions": data.session_count,
            "total_tokens": data.total_tokens,
            "total_cost": data.total_cost.total(),
            "active_hours": data.active_hours,
            "avg_tokens_per_hour": data.total_tokens / data.active_hours.max(1),
            "peak_hour": peak_hour,
            "by_model": by_model,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::table::normalize_model_family;

    #[test]
    fn test_model_stats_derived_metrics() {
        let ms = ModelStats {
            model: "claude-opus-4-6".to_string(),
            turns: 10,
            sessions: 2,
            total_input: 100_000,
            total_output: 10_000,
            cache_read: 80_000,
            cache_creation: 5_000,
            total_tokens: 110_000,
            cost: 1.50,
            bash_loop_turns: 3,
            bash_retry_count: 1,
            read_count: 20,
            edit_count: 4,
            subagent_count: 2,
        };

        assert_eq!(ms.avg_input_per_turn(), 10_000);
        assert_eq!(ms.avg_output_per_turn(), 1_000);
        assert!((ms.cache_hit_ratio() - 0.80).abs() < 0.001);
        assert!((ms.cache_creation_ratio() - 0.05).abs() < 0.001);
        assert!((ms.exploration_ratio() - 5.0).abs() < 0.001);
        assert!((ms.bash_loops_per_100t() - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_model_stats_zero_turns() {
        let ms = ModelStats::default();
        assert_eq!(ms.avg_input_per_turn(), 0);
        assert_eq!(ms.avg_output_per_turn(), 0);
        assert!((ms.cache_hit_ratio() - 0.0).abs() < 0.001);
        assert!((ms.bash_loops_per_100t() - 0.0).abs() < 0.001);
        assert!((ms.exploration_ratio() - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_model_stats_merge() {
        let mut a = ModelStats {
            model: "opus".to_string(),
            turns: 10,
            sessions: 2,
            total_input: 50_000,
            total_output: 5_000,
            cache_read: 40_000,
            cache_creation: 3_000,
            total_tokens: 55_000,
            cost: 1.00,
            bash_loop_turns: 3,
            bash_retry_count: 1,
            read_count: 10,
            edit_count: 2,
            subagent_count: 1,
        };
        let b = ModelStats {
            model: "opus".to_string(),
            turns: 5,
            sessions: 1,
            total_input: 25_000,
            total_output: 2_500,
            cache_read: 20_000,
            cache_creation: 1_000,
            total_tokens: 27_500,
            cost: 0.50,
            bash_loop_turns: 0,
            bash_retry_count: 0,
            read_count: 5,
            edit_count: 1,
            subagent_count: 0,
        };

        a.merge(&b);
        assert_eq!(a.turns, 15);
        assert_eq!(a.sessions, 3);
        assert_eq!(a.total_input, 75_000);
        assert_eq!(a.total_output, 7_500);
        assert_eq!(a.cache_read, 60_000);
        assert_eq!(a.total_tokens, 82_500);
        assert!((a.cost - 1.50).abs() < 0.001);
        assert_eq!(a.bash_loop_turns, 3);
        assert_eq!(a.read_count, 15);
        assert_eq!(a.edit_count, 3);
    }

    #[test]
    fn test_normalize_model_family_strips_date() {
        assert_eq!(
            normalize_model_family("claude-opus-4-6-20250514"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_model_family("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_normalize_model_family_no_date() {
        assert_eq!(normalize_model_family("claude-opus-4-6"), "claude-opus-4-6");
        assert_eq!(
            normalize_model_family("claude-haiku-3-5"),
            "claude-haiku-3-5"
        );
    }

    #[test]
    fn test_merge_by_family() {
        let stats = vec![
            ModelStats {
                model: "claude-opus-4-6-20250514".to_string(),
                turns: 10,
                sessions: 1,
                total_tokens: 100_000,
                total_input: 80_000,
                total_output: 20_000,
                cache_read: 60_000,
                ..Default::default()
            },
            ModelStats {
                model: "claude-opus-4-6-20250601".to_string(),
                turns: 5,
                sessions: 1,
                total_tokens: 50_000,
                total_input: 40_000,
                total_output: 10_000,
                cache_read: 30_000,
                ..Default::default()
            },
            ModelStats {
                model: "claude-haiku-3-5".to_string(),
                turns: 3,
                sessions: 1,
                total_tokens: 10_000,
                total_input: 8_000,
                total_output: 2_000,
                cache_read: 5_000,
                ..Default::default()
            },
        ];

        let merged = merge_by_family(&stats);
        assert_eq!(merged.len(), 2, "should merge two opus versions into one");

        // Find the opus entry
        let opus = merged
            .iter()
            .find(|m| m.model == "claude-opus-4-6")
            .unwrap();
        assert_eq!(opus.turns, 15);
        assert_eq!(opus.sessions, 2);
        assert_eq!(opus.total_tokens, 150_000);

        let haiku = merged
            .iter()
            .find(|m| m.model == "claude-haiku-3-5")
            .unwrap();
        assert_eq!(haiku.turns, 3);
        assert_eq!(haiku.total_tokens, 10_000);
    }
}
