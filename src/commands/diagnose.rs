use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::cost::calculate_usage_cost;
use crate::cost::calculator::cache_hit_ratio;
use crate::output;
use crate::output::json::{
    BashLoopJson, BashRetryJson, CacheStabilityJson, ClaudeMdJson, ContextGrowthJson, DiagnoseJson,
    ProjectBenchmarkJson, ProjectDiagnoseJson, ProjectTrendJson, ToolPatternsJson, TrendPointJson,
};
use crate::output::table::shorten_project;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::{ContentBlock, SessionEntry, SessionSummary, TurnSummary};

#[allow(dead_code)]
pub struct DiagnoseArgs {
    pub identifier: Option<String>,
    pub project: Option<String>,
    pub days: u64,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
    pub with_context: bool,
}

// ── Analysis data structures ──────────────────────────────────────

#[derive(Debug)]
pub struct DiagnoseData {
    pub cache_stability: CacheStability,
    pub context_growth: ContextGrowth,
    pub tool_patterns: ToolPatterns,
    /// Same-error retries detected from entry-level analysis.
    /// None when entry-level analysis is not available (project-level path).
    pub same_error_retries: Option<Vec<BashRetry>>,
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

#[derive(Debug, Clone)]
pub enum BashRetryPattern {
    IdenticalCommand {
        command: String,
    },
    SameError {
        command: String,
        error_snippet: String,
    },
}

#[derive(Debug, Clone)]
pub struct BashRetry {
    pub pattern: BashRetryPattern,
    pub start_turn: usize,
    pub length: usize,
}

#[derive(Debug)]
pub struct ToolPatterns {
    pub bash_loops: Vec<BashLoop>,
    pub bash_retries: Vec<BashRetry>,
    pub read_edit_ratio: f64,
    pub exploration_flagged: bool,
    pub subagent_count: usize,
    pub subagent_flagged: bool,
}

// ── Project-level analysis data structures ────────────────────────

#[derive(Debug, Clone)]
pub struct ProjectBenchmark {
    pub project: String,
    pub session_count: usize,
    pub avg_tokens_per_session: u64,
    pub avg_cache_hit: f64,
    pub dominant_classification: CacheClassification,
    pub bash_loop_count: usize,
    pub bash_retry_count: usize,
    pub exploration_count: usize,
    pub efficiency_score: f64,
}

#[derive(Debug, Clone)]
pub struct SessionTrendPoint {
    pub session_id: String,
    pub slug: Option<String>,
    pub date: Option<String>,
    pub tokens: u64,
    pub cache_hit: f64,
    pub classification: CacheClassification,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrendDirection {
    Improving,
    Declining,
    Stable,
}

#[derive(Debug, Clone)]
pub struct ProjectTrend {
    pub points: Vec<SessionTrendPoint>,
    pub direction: TrendDirection,
    pub recent_avg_cache_hit: f64,
    pub overall_avg_cache_hit: f64,
}

#[derive(Debug, Clone)]
pub struct ClaudeMdAnalysis {
    pub exists: bool,
    pub path: Option<PathBuf>,
    pub size_bytes: u64,
    pub estimated_tokens: u64,
    pub oversized: bool,
    pub content: Option<String>,
    pub recommendations: Vec<String>,
}

#[derive(Debug)]
pub struct ProjectDiagnoseData {
    pub benchmarks: Vec<ProjectBenchmark>,
    pub global_avg_cache_hit: f64,
    pub global_avg_tokens: u64,
    pub trend: Option<ProjectTrend>,
    pub claude_md: Option<ClaudeMdAnalysis>,
    pub recommendations: Vec<String>,
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
        let is_bash_only = !turn.tools.is_empty() && turn.tools.iter().all(|t| t == "Bash");

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

    let bash_retries = detect_identical_command_retries(turns);

    ToolPatterns {
        bash_loops,
        bash_retries,
        read_edit_ratio,
        exploration_flagged: read_edit_ratio > 5.0,
        subagent_count: task_count,
        subagent_flagged: task_count > 3,
    }
}

/// Detect 3+ consecutive turns where the first bash command is identical.
pub fn detect_identical_command_retries(turns: &[TurnSummary]) -> Vec<BashRetry> {
    let mut retries = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;
    let mut streak_cmd: Option<&str> = None;

    for (i, turn) in turns.iter().enumerate() {
        let first_cmd = turn.bash_commands.first().map(|s| s.as_str());

        let continues = match (first_cmd, streak_cmd) {
            (Some(current), Some(prev)) => current == prev,
            _ => false,
        };

        if continues {
            streak_len += 1;
        } else {
            // Flush previous streak
            if streak_len >= 3 {
                if let Some(cmd) = streak_cmd {
                    retries.push(BashRetry {
                        pattern: BashRetryPattern::IdenticalCommand {
                            command: cmd.to_string(),
                        },
                        start_turn: streak_start.unwrap(),
                        length: streak_len,
                    });
                }
            }

            // Start new streak if this turn has a bash command
            if first_cmd.is_some() {
                streak_start = Some(i);
                streak_len = 1;
                streak_cmd = first_cmd;
            } else {
                streak_start = None;
                streak_len = 0;
                streak_cmd = None;
            }
        }
    }

    // Flush trailing streak
    if streak_len >= 3 {
        if let Some(cmd) = streak_cmd {
            retries.push(BashRetry {
                pattern: BashRetryPattern::IdenticalCommand {
                    command: cmd.to_string(),
                },
                start_turn: streak_start.unwrap(),
                length: streak_len,
            });
        }
    }

    retries
}

// ── Same-error detection (entry-level) ────────────────────────

/// Strip ANSI escape sequences from a string.
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip ESC [ ... <letter> sequences
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Normalize an error string for comparison: strip ANSI, shorten paths, normalize line numbers, truncate.
fn normalize_error(s: &str) -> String {
    let stripped = strip_ansi(s);

    // Shorten filesystem paths to last 2 segments
    let mut normalized = String::with_capacity(stripped.len());
    let mut i = 0;
    let bytes = stripped.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] != b' ' {
            // Found start of a path — collect the whole path
            let start = i;
            let mut end = i + 1;
            while end < bytes.len() && !matches!(bytes[end], b' ' | b'\n' | b'\t' | b':' | b')') {
                end += 1;
            }
            let path = &stripped[start..end];
            let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            if segments.len() > 2 {
                normalized.push_str(".../");
                normalized.push_str(&segments[segments.len() - 2..].join("/"));
            } else {
                normalized.push_str(path);
            }
            i = end;
        } else {
            normalized.push(bytes[i] as char);
            i += 1;
        }
    }

    // Normalize line numbers like :123: to :_:
    let mut result = String::with_capacity(normalized.len());
    let mut chars = normalized.chars().peekable();
    while let Some(c) = chars.next() {
        result.push(c);
        if c == ':' {
            // Check if followed by digits then ':'
            let mut digits = String::new();
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    digits.push(d);
                    chars.next();
                } else {
                    break;
                }
            }
            if !digits.is_empty() && chars.peek() == Some(&':') {
                result.push('_');
                result.push(':');
                chars.next(); // consume the trailing ':'
            } else {
                result.push_str(&digits);
            }
        }
    }

    // Truncate to 200 chars
    result.chars().take(200).collect()
}

/// Detect 3+ consecutive Bash calls that produce the same error output.
pub fn detect_same_error_retries(entries: &[SessionEntry]) -> Vec<BashRetry> {
    // Step 1: Walk assistant entries to build a map of tool_use_id → (command, turn_index)
    let mut tool_commands: HashMap<String, (String, usize)> = HashMap::new();
    let mut turn_index = 0;

    for entry in entries {
        if let SessionEntry::Assistant(assistant) = entry {
            if let Some(content) = &assistant.message.content {
                for block in content {
                    if let ContentBlock::ToolUse { id, name, input } = block {
                        if name == "Bash" {
                            if let Some(cmd) = input.get("command").and_then(|v| v.as_str()) {
                                tool_commands.insert(
                                    id.clone(),
                                    (cmd.chars().take(500).collect(), turn_index),
                                );
                            }
                        }
                    }
                }
            }
            turn_index += 1;
        }
    }

    // Step 2: Walk user entries to find tool_result blocks with is_error == true
    struct ErrorResult {
        command: String,
        normalized_error: String,
        turn_index: usize,
    }

    let mut error_results: Vec<ErrorResult> = Vec::new();

    for entry in entries {
        if let SessionEntry::User(user) = entry {
            if let Some(content) = &user.message.content {
                // Content can be an array of tool_result objects
                if let Some(arr) = content.as_array() {
                    for item in arr {
                        let is_error = item
                            .get("is_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if !is_error {
                            continue;
                        }

                        let tool_use_id = match item.get("tool_use_id").and_then(|v| v.as_str()) {
                            Some(id) => id,
                            None => continue,
                        };

                        let error_content =
                            item.get("content").and_then(|v| v.as_str()).unwrap_or("");

                        if let Some((cmd, tidx)) = tool_commands.get(tool_use_id) {
                            error_results.push(ErrorResult {
                                command: cmd.clone(),
                                normalized_error: normalize_error(error_content),
                                turn_index: *tidx,
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by turn_index
    error_results.sort_by_key(|e| e.turn_index);

    // Step 3: Detect 3+ consecutive same-error streaks
    let mut retries = Vec::new();
    let mut streak_start: Option<usize> = None;
    let mut streak_len = 0;
    let mut streak_error: Option<&str> = None;
    let mut streak_cmd: Option<&str> = None;

    for (i, er) in error_results.iter().enumerate() {
        let continues = match streak_error {
            Some(prev) => prev == er.normalized_error,
            None => false,
        };

        if continues {
            streak_len += 1;
        } else {
            if streak_len >= 3 {
                retries.push(BashRetry {
                    pattern: BashRetryPattern::SameError {
                        command: streak_cmd.unwrap_or("").to_string(),
                        error_snippet: streak_error.unwrap_or("").to_string(),
                    },
                    start_turn: error_results[streak_start.unwrap()].turn_index,
                    length: streak_len,
                });
            }
            streak_start = Some(i);
            streak_len = 1;
            streak_error = Some(&er.normalized_error);
            streak_cmd = Some(&er.command);
        }
    }

    // Flush trailing streak
    if streak_len >= 3 {
        retries.push(BashRetry {
            pattern: BashRetryPattern::SameError {
                command: streak_cmd.unwrap_or("").to_string(),
                error_snippet: streak_error.unwrap_or("").to_string(),
            },
            start_turn: error_results[streak_start.unwrap()].turn_index,
            length: streak_len,
        });
    }

    retries
}

/// Collect per-model turn counts from a session's turns, sorted descending by count.
pub fn collect_model_distribution(turns: &[TurnSummary]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for turn in turns {
        let model = turn.model.as_deref().unwrap_or("unknown").to_string();
        *counts.entry(model).or_default() += 1;
    }
    let mut result: Vec<(String, usize)> = counts.into_iter().collect();
    result.sort_by_key(|item| std::cmp::Reverse(item.1));
    result
}

/// Build plain-english recommendation strings from flagged patterns.
pub fn generate_recommendations(
    cache: &CacheStability,
    growth: &ContextGrowth,
    tools: &ToolPatterns,
    same_error_retries: Option<&[BashRetry]>,
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

    if !tools.bash_retries.is_empty() {
        recs.push(format!(
            "{} identical command retry sequence(s) detected. The agent re-ran the same command without changing its approach.",
            tools.bash_retries.len()
        ));
    }

    if let Some(error_retries) = same_error_retries {
        if !error_retries.is_empty() {
            recs.push(format!(
                "{} same-error retry sequence(s) detected. The agent got the same error repeatedly without adapting. Add troubleshooting steps to CLAUDE.md.",
                error_retries.len()
            ));
        }
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
    run_diagnose_with_entries(summary, None)
}

/// Run all diagnostic analyses, optionally with entry-level same-error detection.
pub fn run_diagnose_with_entries(
    summary: &SessionSummary,
    entries: Option<&[SessionEntry]>,
) -> DiagnoseData {
    let cache_stability = analyze_cache_stability(&summary.turns);
    let context_growth = analyze_context_growth(&summary.turns);
    let tool_patterns = analyze_tool_patterns(&summary.turns);
    let same_error_retries = entries.map(detect_same_error_retries);
    let recommendations = generate_recommendations(
        &cache_stability,
        &context_growth,
        &tool_patterns,
        same_error_retries.as_deref(),
    );

    DiagnoseData {
        cache_stability,
        context_growth,
        tool_patterns,
        same_error_retries,
        recommendations,
    }
}

// ── Project-level analysis functions ──────────────────────────────

/// Compute a benchmark for a single project from its session summaries.
pub fn compute_project_benchmark(project: &str, summaries: &[SessionSummary]) -> ProjectBenchmark {
    if summaries.is_empty() {
        return ProjectBenchmark {
            project: project.to_string(),
            session_count: 0,
            avg_tokens_per_session: 0,
            avg_cache_hit: 0.0,
            dominant_classification: CacheClassification::Stable,
            bash_loop_count: 0,
            bash_retry_count: 0,
            exploration_count: 0,
            efficiency_score: 0.0,
        };
    }

    let mut total_tokens: u64 = 0;
    let mut total_cache_hit: f64 = 0.0;
    let mut classifications: HashMap<String, usize> = HashMap::new();
    let mut bash_loop_total = 0;
    let mut bash_retry_total = 0;
    let mut exploration_total = 0;

    for summary in summaries {
        let diag = run_diagnose(summary);
        let hit = cache_hit_ratio(&summary.total_usage);

        total_tokens += summary.total_usage.total_tokens();
        total_cache_hit += hit;

        let class_key = match diag.cache_stability.classification {
            CacheClassification::Stable => "stable",
            CacheClassification::Churning => "churning",
            CacheClassification::Degrading => "degrading",
        };
        *classifications.entry(class_key.to_string()).or_default() += 1;

        bash_loop_total += diag.tool_patterns.bash_loops.len();
        bash_retry_total += diag.tool_patterns.bash_retries.len();
        if diag.tool_patterns.exploration_flagged {
            exploration_total += 1;
        }
    }

    let n = summaries.len();
    let avg_tokens = total_tokens / n as u64;
    let avg_cache_hit = total_cache_hit / n as f64;

    let dominant_classification = {
        let max_entry = classifications.iter().max_by_key(|(_, v)| *v);
        match max_entry.map(|(k, _)| k.as_str()) {
            Some("churning") => CacheClassification::Churning,
            Some("degrading") => CacheClassification::Degrading,
            _ => CacheClassification::Stable,
        }
    };

    let score = efficiency_score(
        avg_cache_hit,
        bash_loop_total as f64 / n as f64,
        exploration_total as f64 / n as f64,
        &dominant_classification,
    );

    ProjectBenchmark {
        project: project.to_string(),
        session_count: n,
        avg_tokens_per_session: avg_tokens,
        avg_cache_hit,
        dominant_classification,
        bash_loop_count: bash_loop_total,
        bash_retry_count: bash_retry_total,
        exploration_count: exploration_total,
        efficiency_score: score,
    }
}

/// Weighted composite efficiency score (0.0–1.0).
/// - Cache hit: 40% weight (higher is better)
/// - Low bash loops: 20% weight (fewer is better, 0 loops = 1.0, 2+ avg = 0.0)
/// - Low exploration: 20% weight (fewer flagged sessions = better)
/// - Stable classification: 20% weight (Stable = 1.0, Churning/Degrading = 0.0)
pub fn efficiency_score(
    avg_cache_hit: f64,
    bash_loop_rate: f64,
    exploration_rate: f64,
    classification: &CacheClassification,
) -> f64 {
    let cache_score = avg_cache_hit.clamp(0.0, 1.0);
    let bash_score = (1.0 - bash_loop_rate / 2.0).clamp(0.0, 1.0);
    let exploration_score = (1.0 - exploration_rate).clamp(0.0, 1.0);
    let class_score = match classification {
        CacheClassification::Stable => 1.0,
        CacheClassification::Churning | CacheClassification::Degrading => 0.0,
    };

    let score = cache_score * 0.4 + bash_score * 0.2 + exploration_score * 0.2 + class_score * 0.2;
    (score * 100.0).round() / 100.0 // round to 2 decimal places
}

/// Sort benchmarks descending by efficiency score.
pub fn rank_benchmarks(benchmarks: &mut [ProjectBenchmark]) {
    benchmarks.sort_by(|a, b| {
        b.efficiency_score
            .partial_cmp(&a.efficiency_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

// ── Project trend analysis ───────────────────────────────────────

/// Analyze cache hit trend across sessions for a project.
/// Sessions should be sorted by time (oldest first).
pub fn analyze_project_trend(summaries: &[SessionSummary]) -> ProjectTrend {
    let points: Vec<SessionTrendPoint> = summaries
        .iter()
        .map(|s| {
            let hit = cache_hit_ratio(&s.total_usage);
            let diag = run_diagnose(s);
            SessionTrendPoint {
                session_id: s.session_id.clone(),
                slug: s.slug.clone(),
                date: s.start_time.map(|t| t.format("%Y-%m-%d %H:%M").to_string()),
                tokens: s.total_usage.total_tokens(),
                cache_hit: hit,
                classification: diag.cache_stability.classification,
            }
        })
        .collect();

    let overall_avg = if points.is_empty() {
        0.0
    } else {
        points.iter().map(|p| p.cache_hit).sum::<f64>() / points.len() as f64
    };

    let recent_count = 5.min(points.len());
    let recent_avg = if recent_count == 0 {
        0.0
    } else {
        points[points.len() - recent_count..]
            .iter()
            .map(|p| p.cache_hit)
            .sum::<f64>()
            / recent_count as f64
    };

    let direction = if points.len() < 3 {
        TrendDirection::Stable
    } else {
        let diff = recent_avg - overall_avg;
        if diff > 0.05 {
            TrendDirection::Improving
        } else if diff < -0.05 {
            TrendDirection::Declining
        } else {
            TrendDirection::Stable
        }
    };

    ProjectTrend {
        points,
        direction,
        recent_avg_cache_hit: recent_avg,
        overall_avg_cache_hit: overall_avg,
    }
}

// ── CLAUDE.md analysis ───────────────────────────────────────────

/// Try to find a CLAUDE.md file at the decoded project path.
pub fn find_claude_md(decoded_project_path: &str) -> Option<PathBuf> {
    let path = PathBuf::from(decoded_project_path).join("CLAUDE.md");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Analyze a CLAUDE.md file for a project.
pub fn analyze_claude_md(decoded_project_path: &str, include_content: bool) -> ClaudeMdAnalysis {
    match find_claude_md(decoded_project_path) {
        None => ClaudeMdAnalysis {
            exists: false,
            path: None,
            size_bytes: 0,
            estimated_tokens: 0,
            oversized: false,
            content: None,
            recommendations: vec![
                "No CLAUDE.md found. Adding one with project structure and key commands improves cache stability.".to_string(),
            ],
        },
        Some(path) => {
            let content_result = std::fs::read_to_string(&path);
            let (size_bytes, content_str) = match &content_result {
                Ok(s) => (s.len() as u64, Some(s.clone())),
                Err(_) => (0, None),
            };
            let estimated_tokens = size_bytes / 4;
            let oversized = estimated_tokens > 8000;

            let mut recommendations = Vec::new();
            if oversized {
                recommendations.push(format!(
                    "CLAUDE.md is ~{} tokens ({}KB). Consider trimming to <8K tokens for better cache efficiency.",
                    estimated_tokens, size_bytes / 1024
                ));
            }

            ClaudeMdAnalysis {
                exists: true,
                path: Some(path),
                size_bytes,
                estimated_tokens,
                oversized,
                content: if include_content { content_str } else { None },
                recommendations,
            }
        }
    }
}

/// Generate project-level recommendations from benchmarks and global stats.
fn generate_project_recommendations(
    benchmarks: &[ProjectBenchmark],
    global_avg_cache_hit: f64,
) -> Vec<String> {
    let mut recs = Vec::new();

    for b in benchmarks {
        if b.avg_cache_hit < global_avg_cache_hit - 0.1 && b.session_count >= 2 {
            recs.push(format!(
                "Project \"{}\" has {:.1}% cache hit vs global {:.1}%. Review session patterns for context churn.",
                b.project,
                b.avg_cache_hit * 100.0,
                global_avg_cache_hit * 100.0
            ));
        }
        if b.bash_loop_count > 3 {
            recs.push(format!(
                "Project \"{}\" had {} bash retry sequences. Add build/test commands to its CLAUDE.md.",
                b.project, b.bash_loop_count
            ));
        }
    }

    if global_avg_cache_hit < 0.7 {
        recs.push(
            "Global cache hit is below 70%. Consider shorter, more focused sessions across all projects."
                .to_string(),
        );
    }

    recs
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

    let diag = run_diagnose_with_entries(summary, Some(entries));

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
    let mut benchmarks: Vec<ProjectBenchmark> = by_project
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
        // Find the matching project key
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

fn bash_retry_to_json(retry: &BashRetry) -> BashRetryJson {
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

fn render_json(
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
        recommendations: diag.recommendations.clone(),
    };

    println!(
        "{}",
        serde_json::to_string_pretty(&json).unwrap_or_default()
    );
}

// ── Project-level text rendering ──────────────────────────────────

pub fn classification_str(c: &CacheClassification) -> &'static str {
    match c {
        CacheClassification::Stable => "stable",
        CacheClassification::Churning => "churning",
        CacheClassification::Degrading => "degrading",
    }
}

fn render_project_text(data: &ProjectDiagnoseData, days: u64) {
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

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}

// ── Project-level JSON rendering ─────────────────────────────────

fn render_project_json(data: &ProjectDiagnoseData, days: u64) {
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

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::types::TokenUsage;

    fn make_turn(
        index: usize,
        input: u64,
        cache_creation: u64,
        cache_read: u64,
        output: u64,
        tools: Vec<&str>,
    ) -> TurnSummary {
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
            bash_commands: Vec::new(),
        }
    }

    fn make_turn_with_bash(index: usize, tools: Vec<&str>, commands: Vec<&str>) -> TurnSummary {
        TurnSummary {
            index,
            timestamp: None,
            usage: TokenUsage {
                input_tokens: 100,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                output_tokens: 50,
                cache_creation: None,
                service_tier: None,
            },
            tools: tools.into_iter().map(String::from).collect(),
            model: None,
            bash_commands: commands.into_iter().map(String::from).collect(),
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
            bash_retries: vec![BashRetry {
                pattern: BashRetryPattern::IdenticalCommand {
                    command: "cargo test".to_string(),
                },
                start_turn: 5,
                length: 4,
            }],
            read_edit_ratio: 8.0,
            exploration_flagged: true,
            subagent_count: 5,
            subagent_flagged: true,
        };

        let same_error = vec![BashRetry {
            pattern: BashRetryPattern::SameError {
                command: "pulumi up".to_string(),
                error_snippet: "no Pulumi.yaml".to_string(),
            },
            start_turn: 10,
            length: 3,
        }];

        let recs = generate_recommendations(&cache, &growth, &tools, Some(&same_error));

        // Should have recommendations for: churning, growth, bash loops, identical retries, same-error, exploration, subagents
        assert_eq!(recs.len(), 7);
        assert!(recs[0].contains("30%"));
        assert!(recs[1].contains("3.0x"));
        assert!(recs[2].contains("Bash"));
        assert!(recs[3].contains("identical command"));
        assert!(recs[4].contains("same-error"));
        assert!(recs[5].contains("Read:Edit"));
        assert!(recs[6].contains("Task"));
    }

    // ── Project-level analysis tests ──────────────────────────────

    fn make_session_summary(
        id: &str,
        project: &str,
        cache_read_pct: f64,
        turns_count: usize,
    ) -> SessionSummary {
        // cache_read_pct is 0.0-1.0 fraction of input that comes from cache reads
        let total_input = 100_000u64;
        let cache_read = (total_input as f64 * cache_read_pct) as u64;
        let input = total_input - cache_read;

        let turns: Vec<TurnSummary> = (0..turns_count)
            .map(|i| {
                let per_turn_input = input / turns_count as u64;
                let per_turn_cache = cache_read / turns_count as u64;
                if i < 3 {
                    make_turn(
                        i,
                        per_turn_input,
                        500,
                        per_turn_cache,
                        100,
                        vec!["Read", "Edit"],
                    )
                } else {
                    make_turn(
                        i,
                        per_turn_input,
                        50,
                        per_turn_cache,
                        100,
                        vec!["Read", "Edit"],
                    )
                }
            })
            .collect();

        use crate::parser::types::TokenUsage;
        let mut total_usage = TokenUsage::default();
        for t in &turns {
            total_usage += t.usage.clone();
        }

        SessionSummary {
            session_id: id.to_string(),
            slug: Some(format!("{}-slug", id)),
            project_path: project.to_string(),
            start_time: Some(chrono::Utc::now()),
            end_time: Some(chrono::Utc::now() + chrono::Duration::minutes(30)),
            total_usage,
            turns,
            ..Default::default()
        }
    }

    #[test]
    fn test_efficiency_score_perfect() {
        // Perfect: high cache, no bash loops, no exploration, stable
        let score = efficiency_score(0.95, 0.0, 0.0, &CacheClassification::Stable);
        assert!(score > 0.9, "expected >0.9, got {}", score);
    }

    #[test]
    fn test_efficiency_score_poor() {
        // Poor: low cache, lots of bash, exploration, churning
        let score = efficiency_score(0.3, 3.0, 1.0, &CacheClassification::Churning);
        assert!(score < 0.3, "expected <0.3, got {}", score);
    }

    #[test]
    fn test_compute_project_benchmark_single_session() {
        let summary = make_session_summary("abc-123", "/foo/bar", 0.85, 10);
        let benchmark = compute_project_benchmark("foo/bar", &[summary]);

        assert_eq!(benchmark.project, "foo/bar");
        assert_eq!(benchmark.session_count, 1);
        assert!(benchmark.avg_tokens_per_session > 0);
        assert!(benchmark.efficiency_score > 0.0);
    }

    #[test]
    fn test_compute_project_benchmark_multiple_sessions() {
        let s1 = make_session_summary("aaa", "/foo/bar", 0.90, 10);
        let s2 = make_session_summary("bbb", "/foo/bar", 0.80, 10);
        let s3 = make_session_summary("ccc", "/foo/bar", 0.85, 10);

        let benchmark = compute_project_benchmark("foo/bar", &[s1, s2, s3]);

        assert_eq!(benchmark.session_count, 3);
        // avg cache hit should be somewhere near 0.85
        assert!(benchmark.avg_cache_hit > 0.5);
    }

    #[test]
    fn test_rank_benchmarks_ordering() {
        let mut benchmarks = vec![
            ProjectBenchmark {
                project: "low".to_string(),
                session_count: 1,
                avg_tokens_per_session: 100_000,
                avg_cache_hit: 0.3,
                dominant_classification: CacheClassification::Churning,
                bash_loop_count: 5,
                bash_retry_count: 0,
                exploration_count: 2,
                efficiency_score: 0.2,
            },
            ProjectBenchmark {
                project: "high".to_string(),
                session_count: 1,
                avg_tokens_per_session: 100_000,
                avg_cache_hit: 0.95,
                dominant_classification: CacheClassification::Stable,
                bash_loop_count: 0,
                bash_retry_count: 0,
                exploration_count: 0,
                efficiency_score: 0.95,
            },
            ProjectBenchmark {
                project: "mid".to_string(),
                session_count: 1,
                avg_tokens_per_session: 100_000,
                avg_cache_hit: 0.7,
                dominant_classification: CacheClassification::Stable,
                bash_loop_count: 1,
                bash_retry_count: 0,
                exploration_count: 0,
                efficiency_score: 0.7,
            },
        ];

        rank_benchmarks(&mut benchmarks);

        assert_eq!(benchmarks[0].project, "high");
        assert_eq!(benchmarks[1].project, "mid");
        assert_eq!(benchmarks[2].project, "low");
    }

    #[test]
    fn test_analyze_project_trend_stable() {
        // All sessions have similar cache hit → Stable
        let summaries: Vec<SessionSummary> = (0..5)
            .map(|i| make_session_summary(&format!("s{}", i), "/foo/bar", 0.85, 10))
            .collect();

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Stable);
        assert_eq!(trend.points.len(), 5);
    }

    #[test]
    fn test_analyze_project_trend_improving() {
        // Early sessions poor, recent sessions good → Improving
        let mut summaries: Vec<SessionSummary> = Vec::new();
        for i in 0..8 {
            let cache = if i < 3 { 0.50 } else { 0.92 };
            summaries.push(make_session_summary(
                &format!("s{}", i),
                "/foo/bar",
                cache,
                10,
            ));
        }

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Improving);
    }

    #[test]
    fn test_analyze_project_trend_declining() {
        // Early sessions good, recent sessions poor → Declining
        let mut summaries: Vec<SessionSummary> = Vec::new();
        for i in 0..8 {
            let cache = if i < 3 { 0.92 } else { 0.50 };
            summaries.push(make_session_summary(
                &format!("s{}", i),
                "/foo/bar",
                cache,
                10,
            ));
        }

        let trend = analyze_project_trend(&summaries);
        assert_eq!(trend.direction, TrendDirection::Declining);
    }

    #[test]
    fn test_analyze_claude_md_missing() {
        // Path that doesn't exist → missing analysis
        let analysis = analyze_claude_md("/nonexistent/path/that/does/not/exist", false);
        assert!(!analysis.exists);
        assert!(analysis.path.is_none());
        assert!(!analysis.recommendations.is_empty());
        assert!(analysis.recommendations[0].contains("No CLAUDE.md"));
    }

    #[test]
    fn test_efficiency_score_boundaries() {
        // Cache hit = 0, all other factors bad
        let score = efficiency_score(0.0, 5.0, 1.0, &CacheClassification::Degrading);
        assert!(score >= 0.0 && score <= 1.0);

        // Cache hit = 1, all other factors perfect
        let score = efficiency_score(1.0, 0.0, 0.0, &CacheClassification::Stable);
        assert!(score >= 0.0 && score <= 1.0);
        assert!((score - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_project_recommendations_low_cache() {
        let benchmarks = vec![ProjectBenchmark {
            project: "bad-project".to_string(),
            session_count: 3,
            avg_tokens_per_session: 100_000,
            avg_cache_hit: 0.5,
            dominant_classification: CacheClassification::Churning,
            bash_loop_count: 0,
            bash_retry_count: 0,
            exploration_count: 0,
            efficiency_score: 0.4,
        }];

        let recs = generate_project_recommendations(&benchmarks, 0.85);
        assert!(!recs.is_empty());
        assert!(recs.iter().any(|r| r.contains("bad-project")));
    }

    // ── Identical command retry tests ─────────────────────────────

    #[test]
    fn test_identical_command_3x_detected() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Read"], vec![]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(3, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(4, vec!["Edit"], vec![]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start_turn, 1);
        assert_eq!(result[0].length, 3);
        if let BashRetryPattern::IdenticalCommand { ref command } = result[0].pattern {
            assert_eq!(command, "cargo test");
        } else {
            panic!("expected IdenticalCommand pattern");
        }
    }

    #[test]
    fn test_identical_command_different_commands_not_triggered() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo build"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo check"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty());
    }

    #[test]
    fn test_identical_command_multi_bash_uses_first() {
        // Multi-bash turns: first command is used for comparison
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash", "Bash"], vec!["cargo test", "echo done"]),
            make_turn_with_bash(1, vec!["Bash", "Bash"], vec!["cargo test", "echo other"]),
            make_turn_with_bash(2, vec!["Bash"], vec!["cargo test"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].length, 3);
    }

    #[test]
    fn test_identical_command_non_bash_breaks_streak() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(1, vec!["Bash"], vec!["cargo test"]),
            make_turn_with_bash(2, vec!["Read"], vec![]), // no bash commands
            make_turn_with_bash(3, vec!["Bash"], vec!["cargo test"]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty()); // no streak >= 3
    }

    #[test]
    fn test_identical_command_empty_bash_commands_ignored() {
        let turns = vec![
            make_turn_with_bash(0, vec!["Bash"], vec![]),
            make_turn_with_bash(1, vec!["Bash"], vec![]),
            make_turn_with_bash(2, vec!["Bash"], vec![]),
        ];

        let result = detect_identical_command_retries(&turns);
        assert!(result.is_empty());
    }

    // ── Error normalization tests ─────────────────────────────────

    #[test]
    fn test_strip_ansi() {
        assert_eq!(strip_ansi("\x1b[31merror\x1b[0m"), "error");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
        assert_eq!(strip_ansi("\x1b[1;31mbold red\x1b[0m"), "bold red");
    }

    #[test]
    fn test_normalize_error_path_shortening() {
        let input = "error in /Users/foo/bar/baz/src/main.rs";
        let normalized = normalize_error(input);
        assert!(normalized.contains(".../src/main.rs"));
        assert!(!normalized.contains("/Users/foo/bar/baz"));
    }

    #[test]
    fn test_normalize_error_line_numbers() {
        let input = "error at file.rs:42: something failed";
        let normalized = normalize_error(input);
        assert!(normalized.contains("file.rs:_:"));
        assert!(!normalized.contains(":42:"));
    }

    #[test]
    fn test_normalize_error_truncation() {
        let long_error = "x".repeat(300);
        let normalized = normalize_error(&long_error);
        assert_eq!(normalized.len(), 200);
    }

    // ── Same-error retry tests ────────────────────────────────────

    fn make_assistant_entry(tool_use_id: &str, command: &str) -> SessionEntry {
        use crate::parser::types::{AssistantEntry, AssistantMessage, CommonFields};
        SessionEntry::Assistant(AssistantEntry {
            common: CommonFields {
                uuid: None,
                session_id: None,
                timestamp: None,
                parent_uuid: None,
                cwd: None,
                version: None,
                git_branch: None,
                slug: None,
                is_sidechain: None,
            },
            message: AssistantMessage {
                model: Some("test-model".to_string()),
                id: None,
                role: Some("assistant".to_string()),
                content: Some(vec![ContentBlock::ToolUse {
                    id: tool_use_id.to_string(),
                    name: "Bash".to_string(),
                    input: serde_json::json!({"command": command}),
                }]),
                stop_reason: None,
                usage: Some(TokenUsage::default()),
            },
            request_id: None,
        })
    }

    fn make_user_entry(tool_use_id: &str, error_content: &str, is_error: bool) -> SessionEntry {
        use crate::parser::types::{CommonFields, UserEntry, UserMessage};
        SessionEntry::User(UserEntry {
            common: CommonFields {
                uuid: None,
                session_id: None,
                timestamp: None,
                parent_uuid: None,
                cwd: None,
                version: None,
                git_branch: None,
                slug: None,
                is_sidechain: None,
            },
            message: UserMessage {
                role: Some("user".to_string()),
                content: Some(serde_json::json!([{
                    "type": "tool_result",
                    "tool_use_id": tool_use_id,
                    "content": error_content,
                    "is_error": is_error
                }])),
            },
            tool_use_result: None,
        })
    }

    #[test]
    fn test_same_error_3x_detected() {
        let entries = vec![
            make_assistant_entry("t1", "pulumi up"),
            make_user_entry("t1", "error: no Pulumi.yaml found", true),
            make_assistant_entry("t2", "pulumi up"),
            make_user_entry("t2", "error: no Pulumi.yaml found", true),
            make_assistant_entry("t3", "pulumi up"),
            make_user_entry("t3", "error: no Pulumi.yaml found", true),
        ];

        let result = detect_same_error_retries(&entries);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].length, 3);
        if let BashRetryPattern::SameError { ref command, .. } = result[0].pattern {
            assert_eq!(command, "pulumi up");
        } else {
            panic!("expected SameError pattern");
        }
    }

    #[test]
    fn test_same_error_different_errors_not_triggered() {
        let entries = vec![
            make_assistant_entry("t1", "cargo test"),
            make_user_entry("t1", "error: cannot find module 'foo'", true),
            make_assistant_entry("t2", "cargo test"),
            make_user_entry("t2", "error: type mismatch in bar", true),
            make_assistant_entry("t3", "cargo test"),
            make_user_entry("t3", "error: unused variable 'x'", true),
        ];

        let result = detect_same_error_retries(&entries);
        assert!(result.is_empty());
    }

    #[test]
    fn test_same_error_non_error_results_ignored() {
        let entries = vec![
            make_assistant_entry("t1", "ls"),
            make_user_entry("t1", "file1.txt\nfile2.txt", false),
            make_assistant_entry("t2", "ls"),
            make_user_entry("t2", "file1.txt\nfile2.txt", false),
            make_assistant_entry("t3", "ls"),
            make_user_entry("t3", "file1.txt\nfile2.txt", false),
        ];

        let result = detect_same_error_retries(&entries);
        assert!(result.is_empty());
    }
}
