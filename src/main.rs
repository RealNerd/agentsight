mod commands;
mod config;
mod cost;
mod output;
mod parser;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "agentsight",
    about = "Token attribution and session intelligence for Claude Code",
    version
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
    Session {
        /// Session slug, UUID prefix, or index
        identifier: String,

        /// Show full turn-by-turn table
        #[arg(long)]
        turns: bool,
    },

    /// Cross-session aggregation and trends
    Summary {
        /// Period to summarize in days
        #[arg(long, default_value_t = 7)]
        days: u64,

        /// Filter to a specific project (substring match)
        #[arg(long)]
        project: Option<String>,
    },

    /// Live-watch all active sessions
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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

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
        Commands::Summary { days, project } => commands::summary::run(
            &claude_dir,
            &cfg,
            &commands::summary::SummaryArgs {
                days,
                project,
                json: cli.json,
                show_cost,
                verbose: cli.verbose,
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
    }
}

fn default_claude_dir_string() -> String {
    config::default_claude_dir().to_string_lossy().to_string()
}
