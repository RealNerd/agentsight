use std::path::PathBuf;

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
    /// Verdict on whether this session should be `/clear`ed, and why.
    pub clear_advice: super::clear_advisor::ClearAdvice,
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
