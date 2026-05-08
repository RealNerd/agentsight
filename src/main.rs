use agentsight::commands;
use agentsight::config;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agentsight",
    about = "Token attribution and session intelligence for Claude Code",
    version,
    long_version = concat!(
        env!("CARGO_PKG_VERSION"), " (",
        env!("AGENTSIGHT_GIT_HASH"), " ",
        env!("AGENTSIGHT_BUILD_DATE"), ")"
    )
)]
struct Cli {
    /// Output as JSON
    #[arg(long, global = true)]
    json: bool,

    /// Show estimated API cost (for pay-per-token users)
    #[arg(long, global = true)]
    cost: bool,

    /// Path to config file
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Path to Claude Code data directory
    #[arg(long, global = true, default_value_t = default_claude_dir_string())]
    claude_dir: String,

    /// Show parse warnings for malformed JSONL lines
    #[arg(long, short, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List recent sessions with usage summaries
    #[command(after_help = "\
Examples:
  agentsight sessions                       # Last 7 days, sorted by date
  agentsight sessions --days 30 --sort tokens
  agentsight sessions --project myapp --limit 5
  agentsight --json sessions                # Machine-readable output")]
    Sessions {
        /// How many days back to look
        #[arg(long, default_value_t = 7)]
        days: u64,

        /// Filter to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,

        /// Sort by: tokens, date, turns, project, cost
        #[arg(long, default_value = "date")]
        sort: String,

        /// Max sessions to show
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Drill into a single session
    #[command(after_help = "\
Examples:
  agentsight session abc123                 # By UUID prefix
  agentsight session my-project/abc123      # By slug
  agentsight --json session abc123          # Machine-readable output
  agentsight --cost session abc123          # Include cost estimates")]
    Session {
        /// Session slug, UUID prefix, or index
        identifier: String,

        /// Show full turn-by-turn table
        #[arg(long)]
        turns: bool,
    },

    /// Cross-session aggregation and trends
    #[command(after_help = "\
Examples:
  agentsight summary                        # Last 7 days
  agentsight summary --days 30 --project myapp
  agentsight summary --by-model             # Per-model breakdown
  agentsight summary --by-model --group-family")]
    Summary {
        /// Period to summarize in days
        #[arg(long, default_value_t = 7)]
        days: u64,

        /// Filter to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,

        /// Show detailed per-model comparison table
        #[arg(long)]
        by_model: bool,

        /// Merge model versions by family (strip date suffixes)
        #[arg(long)]
        group_family: bool,
    },

    /// Live-watch all active sessions
    #[command(after_help = "\
Examples:
  agentsight watch                          # Watch all active sessions
  agentsight watch --session abc123         # Filter to one session
  agentsight watch --idle-timeout 60        # Exit after 60s idle
  agentsight watch --active-window 30       # Hide sessions idle >30s")]
    Watch {
        /// Filter to sessions matching this prefix
        #[arg(long)]
        session: Option<String>,

        /// Exit after N seconds of inactivity
        #[arg(long, default_value_t = 300)]
        idle_timeout: u64,

        /// Seconds of inactivity before a session is hidden
        #[arg(long, default_value_t = 60)]
        active_window: u64,
    },

    /// Show session timeline with concurrency analysis
    #[command(after_help = "\
Examples:
  agentsight timeline                       # Today's sessions
  agentsight timeline --days 7              # Last week
  agentsight timeline --project myapp       # Filter by project
  agentsight --json timeline                # Machine-readable output")]
    Timeline {
        /// How many days back to look
        #[arg(long, default_value_t = 1)]
        days: u64,

        /// Filter to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,
    },

    /// Diagnose session efficiency and suggest improvements
    #[command(after_help = "\
Examples:
  agentsight diagnose abc123                # Diagnose a single session
  agentsight diagnose                       # Project-level overview
  agentsight diagnose --project myapp --days 30
  agentsight diagnose --project myapp --with-context")]
    Diagnose {
        /// Session slug or UUID prefix (omit for project-level overview)
        identifier: Option<String>,

        /// Filter to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,

        /// How many days back to look
        #[arg(long, default_value_t = 7)]
        days: u64,

        /// Include CLAUDE.md analysis (requires --project)
        #[arg(long)]
        with_context: bool,
    },

    /// Launch web dashboard
    #[command(after_help = "\
Examples:
  agentsight dashboard                      # Launch on default port 3141
  agentsight dashboard --port 8080          # Custom port
  agentsight dashboard --port 0            # Auto-select a free port
  agentsight dashboard --no-open            # Don't auto-open browser
  agentsight dashboard --replace            # Replace existing instance")]
    Dashboard {
        /// Port to serve on (0 = auto-select free port)
        #[arg(long, default_value_t = 3141)]
        port: u16,

        /// Don't open browser automatically
        #[arg(long)]
        no_open: bool,

        /// Replace existing dashboard on this port
        #[arg(long)]
        replace: bool,
    },

    /// Sanitize a session JSONL file for use as a test fixture
    #[command(after_help = "\
Examples:
  agentsight sanitize abc123                # Sanitize to stdout
  agentsight sanitize abc123 -o fixture.jsonl
  agentsight sanitize abc123 --max-lines 50 # Truncate output
  agentsight sanitize ./path/to/session.jsonl")]
    Sanitize {
        /// Session slug, UUID prefix, or path to .jsonl file
        identifier: String,

        /// Output file path (default: stdout)
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Maximum number of lines to output (0 = all)
        #[arg(long, default_value_t = 0)]
        max_lines: usize,
    },

    /// Environment health check and baseline usage report
    #[command(after_help = "\
Examples:
  agentsight health                         # Full health check
  agentsight health --quick                 # Environment audit only
  agentsight health --project myapp         # Filter baseline to project
  agentsight --json health                  # Machine-readable output")]
    Health {
        /// Environment audit only, skip session analysis
        #[arg(long)]
        quick: bool,

        /// Filter baseline to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,
    },

    /// Install AgentSight skills as Claude Code slash commands
    #[command(after_help = "\
Examples:
  agentsight install-skill                  # Install all skills
  agentsight install-skill diagnose         # Install one skill
  agentsight install-skill --list           # List available skills
  agentsight install-skill --force          # Overwrite existing files")]
    InstallSkill {
        /// Skill name to install (default: install all)
        name: Option<String>,

        /// List available skills
        #[arg(long)]
        list: bool,

        /// Overwrite existing skill files
        #[arg(long)]
        force: bool,
    },

    /// Generate shell completions
    #[command(hide = true)]
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Create default config on first run (before loading)
    if cli.config.is_none() {
        config::ensure_config_exists();
    }

    let claude_dir = PathBuf::from(&cli.claude_dir);
    let cfg = config::Config::load(cli.config.as_deref())?;

    // --cost flag overrides config; otherwise use config billing mode
    let show_cost = cli.cost || cfg.billing_mode().show_cost();

    match cli.command {
        Commands::Sessions {
            days,
            project,
            sort,
            limit,
        } => {
            let sort_by = match sort.as_str() {
                "cost" => commands::sessions::SortField::Cost,
                "tokens" => commands::sessions::SortField::Tokens,
                "date" => commands::sessions::SortField::Date,
                "turns" => commands::sessions::SortField::Turns,
                "project" => commands::sessions::SortField::Project,
                _ => commands::sessions::SortField::Date,
            };
            commands::sessions::run(
                &claude_dir,
                &cfg,
                &commands::sessions::SessionsArgs {
                    days,
                    project,
                    sort_by,
                    limit,
                    json: cli.json,
                    show_cost,
                    verbose: cli.verbose,
                },
            )
        }
        Commands::Session {
            identifier,
            turns: _,
        } => commands::session::run(
            &claude_dir,
            &cfg,
            &commands::session::SessionArgs {
                identifier,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
            },
        ),
        Commands::Summary {
            days,
            project,
            by_model,
            group_family,
        } => commands::summary::run(
            &claude_dir,
            &cfg,
            &commands::summary::SummaryArgs {
                days,
                project,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
                by_model,
                group_family,
            },
        ),
        Commands::Watch {
            session,
            idle_timeout,
            active_window,
        } => commands::watch::run(
            &claude_dir,
            &cfg,
            &commands::watch::WatchArgs {
                session,
                idle_timeout,
                active_window,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
            },
        ),
        Commands::Timeline { days, project } => commands::timeline::run(
            &claude_dir,
            &cfg,
            &commands::timeline::TimelineArgs {
                days,
                project,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
            },
        ),
        Commands::Diagnose {
            identifier,
            project,
            days,
            with_context,
        } => commands::diagnose::run(
            &claude_dir,
            &cfg,
            &commands::diagnose::DiagnoseArgs {
                identifier,
                project,
                days,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
                with_context,
            },
        ),
        Commands::Dashboard {
            port,
            no_open,
            replace,
        } => commands::dashboard::run(
            &claude_dir,
            &cfg,
            &commands::dashboard::DashboardArgs {
                port,
                no_open,
                show_cost,
                replace,
            },
        ),
        Commands::Sanitize {
            identifier,
            output,
            max_lines,
        } => commands::sanitize::run(
            &claude_dir,
            &commands::sanitize::SanitizeArgs {
                identifier,
                output,
                max_lines,
                verbose: cli.verbose,
            },
        ),
        Commands::Health { quick, project } => commands::health::run(
            &claude_dir,
            &cfg,
            &commands::health::HealthArgs {
                quick,
                project,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
            },
        ),
        Commands::InstallSkill { name, list, force } => {
            commands::install_skill::run(&commands::install_skill::InstallSkillArgs {
                name,
                list,
                force,
                json: cli.json,
                verbose: cli.verbose,
            })
        }
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            generate(shell, &mut cmd, "agentsight", &mut std::io::stdout());
            Ok(())
        }
    }
}

fn default_claude_dir_string() -> String {
    config::default_claude_dir().to_string_lossy().to_string()
}
