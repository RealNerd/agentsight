use super::types::{
    BashRetry, CacheClassification, CacheStability, ContextGrowth, ProjectBenchmark, ToolPatterns,
};

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

/// Generate project-level recommendations from benchmarks and global stats.
pub(crate) fn generate_project_recommendations(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::diagnose::types::{BashLoop, BashRetryPattern};

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
}
