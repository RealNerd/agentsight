use anyhow::Result;
use std::path::Path;

use crate::config::Config;
use crate::cost::calculator::cache_hit_ratio;
use crate::cost::calculate_usage_cost;
use crate::output;
use crate::output::json::{
    BashLoopJson, CacheStabilityJson, ContextGrowthJson, DiagnoseJson, ToolPatternsJson,
};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::{SessionSummary, TurnSummary};

#[allow(dead_code)]
pub struct DiagnoseArgs {
    pub identifier: Option<String>,
    pub project: Option<String>,
    pub days: u64,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

// ── Analysis data structures ──────────────────────────────────────

#[derive(Debug)]
pub struct DiagnoseData {
    pub cache_stability: CacheStability,
    pub context_growth: ContextGrowth,
    pub tool_patterns: ToolPatterns,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CacheClassification {
    Stable,
    Churning,
    Degrading,
}

#[derive(Debug)]
pub struct CacheStability {
    pub classification: CacheClassification,
    pub turns_above_threshold: usize,
    pub total_turns: usize,
    pub avg_cache_creation_pct: f64,
    pub per_turn_ratios: Vec<f64>,
}

#[derive(Debug)]
pub struct ContextGrowth {
    pub growth_factor: f64,
    pub flagged: bool,
    pub per_turn_input: Vec<u64>,
}

#[derive(Debug, Clone)]
pub struct BashLoop {
    pub start_turn: usize,
    pub length: usize,
}

#[derive(Debug)]
pub struct ToolPatterns {
    pub bash_loops: Vec<BashLoop>,
    pub read_edit_ratio: f64,
    pub exploration_flagged: bool,
    pub subagent_count: usize,
    pub subagent_flagged: bool,
}

// ── Pure analysis functions ───────────────────────────────────────

/// Compute per-turn cache creation ratio and classify the session's cache behavior.
///
/// - Stable: cache creation >30% on early turns, then drops below 15%
/// - Churning: cache creation stays >30% past turn 5
/// - Degrading: second-half average exceeds first-half average
pub fn analyze_cache_stability(turns: &[TurnSummary]) -> CacheStability {
    if turns.len() < 5 {
        return CacheStability {
            classification: CacheClassification::Stable,
            turns_above_threshold: 0,
            total_turns: turns.len(),
            avg_cache_creation_pct: 0.0,
            per_turn_ratios: Vec::new(),
        };
    }

    let per_turn_ratios: Vec<f64> = turns
        .iter()
        .map(|t| {
            let total = t.usage.input_tokens
                + t.usage.cache_creation_input_tokens
                + t.usage.cache_read_input_tokens;
            if total == 0 {
                0.0
            } else {
                t.usage.cache_creation_input_tokens as f64 / total as f64
            }
        })
        .collect();

    let turns_above_threshold = per_turn_ratios.iter().filter(|r| **r > 0.30).count();

    let avg_cache_creation_pct = if per_turn_ratios.is_empty() {
        0.0
    } else {
        per_turn_ratios.iter().sum::<f64>() / per_turn_ratios.len() as f64 * 100.0
    };

    let mid = per_turn_ratios.len() / 2;
    let first_half = &per_turn_ratios[..mid];
    let second_half = &per_turn_ratios[mid..];

    let first_avg = if first_half.is_empty() {
        0.0
    } else {
        first_half.iter().sum::<f64>() / first_half.len() as f64
    };
    let second_avg = if second_half.is_empty() {
        0.0
    } else {
        second_half.iter().sum::<f64>() / second_half.len() as f64
    };

    let classification = if second_avg > first_avg && second_avg > 0.15 {
        CacheClassification::Degrading
    } else if turns_above_threshold > 5 {
        CacheClassification::Churning
    } else {
        CacheClassification::Stable
    };

    CacheStability {
        classification,
        turns_above_threshold,
        total_turns: turns.len(),
        avg_cache_creation_pct,
        per_turn_ratios,
    }
}

/// Track total input tokens per turn and flag if input grows >2x from turn 5 to final.
pub fn analyze_context_growth(turns: &[TurnSummary]) -> ContextGrowth {
    let per_turn_input: Vec<u64> = turns
        .iter()
        .map(|t| {
            t.usage.input_tokens
                + t.usage.cache_creation_input_tokens
                + t.usage.cache_read_input_tokens
        })
        .collect();

    let (growth_factor, flagged) = if per_turn_input.len() > 5 {
        let turn5_input = per_turn_input[4];
        let final_input = *per_turn_input.last().unwrap_or(&0);
        if turn5_input > 0 {
            let factor = final_input as f64 / turn5_input as f64;
            (factor, factor > 2.0)
        } else {
            (0.0, false)
        }
    } else {
        (0.0, false)
    };

    ContextGrowth {
        growth_factor,
        flagged,
        per_turn_input,
    }
}

/// Detect tool usage anti-patterns:
/// - Bash loops: 3+ consecutive turns where the only tool calls are Bash
/// - Exploration heavy: Read+Glob calls > 5x Edit+Write calls
/// - Subagent overhead: >3 Task tool calls
pub fn analyze_tool_patterns(turns: &[TurnSummary]) -> ToolPatterns {
    // Bash loop detection
    let mut bash_loops = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;

    for (i, turn) in turns.iter().enumerate() {
        let is_bash_only =
            !turn.tools.is_empty() && turn.tools.iter().all(|t| t == "Bash");

        if is_bash_only {
            if streak_start.is_none() {
                streak_start = Some(i);
                streak_len = 1;
            } else {
                streak_len += 1;
            }
        } else {
            if streak_len >= 3 {
                bash_loops.push(BashLoop {
                    start_turn: streak_start.unwrap(),
                    length: streak_len,
                });
            }
            streak_start = None;
            streak_len = 0;
        }
    }
    // Flush trailing streak
    if streak_len >= 3 {
        bash_loops.push(BashLoop {
            start_turn: streak_start.unwrap(),
            length: streak_len,
        });
    }

    // Read/Edit ratio from aggregated tool counts across all turns
    let mut read_glob_count: u32 = 0;
    let mut edit_write_count: u32 = 0;
    let mut task_count: usize = 0;

    for turn in turns {
        for tool in &turn.tools {
            match tool.as_str() {
                "Read" | "Glob" | "Grep" => read_glob_count += 1,
                "Edit" | "Write" => edit_write_count += 1,
                "Task" => task_count += 1,
                _ => {}
            }
        }
    }

    let read_edit_ratio = if edit_write_count > 0 {
        read_glob_count as f64 / edit_write_count as f64
    } else if read_glob_count > 0 {
        read_glob_count as f64 // treat 0 edits as ratio = read count
    } else {
        0.0
    };

    ToolPatterns {
        bash_loops,
        read_edit_ratio,
        exploration_flagged: read_edit_ratio > 5.0,
        subagent_count: task_count,
        subagent_flagged: task_count > 3,
    }
}

/// Build plain-english recommendation strings from flagged patterns.
pub fn generate_recommendations(
    cache: &CacheStability,
    growth: &ContextGrowth,
    tools: &ToolPatterns,
) -> Vec<String> {
    let mut recs = Vec::new();

    match cache.classification {
        CacheClassification::Churning => {
            recs.push(format!(
                "Cache creation stayed above 30% on {} of {} turns. Break multi-topic work into focused sessions.",
                cache.turns_above_threshold, cache.total_turns
            ));
        }
        CacheClassification::Degrading => {
            recs.push(
                "Cache creation ratio increased over the session — context is being rebuilt. Consider shorter, focused sessions."
                    .to_string(),
            );
        }
        CacheClassification::Stable => {}
    }

    if growth.flagged {
        recs.push(format!(
            "Input per turn grew {:.1}x. Start a new session before context compaction hits.",
            growth.growth_factor
        ));
    }

    if !tools.bash_loops.is_empty() {
        let total_turns: usize = tools.bash_loops.iter().map(|l| l.length).sum();
        recs.push(format!(
            "{} Bash retry sequence{} detected ({} turns). Add build/test commands to CLAUDE.md.",
            tools.bash_loops.len(),
            if tools.bash_loops.len() > 1 { "s" } else { "" },
            total_turns
        ));
    }

    if tools.exploration_flagged {
        recs.push(format!(
            "Read:Edit ratio is {:.0}:1. Add a project map to CLAUDE.md listing key source files.",
            tools.read_edit_ratio
        ));
    }

    if tools.subagent_flagged {
        recs.push(format!(
            "{} Task (subagent) calls detected. Each spawns a new context window — consider inlining simpler operations.",
            tools.subagent_count
        ));
    }

    recs
}

/// Run all diagnostic analyses on a session summary.
pub fn run_diagnose(summary: &SessionSummary) -> DiagnoseData {
    let cache_stability = analyze_cache_stability(&summary.turns);
    let context_growth = analyze_context_growth(&summary.turns);
    let tool_patterns = analyze_tool_patterns(&summary.turns);
    let recommendations =
        generate_recommendations(&cache_stability, &context_growth, &tool_patterns);

    DiagnoseData {
        cache_stability,
        context_growth,
        tool_patterns,
        recommendations,
    }
}

// ── CLI entry point ───────────────────────────────────────────────

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

    let summary = match &args.identifier {
        Some(id) => resolve_session(&filtered, id, args.verbose)?,
        None => most_recent_session(&filtered, args.verbose)?,
    };

    let pricing = lookup_pricing(config, summary.model.as_deref());
    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
    let hit = cache_hit_ratio(&summary.total_usage);

    let diag = run_diagnose(&summary);

    if args.json {
        render_json(&summary, &diag, &cost, hit, args.show_cost);
    } else {
        render_text(&summary, &diag, &cost, hit, args.show_cost);
    }

    Ok(())
}

fn resolve_session(
    session_files: &[&session_index::SessionFile],
    identifier: &str,
    verbose: bool,
) -> Result<SessionSummary> {
    // Try UUID prefix match
    if let Some(sf) = session_files
        .iter()
        .find(|sf| sf.session_id.starts_with(identifier))
    {
        let project_path = decode_project_path(&sf.project_dir_name);
        let entries = reader::parse_session_file(&sf.path, verbose)?;
        return Ok(reader::summarize_session(
            &entries,
            sf.session_id.clone(),
            project_path,
        ));
    }

    // Try slug match
    let id_lower = identifier.to_lowercase();
    let mut best: Option<SessionSummary> = None;

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
            (Some(prev), Some(new_start)) => match prev.start_time {
                Some(prev_start) => new_start > prev_start,
                None => true,
            },
            _ => false,
        };

        if is_newer {
            best = Some(summary);
        }
    }

    best.ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", identifier))
}

fn most_recent_session(
    session_files: &[&session_index::SessionFile],
    verbose: bool,
) -> Result<SessionSummary> {
    if session_files.is_empty() {
        anyhow::bail!("No session files found");
    }

    // Sort by file mtime, take most recent
    let mut files_with_mtime: Vec<_> = session_files
        .iter()
        .filter_map(|sf| {
            sf.path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(|mtime| (*sf, mtime))
        })
        .collect();

    files_with_mtime.sort_by_key(|item| std::cmp::Reverse(item.1));

    let (sf, _) = files_with_mtime
        .first()
        .ok_or_else(|| anyhow::anyhow!("No session files with readable metadata"))?;

    let project_path = decode_project_path(&sf.project_dir_name);
    let entries = reader::parse_session_file(&sf.path, verbose)?;
    Ok(reader::summarize_session(
        &entries,
        sf.session_id.clone(),
        project_path,
    ))
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

// ── Text rendering ────────────────────────────────────────────────

fn render_text(
    summary: &SessionSummary,
    diag: &DiagnoseData,
    cost: &crate::cost::CostBreakdown,
    hit: f64,
    show_cost: bool,
) {
    let slug_display = summary
        .slug
        .as_deref()
        .unwrap_or(&summary.session_id[..8]);
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
    println!(
        " Project:  {}",
        crate::output::table::shorten_project(&summary.project_path)
    );
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

    // Tool Patterns
    println!();
    println!(" ── Tool Patterns ─────────────────────────────────────────");
    if diag.tool_patterns.bash_loops.is_empty()
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

fn render_json(
    summary: &SessionSummary,
    diag: &DiagnoseData,
    cost: &crate::cost::CostBreakdown,
    hit: f64,
    show_cost: bool,
) {
    let session = output::json::session_to_json(summary, cost, hit, show_cost);

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
            read_edit_ratio: diag.tool_patterns.read_edit_ratio,
            exploration_flagged: diag.tool_patterns.exploration_flagged,
            subagent_count: diag.tool_patterns.subagent_count,
            subagent_flagged: diag.tool_patterns.subagent_flagged,
        },
        recommendations: diag.recommendations.clone(),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::TokenUsage;

    fn make_turn(index: usize, input: u64, cache_creation: u64, cache_read: u64, output: u64, tools: Vec<&str>) -> TurnSummary {
        TurnSummary {
            index,
            timestamp: None,
            usage: TokenUsage {
                input_tokens: input,
                cache_creation_input_tokens: cache_creation,
                cache_read_input_tokens: cache_read,
                output_tokens: output,
                cache_creation: None,
                service_tier: None,
            },
            tools: tools.into_iter().map(String::from).collect(),
            model: None,
        }
    }

    #[test]
    fn test_cache_stability_stable() {
        // High early cache creation that drops off → Stable
        let turns: Vec<TurnSummary> = (0..10)
            .map(|i| {
                if i < 3 {
                    // Early turns: high cache creation (60%)
                    make_turn(i, 100, 600, 300, 50, vec![])
                } else {
                    // Later turns: low cache creation (5%), high cache reads
                    make_turn(i, 100, 50, 850, 50, vec![])
                }
            })
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Stable);
        assert_eq!(result.total_turns, 10);
    }

    #[test]
    fn test_cache_stability_churning() {
        // Sustained high cache creation across all turns → Churning
        let turns: Vec<TurnSummary> = (0..10)
            .map(|i| {
                // 50% cache creation on every turn
                make_turn(i, 100, 500, 400, 50, vec![])
            })
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Churning);
        assert!(result.turns_above_threshold > 5);
    }

    #[test]
    fn test_cache_stability_degrading() {
        // Cache creation increases over the session → Degrading
        let turns: Vec<TurnSummary> = (0..10)
            .map(|i| {
                if i < 5 {
                    // First half: low cache creation (5%)
                    make_turn(i, 100, 50, 850, 50, vec![])
                } else {
                    // Second half: high cache creation (40%)
                    make_turn(i, 100, 400, 500, 50, vec![])
                }
            })
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Degrading);
    }

    #[test]
    fn test_cache_stability_short_session() {
        // <5 turns → Stable (too short to classify)
        let turns: Vec<TurnSummary> = (0..3)
            .map(|i| make_turn(i, 100, 500, 400, 50, vec![]))
            .collect();

        let result = analyze_cache_stability(&turns);
        assert_eq!(result.classification, CacheClassification::Stable);
        assert_eq!(result.total_turns, 3);
    }

    #[test]
    fn test_context_growth_flagged() {
        // Input grows >2x from turn 5 to final → flagged
        let turns: Vec<TurnSummary> = (0..10)
            .map(|i| {
                let input = 10_000 * (1 + i as u64 * i as u64); // quadratic growth
                make_turn(i, input, 0, 0, 100, vec![])
            })
            .collect();

        let result = analyze_context_growth(&turns);
        // turn 5 (i=4): 10000*(1+16) = 170000, final (i=9): 10000*(1+81) = 820000
        // growth = 820000/170000 ≈ 4.8x
        assert!(result.flagged);
        assert!(result.growth_factor > 2.0);
    }

    #[test]
    fn test_context_growth_flat() {
        // Stable input → not flagged
        let turns: Vec<TurnSummary> = (0..10)
            .map(|i| make_turn(i, 10_000, 0, 0, 100, vec![]))
            .collect();

        let result = analyze_context_growth(&turns);
        assert!(!result.flagged);
        assert!((result.growth_factor - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_bash_loop_detection() {
        // 3+ consecutive Bash-only turns detected
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read"]),
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(4, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(5, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert_eq!(result.bash_loops.len(), 1);
        assert_eq!(result.bash_loops[0].start_turn, 1);
        assert_eq!(result.bash_loops[0].length, 4);
    }

    #[test]
    fn test_bash_loop_no_false_positive() {
        // Mixed tools → no bash loop flag
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Bash", "Read"]),
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Edit"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(4, 100, 0, 0, 50, vec!["Read"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert!(result.bash_loops.is_empty());
    }

    #[test]
    fn test_exploration_ratio_flagged() {
        // >5:1 Read:Edit → flagged
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read", "Read", "Glob"]),
            make_turn(1, 100, 0, 0, 50, vec!["Read", "Read", "Grep"]),
            make_turn(2, 100, 0, 0, 50, vec!["Read"]),
            make_turn(3, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert!(result.exploration_flagged);
        assert!(result.read_edit_ratio > 5.0);
    }

    #[test]
    fn test_bash_loop_long_streak_with_interruptions() {
        // Models the real-world pattern: agent retries Bash many times,
        // occasionally emitting a text-only turn ("I need to cd...") with no tools.
        // The text-only turn breaks the streak, but multiple sequences still get flagged.
        let turns = vec![
            make_turn(0, 100, 0, 0, 50, vec!["Read"]),
            // First bash streak: 5 turns
            make_turn(1, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(2, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(3, 100, 0, 0, 50, vec!["Bash", "Bash"]), // multiple Bash in one turn
            make_turn(4, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(5, 100, 0, 0, 50, vec!["Bash"]),
            // Text-only interruption (agent says "let me try cd") — no tools
            make_turn(6, 100, 0, 0, 50, vec![]),
            // Second bash streak: 4 turns
            make_turn(7, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(8, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(9, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(10, 100, 0, 0, 50, vec!["Bash"]),
            make_turn(11, 100, 0, 0, 50, vec!["Edit"]),
        ];

        let result = analyze_tool_patterns(&turns);
        assert_eq!(result.bash_loops.len(), 2);
        assert_eq!(result.bash_loops[0].start_turn, 1);
        assert_eq!(result.bash_loops[0].length, 5);
        assert_eq!(result.bash_loops[1].start_turn, 7);
        assert_eq!(result.bash_loops[1].length, 4);
    }

    #[test]
    fn test_recommendations_cover_all_flags() {
        // Set up all flags
        let cache = CacheStability {
            classification: CacheClassification::Churning,
            turns_above_threshold: 8,
            total_turns: 10,
            avg_cache_creation_pct: 45.0,
            per_turn_ratios: vec![],
        };
        let growth = ContextGrowth {
            growth_factor: 3.0,
            flagged: true,
            per_turn_input: vec![],
        };
        let tools = ToolPatterns {
            bash_loops: vec![BashLoop {
                start_turn: 5,
                length: 4,
            }],
            read_edit_ratio: 8.0,
            exploration_flagged: true,
            subagent_count: 5,
            subagent_flagged: true,
        };

        let recs = generate_recommendations(&cache, &growth, &tools);

        // Should have recommendations for: churning, growth, bash loops, exploration, subagents
        assert_eq!(recs.len(), 5);
        assert!(recs[0].contains("30%"));
        assert!(recs[1].contains("3.0x"));
        assert!(recs[2].contains("Bash"));
        assert!(recs[3].contains("Read:Edit"));
        assert!(recs[4].contains("Task"));
    }
}
