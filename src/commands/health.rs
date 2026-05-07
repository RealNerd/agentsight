use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

use crate::config::Config;
use crate::output;
use crate::output::json::ProjectBenchmarkJson;
use crate::output::table::shorten_project;
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

use super::diagnose::{
    classification_str, compute_project_benchmark, rank_benchmarks, ProjectBenchmark,
};

pub struct HealthArgs {
    pub quick: bool,
    pub project: Option<String>,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

// ── Data structures ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum CheckStatus {
    Pass,
    Warn,
    Missing,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OverallGrade {
    Good,
    Fair,
    NeedsWork,
}

#[derive(Debug, Clone)]
pub struct CheckItem {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
    pub recommendation: Option<String>,
}

#[derive(Debug)]
pub struct EnvironmentCheck {
    pub grade: OverallGrade,
    pub items: Vec<CheckItem>,
}

#[derive(Debug)]
pub struct BaselineReport {
    pub session_count: u64,
    pub total_tokens: u64,
    pub project_count: usize,
    pub global_avg_cache_hit: f64,
    pub benchmarks: Vec<ProjectBenchmark>,
    pub top_recommendations: Vec<String>,
}

#[derive(Debug)]
pub struct NextStep {
    pub description: String,
    pub command: Option<String>,
}

#[derive(Debug)]
pub struct HealthReport {
    pub environment: EnvironmentCheck,
    pub baseline: Option<BaselineReport>,
    pub next_steps: Vec<NextStep>,
}

// ── Environment check ────────────────────────────────────────────

/// Probe 6 environment items and return an EnvironmentCheck.
pub fn run_environment_check(claude_dir: &Path) -> EnvironmentCheck {
    let mut items = Vec::new();

    // 1. ~/.claude/ directory
    if claude_dir.exists() {
        items.push(CheckItem {
            name: "~/.claude/ directory".to_string(),
            status: CheckStatus::Pass,
            detail: "Exists".to_string(),
            recommendation: None,
        });
    } else {
        items.push(CheckItem {
            name: "~/.claude/ directory".to_string(),
            status: CheckStatus::Missing,
            detail: "Not found".to_string(),
            recommendation: Some("Install Claude Code to create this directory.".to_string()),
        });
    }

    // 2. ~/.claude/projects/
    let projects_dir = claude_dir.join("projects");
    if projects_dir.exists() {
        // Count session files
        let session_count = count_session_files(&projects_dir);
        if session_count > 0 {
            items.push(CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Pass,
                detail: format!("{} sessions found", session_count),
                recommendation: None,
            });
        } else {
            items.push(CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Warn,
                detail: "Directory exists but no sessions found".to_string(),
                recommendation: Some("Run a Claude Code session to generate data.".to_string()),
            });
        }
    } else {
        items.push(CheckItem {
            name: "~/.claude/projects/".to_string(),
            status: CheckStatus::Missing,
            detail: "Not found".to_string(),
            recommendation: Some("Run a Claude Code session to create this directory.".to_string()),
        });
    }

    // 3. ~/.claude/CLAUDE.md (global)
    let global_claude_md = claude_dir.join("CLAUDE.md");
    items.push(check_claude_md_file(
        &global_claude_md,
        "~/.claude/CLAUDE.md",
    ));

    // 4. Project CLAUDE.md (cwd)
    let cwd = std::env::current_dir().unwrap_or_default();
    let project_claude_md = cwd.join("CLAUDE.md");
    items.push(check_claude_md_file(
        &project_claude_md,
        "Project CLAUDE.md",
    ));

    // 5. ~/.claude/settings.json
    let settings = claude_dir.join("settings.json");
    if settings.exists() {
        items.push(CheckItem {
            name: "~/.claude/settings.json".to_string(),
            status: CheckStatus::Pass,
            detail: "Exists".to_string(),
            recommendation: None,
        });
    } else {
        items.push(CheckItem {
            name: "~/.claude/settings.json".to_string(),
            status: CheckStatus::Missing,
            detail: "Not found".to_string(),
            recommendation: None,
        });
    }

    // 6. ~/.agentsight/config.toml
    let agentsight_config = dirs::home_dir()
        .map(|h| h.join(".agentsight").join("config.toml"))
        .unwrap_or_default();
    if agentsight_config.exists() {
        items.push(CheckItem {
            name: "~/.agentsight/config.toml".to_string(),
            status: CheckStatus::Pass,
            detail: "Exists".to_string(),
            recommendation: None,
        });
    } else {
        items.push(CheckItem {
            name: "~/.agentsight/config.toml".to_string(),
            status: CheckStatus::Missing,
            detail: "Not found".to_string(),
            recommendation: None,
        });
    }

    let grade = compute_grade(&items);

    EnvironmentCheck { grade, items }
}

fn check_claude_md_file(path: &Path, display_name: &str) -> CheckItem {
    if path.exists() {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let estimated_tokens = content.len() as u64 / 4;
                if estimated_tokens > 8000 {
                    CheckItem {
                        name: display_name.to_string(),
                        status: CheckStatus::Warn,
                        detail: format!("~{} tokens (oversized)", estimated_tokens),
                        recommendation: Some(format!(
                            "Consider trimming to <8K tokens for better cache efficiency (currently ~{} tokens).",
                            estimated_tokens
                        )),
                    }
                } else {
                    CheckItem {
                        name: display_name.to_string(),
                        status: CheckStatus::Pass,
                        detail: format!("{} tokens", estimated_tokens),
                        recommendation: None,
                    }
                }
            }
            Err(_) => CheckItem {
                name: display_name.to_string(),
                status: CheckStatus::Warn,
                detail: "Exists but unreadable".to_string(),
                recommendation: None,
            },
        }
    } else {
        CheckItem {
            name: display_name.to_string(),
            status: CheckStatus::Missing,
            detail: "Not found".to_string(),
            recommendation: if display_name.contains("~/.claude") {
                Some("Create a ~/.claude/CLAUDE.md with global rules and preferences.".to_string())
            } else {
                None
            },
        }
    }
}

fn count_session_files(projects_dir: &Path) -> u64 {
    let mut count = 0u64;
    if let Ok(project_entries) = std::fs::read_dir(projects_dir) {
        for entry in project_entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            if let Ok(files) = std::fs::read_dir(entry.path()) {
                for file in files.flatten() {
                    if file.path().extension().and_then(|e| e.to_str()) == Some("jsonl") {
                        count += 1;
                    }
                }
            }
        }
    }
    count
}

/// Compute the overall grade from check items.
/// - Good: ~/.claude/ exists, projects/ has sessions, no warnings
/// - Fair: Core items exist but have warnings (oversized CLAUDE.md, empty projects)
/// - NeedsWork: ~/.claude/ or projects/ missing
pub fn compute_grade(items: &[CheckItem]) -> OverallGrade {
    let claude_dir = items
        .iter()
        .find(|i| i.name.contains("~/.claude/ directory"));
    let projects = items
        .iter()
        .find(|i| i.name.contains("~/.claude/projects/"));

    // NeedsWork if core dirs are missing
    if let Some(item) = claude_dir {
        if item.status == CheckStatus::Missing {
            return OverallGrade::NeedsWork;
        }
    }
    if let Some(item) = projects {
        if item.status == CheckStatus::Missing {
            return OverallGrade::NeedsWork;
        }
    }

    // Fair if any item has a warning
    let has_warnings = items.iter().any(|i| i.status == CheckStatus::Warn);
    if has_warnings {
        return OverallGrade::Fair;
    }

    OverallGrade::Good
}

// ── Baseline report ──────────────────────────────────────────────

/// Compute the baseline report by scanning all sessions.
pub fn compute_baseline(
    claude_dir: &Path,
    project_filter: Option<&str>,
    verbose: bool,
) -> Result<BaselineReport> {
    let session_files = session_index::discover_sessions(claude_dir)?;

    // Filter by project if specified
    let filtered: Vec<&session_index::SessionFile> = if let Some(filter) = project_filter {
        session_files
            .iter()
            .filter(|sf| decode_project_path(&sf.project_dir_name).contains(filter))
            .collect()
    } else {
        session_files.iter().collect()
    };

    // Parse all sessions and group by project
    let mut by_project: HashMap<String, Vec<crate::parser::types::SessionSummary>> = HashMap::new();
    let mut total_tokens: u64 = 0;

    for sf in &filtered {
        let decoded = decode_project_path(&sf.project_dir_name);
        let entries = match reader::parse_session_file(&sf.path, verbose) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let summary = reader::summarize_session(&entries, sf.session_id.clone(), decoded.clone());
        total_tokens += summary.total_usage.total_tokens();
        let short = shorten_project(&decoded);
        by_project.entry(short).or_default().push(summary);
    }

    // Compute benchmarks
    let mut benchmarks: Vec<ProjectBenchmark> = by_project
        .iter()
        .map(|(project, summaries)| compute_project_benchmark(project, summaries))
        .collect();
    rank_benchmarks(&mut benchmarks);

    let session_count = filtered.len() as u64;
    let project_count = benchmarks.len();

    // Global average cache hit (weighted by session count)
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

    // Generate top recommendations
    let top_recommendations = generate_baseline_recommendations(&benchmarks, global_avg_cache_hit);

    Ok(BaselineReport {
        session_count,
        total_tokens,
        project_count,
        global_avg_cache_hit,
        benchmarks,
        top_recommendations,
    })
}

fn generate_baseline_recommendations(
    benchmarks: &[ProjectBenchmark],
    global_avg_cache_hit: f64,
) -> Vec<String> {
    let mut recs = Vec::new();

    // Flag projects significantly below global average
    for b in benchmarks {
        if b.avg_cache_hit < global_avg_cache_hit - 0.1 && b.session_count >= 2 {
            recs.push(format!(
                "Project \"{}\" has {:.1}% cache hit vs global {:.1}%.",
                b.project,
                b.avg_cache_hit * 100.0,
                global_avg_cache_hit * 100.0
            ));
        }
    }

    // Global cache hit below 80%
    if global_avg_cache_hit < 0.8 {
        recs.push("Global cache hit is below 80%. Shorter, focused sessions help.".to_string());
    }

    // Cap at 3
    recs.truncate(3);
    recs
}

// ── Next steps generation ────────────────────────────────────────

/// Generate next steps from environment check and optional baseline.
pub fn generate_next_steps(
    env: &EnvironmentCheck,
    baseline: Option<&BaselineReport>,
) -> Vec<NextStep> {
    let mut steps = Vec::new();

    // Check if CC is installed
    let claude_dir_missing = env
        .items
        .iter()
        .any(|i| i.name.contains("~/.claude/ directory") && i.status == CheckStatus::Missing);

    if claude_dir_missing {
        steps.push(NextStep {
            description: "Install Claude Code to get started.".to_string(),
            command: None,
        });
        return steps;
    }

    // Check for no sessions
    let no_sessions = env.items.iter().any(|i| {
        i.name.contains("~/.claude/projects/")
            && matches!(i.status, CheckStatus::Missing | CheckStatus::Warn)
    });

    if no_sessions {
        steps.push(NextStep {
            description: "Run a Claude Code session first to generate usage data.".to_string(),
            command: None,
        });
    }

    // Missing global CLAUDE.md
    let global_md_missing = env
        .items
        .iter()
        .any(|i| i.name == "~/.claude/CLAUDE.md" && i.status == CheckStatus::Missing);

    if global_md_missing {
        steps.push(NextStep {
            description: "Create a ~/.claude/CLAUDE.md with global rules and preferences."
                .to_string(),
            command: None,
        });
    }

    // Oversized CLAUDE.md
    let md_oversized = env
        .items
        .iter()
        .any(|i| i.name.contains("CLAUDE.md") && i.status == CheckStatus::Warn);

    if md_oversized {
        steps.push(NextStep {
            description:
                "Trim oversized CLAUDE.md files to <8K tokens for better cache efficiency."
                    .to_string(),
            command: None,
        });
    }

    // Baseline-driven recommendations
    if let Some(baseline) = baseline {
        if baseline.session_count > 0 {
            // Find the lowest-efficiency project with enough sessions
            if let Some(worst) = baseline
                .benchmarks
                .iter()
                .rev()
                .find(|b| b.session_count >= 2)
            {
                steps.push(NextStep {
                    description: format!(
                        "Drill into your least efficient project (\"{}\"):",
                        worst.project
                    ),
                    command: Some(format!(
                        "agentsight diagnose --project {}",
                        worst.project.rsplit('/').next().unwrap_or(&worst.project)
                    )),
                });
            }
        }
    }

    // Always suggest summary and watch
    steps.push(NextStep {
        description: "See a full usage summary:".to_string(),
        command: Some("agentsight summary --days 30".to_string()),
    });

    steps.push(NextStep {
        description: "Monitor your next session in real time:".to_string(),
        command: Some("agentsight watch".to_string()),
    });

    steps
}

// ── CLI entry point ──────────────────────────────────────────────

pub fn run(claude_dir: &Path, _config: &Config, args: &HealthArgs) -> Result<()> {
    let environment = run_environment_check(claude_dir);

    let baseline = if args.quick {
        None
    } else {
        // Only attempt baseline if projects dir exists
        let projects_dir = claude_dir.join("projects");
        if projects_dir.exists() {
            match compute_baseline(claude_dir, args.project.as_deref(), args.verbose) {
                Ok(b) => Some(b),
                Err(e) => {
                    if args.verbose {
                        eprintln!("warn: baseline computation failed: {}", e);
                    }
                    None
                }
            }
        } else {
            None
        }
    };

    let next_steps = generate_next_steps(&environment, baseline.as_ref());

    let report = HealthReport {
        environment,
        baseline,
        next_steps,
    };

    if args.json {
        render_json(&report);
    } else {
        render_text(&report, args.quick);
    }

    Ok(())
}

// ── Text rendering ───────────────────────────────────────────────

fn status_marker(status: &CheckStatus) -> &'static str {
    match status {
        CheckStatus::Pass => "[OK]",
        CheckStatus::Warn => "[!!]",
        CheckStatus::Missing => "[  ]",
    }
}

fn grade_label(grade: &OverallGrade) -> &'static str {
    match grade {
        OverallGrade::Good => "Good",
        OverallGrade::Fair => "Fair",
        OverallGrade::NeedsWork => "Needs Work",
    }
}

fn render_text(report: &HealthReport, quick: bool) {
    println!();
    println!(" ── AgentSight: Environment Check ────────────────────────");
    println!();

    for item in &report.environment.items {
        println!(
            "  {}  {} ({})",
            status_marker(&item.status),
            item.name,
            item.detail
        );
    }

    let attention_count = report
        .environment
        .items
        .iter()
        .filter(|i| matches!(i.status, CheckStatus::Warn | CheckStatus::Missing))
        .count();

    println!();
    if attention_count == 0 {
        println!(
            "  Overall: {} — all checks passed",
            grade_label(&report.environment.grade)
        );
    } else {
        println!(
            "  Overall: {} — {} item{} need{} attention",
            grade_label(&report.environment.grade),
            attention_count,
            if attention_count > 1 { "s" } else { "" },
            if attention_count == 1 { "s" } else { "" },
        );
    }

    if quick {
        println!();
        return;
    }

    // Baseline report
    if let Some(ref baseline) = report.baseline {
        println!();
        println!(" ── Baseline Report ──────────────────────────────────────");
        println!();
        println!(
            "  {} sessions across {} projects ({} tokens)",
            baseline.session_count,
            baseline.project_count,
            output::format_tokens(baseline.total_tokens)
        );
        println!(
            "  Global cache hit: {}",
            output::format_percent(baseline.global_avg_cache_hit)
        );

        if !baseline.benchmarks.is_empty() {
            println!();
            println!(
                "  {:<3} {:<30} {:>8} {:>12} {:>9} {:>6}",
                "#", "Project", "Sessions", "Tok/Sess", "Cache Hit", "Score"
            );
            for (i, b) in baseline.benchmarks.iter().enumerate() {
                println!(
                    "  {:<3} {:<30} {:>8} {:>12} {:>9} {:>6}",
                    i + 1,
                    truncate_str(&b.project, 30),
                    b.session_count,
                    output::format_tokens(b.avg_tokens_per_session),
                    output::format_percent(b.avg_cache_hit),
                    format!("{:.2}", b.efficiency_score),
                );
            }
        }

        if !baseline.top_recommendations.is_empty() {
            println!();
            println!(" ── Top Recommendations ──────────────────────────────────");
            println!();
            for (i, rec) in baseline.top_recommendations.iter().enumerate() {
                println!("  {}. {}", i + 1, rec);
            }
        }
    }

    // Next steps
    if !report.next_steps.is_empty() {
        println!();
        println!(" ── Next Steps ───────────────────────────────────────────");
        println!();
        for (i, step) in report.next_steps.iter().enumerate() {
            println!("  {}. {}", i + 1, step.description);
            if let Some(ref cmd) = step.command {
                println!("       {}", cmd);
            }
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

// ── JSON rendering ───────────────────────────────────────────────

fn render_json(report: &HealthReport) {
    use crate::output::json::{
        BaselineReportJson, CheckItemJson, EnvironmentCheckJson, HealthJson, NextStepJson,
    };

    let environment = EnvironmentCheckJson {
        grade: match report.environment.grade {
            OverallGrade::Good => "good".to_string(),
            OverallGrade::Fair => "fair".to_string(),
            OverallGrade::NeedsWork => "needs_work".to_string(),
        },
        items: report
            .environment
            .items
            .iter()
            .map(|i| CheckItemJson {
                name: i.name.clone(),
                status: match i.status {
                    CheckStatus::Pass => "pass".to_string(),
                    CheckStatus::Warn => "warn".to_string(),
                    CheckStatus::Missing => "missing".to_string(),
                },
                detail: i.detail.clone(),
                recommendation: i.recommendation.clone(),
            })
            .collect(),
    };

    let baseline = report.baseline.as_ref().map(|b| BaselineReportJson {
        session_count: b.session_count,
        total_tokens: b.total_tokens,
        project_count: b.project_count,
        global_avg_cache_hit: b.global_avg_cache_hit,
        benchmarks: b
            .benchmarks
            .iter()
            .map(|bm| ProjectBenchmarkJson {
                project: bm.project.clone(),
                session_count: bm.session_count,
                avg_tokens_per_session: bm.avg_tokens_per_session,
                avg_cache_hit: bm.avg_cache_hit,
                dominant_classification: classification_str(&bm.dominant_classification)
                    .to_string(),
                bash_loop_count: bm.bash_loop_count,
                bash_retry_count: bm.bash_retry_count,
                exploration_count: bm.exploration_count,
                efficiency_score: bm.efficiency_score,
            })
            .collect(),
        top_recommendations: b.top_recommendations.clone(),
    });

    let next_steps: Vec<NextStepJson> = report
        .next_steps
        .iter()
        .map(|s| NextStepJson {
            description: s.description.clone(),
            command: s.command.clone(),
        })
        .collect();

    let json = HealthJson {
        environment,
        baseline,
        next_steps,
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

    #[test]
    fn test_compute_grade_good() {
        let items = vec![
            CheckItem {
                name: "~/.claude/ directory".to_string(),
                status: CheckStatus::Pass,
                detail: "Exists".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Pass,
                detail: "42 sessions".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/CLAUDE.md".to_string(),
                status: CheckStatus::Pass,
                detail: "500 tokens".to_string(),
                recommendation: None,
            },
        ];
        assert_eq!(compute_grade(&items), OverallGrade::Good);
    }

    #[test]
    fn test_compute_grade_fair_with_warning() {
        let items = vec![
            CheckItem {
                name: "~/.claude/ directory".to_string(),
                status: CheckStatus::Pass,
                detail: "Exists".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Pass,
                detail: "42 sessions".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/CLAUDE.md".to_string(),
                status: CheckStatus::Warn,
                detail: "Oversized".to_string(),
                recommendation: None,
            },
        ];
        assert_eq!(compute_grade(&items), OverallGrade::Fair);
    }

    #[test]
    fn test_compute_grade_needs_work_claude_dir_missing() {
        let items = vec![
            CheckItem {
                name: "~/.claude/ directory".to_string(),
                status: CheckStatus::Missing,
                detail: "Not found".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Missing,
                detail: "Not found".to_string(),
                recommendation: None,
            },
        ];
        assert_eq!(compute_grade(&items), OverallGrade::NeedsWork);
    }

    #[test]
    fn test_compute_grade_needs_work_projects_missing() {
        let items = vec![
            CheckItem {
                name: "~/.claude/ directory".to_string(),
                status: CheckStatus::Pass,
                detail: "Exists".to_string(),
                recommendation: None,
            },
            CheckItem {
                name: "~/.claude/projects/".to_string(),
                status: CheckStatus::Missing,
                detail: "Not found".to_string(),
                recommendation: None,
            },
        ];
        assert_eq!(compute_grade(&items), OverallGrade::NeedsWork);
    }

    #[test]
    fn test_generate_next_steps_no_cc() {
        let env = EnvironmentCheck {
            grade: OverallGrade::NeedsWork,
            items: vec![CheckItem {
                name: "~/.claude/ directory".to_string(),
                status: CheckStatus::Missing,
                detail: "Not found".to_string(),
                recommendation: None,
            }],
        };
        let steps = generate_next_steps(&env, None);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].description.contains("Install Claude Code"));
    }

    #[test]
    fn test_generate_next_steps_no_sessions() {
        let env = EnvironmentCheck {
            grade: OverallGrade::Fair,
            items: vec![
                CheckItem {
                    name: "~/.claude/ directory".to_string(),
                    status: CheckStatus::Pass,
                    detail: "Exists".to_string(),
                    recommendation: None,
                },
                CheckItem {
                    name: "~/.claude/projects/".to_string(),
                    status: CheckStatus::Warn,
                    detail: "Empty".to_string(),
                    recommendation: None,
                },
            ],
        };
        let steps = generate_next_steps(&env, None);
        assert!(steps
            .iter()
            .any(|s| s.description.contains("Run a Claude Code session")));
    }

    #[test]
    fn test_generate_next_steps_missing_global_md() {
        let env = EnvironmentCheck {
            grade: OverallGrade::Fair,
            items: vec![
                CheckItem {
                    name: "~/.claude/ directory".to_string(),
                    status: CheckStatus::Pass,
                    detail: "Exists".to_string(),
                    recommendation: None,
                },
                CheckItem {
                    name: "~/.claude/projects/".to_string(),
                    status: CheckStatus::Pass,
                    detail: "42 sessions".to_string(),
                    recommendation: None,
                },
                CheckItem {
                    name: "~/.claude/CLAUDE.md".to_string(),
                    status: CheckStatus::Missing,
                    detail: "Not found".to_string(),
                    recommendation: None,
                },
            ],
        };
        let steps = generate_next_steps(&env, None);
        assert!(steps.iter().any(|s| s.description.contains("CLAUDE.md")));
        // Should always have summary and watch
        assert!(steps
            .iter()
            .any(|s| s.command.as_deref() == Some("agentsight summary --days 30")));
        assert!(steps
            .iter()
            .any(|s| s.command.as_deref() == Some("agentsight watch")));
    }

    #[test]
    fn test_generate_next_steps_healthy_with_baseline() {
        let env = EnvironmentCheck {
            grade: OverallGrade::Good,
            items: vec![
                CheckItem {
                    name: "~/.claude/ directory".to_string(),
                    status: CheckStatus::Pass,
                    detail: "Exists".to_string(),
                    recommendation: None,
                },
                CheckItem {
                    name: "~/.claude/projects/".to_string(),
                    status: CheckStatus::Pass,
                    detail: "42 sessions".to_string(),
                    recommendation: None,
                },
                CheckItem {
                    name: "~/.claude/CLAUDE.md".to_string(),
                    status: CheckStatus::Pass,
                    detail: "500 tokens".to_string(),
                    recommendation: None,
                },
            ],
        };

        use super::super::diagnose::{CacheClassification, ProjectBenchmark};
        let baseline = BaselineReport {
            session_count: 42,
            total_tokens: 847_291,
            project_count: 3,
            global_avg_cache_hit: 0.78,
            benchmarks: vec![
                ProjectBenchmark {
                    project: "personal/agentsight".to_string(),
                    session_count: 12,
                    avg_tokens_per_session: 32_451,
                    avg_cache_hit: 0.85,
                    dominant_classification: CacheClassification::Stable,
                    bash_loop_count: 0,
                    bash_retry_count: 0,
                    exploration_count: 0,
                    efficiency_score: 0.89,
                },
                ProjectBenchmark {
                    project: "work/backend".to_string(),
                    session_count: 18,
                    avg_tokens_per_session: 45_102,
                    avg_cache_hit: 0.74,
                    dominant_classification: CacheClassification::Stable,
                    bash_loop_count: 1,
                    bash_retry_count: 0,
                    exploration_count: 0,
                    efficiency_score: 0.72,
                },
            ],
            top_recommendations: vec!["Global cache hit is below 80%.".to_string()],
        };

        let steps = generate_next_steps(&env, Some(&baseline));
        // Should suggest drilling into least efficient project
        assert!(steps.iter().any(|s| s.description.contains("work/backend")));
        // Should always have summary and watch
        assert!(steps
            .iter()
            .any(|s| s.command.as_deref() == Some("agentsight summary --days 30")));
    }

    #[test]
    fn test_status_marker() {
        assert_eq!(status_marker(&CheckStatus::Pass), "[OK]");
        assert_eq!(status_marker(&CheckStatus::Warn), "[!!]");
        assert_eq!(status_marker(&CheckStatus::Missing), "[  ]");
    }
}
