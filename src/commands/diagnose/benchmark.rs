use std::collections::HashMap;

use crate::cost::calculator::cache_hit_ratio;
use crate::parser::types::SessionSummary;

use super::types::{CacheClassification, ProjectBenchmark};

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
        let diag = super::run_diagnose(summary);
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

/// Weighted composite efficiency score (0.0-1.0).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::test_helpers::make_session_summary;

    #[test]
    fn test_efficiency_score_perfect() {
        let score = efficiency_score(0.95, 0.0, 0.0, &CacheClassification::Stable);
        assert!(score > 0.9, "expected >0.9, got {}", score);
    }

    #[test]
    fn test_efficiency_score_poor() {
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
    fn test_efficiency_score_boundaries() {
        let score = efficiency_score(0.0, 5.0, 1.0, &CacheClassification::Degrading);
        assert!((0.0..=1.0).contains(&score));

        let score = efficiency_score(1.0, 0.0, 0.0, &CacheClassification::Stable);
        assert!((0.0..=1.0).contains(&score));
        assert!((score - 1.0).abs() < 0.01);
    }
}
