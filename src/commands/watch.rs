use anyhow::Result;
use std::path::Path;

use crate::config::Config;
use crate::cost::calculator::cache_hit_ratio;
use crate::cost::calculate_usage_cost;
use crate::output::{format_cost, format_percent, format_tokens};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;

pub struct WatchArgs {
    pub session: Option<String>,
    pub idle_timeout: u64,
    pub json: bool,
    pub show_cost: bool,
}

pub fn run(claude_dir: &Path, config: &Config, args: &WatchArgs) -> Result<()> {
    let mut session_files = session_index::discover_sessions(claude_dir)?;

    if session_files.is_empty() {
        anyhow::bail!("No session files found");
    }

    // Sort by file modification time (most recent first)
    session_files.sort_by(|a, b| {
        let a_mod = std::fs::metadata(&a.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let b_mod = std::fs::metadata(&b.path)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        b_mod.cmp(&a_mod)
    });

    let target = if let Some(ref id) = args.session {
        session_files
            .iter()
            .find(|sf| sf.session_id.starts_with(id.as_str()))
            .ok_or_else(|| anyhow::anyhow!("No session found matching '{}'", id))?
    } else {
        &session_files[0]
    };

    let project_path = decode_project_path(&target.project_dir_name);
    let watch_path = target.path.clone();
    let session_id = target.session_id.clone();

    println!(" Watching: {} ({})", session_id, project_path);
    println!(" Press Ctrl+C to stop\n");

    let mut last_size = 0u64;
    let mut idle_count = 0u64;

    loop {
        let meta = std::fs::metadata(&watch_path)?;
        let current_size = meta.len();

        if current_size != last_size {
            last_size = current_size;
            idle_count = 0;

            let entries = reader::parse_session_file(&watch_path)?;
            let summary =
                reader::summarize_session(&entries, session_id.clone(), project_path.clone());

            let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
            let pricing = config
                .pricing_for_model(model_name)
                .cloned()
                .unwrap_or_else(|| crate::config::ModelPricing {
                    input_per_million: 5.0,
                    output_per_million: 25.0,
                    cache_creation_per_million: 6.25,
                    cache_read_per_million: 0.5,
                });

            let cost = calculate_usage_cost(&summary.total_usage, &pricing);
            let hit = cache_hit_ratio(&summary.total_usage);

            if args.json {
                crate::output::json::print_session_json(&summary, &cost, hit, args.show_cost);
            } else {
                print!("\r\x1b[K");
                if args.show_cost {
                    print!(
                        " Tokens: {} | Cost: {} | Turns: {} | Cache: {} | {}",
                        format_tokens(summary.total_usage.total_tokens()),
                        format_cost(cost.total()),
                        summary.turns.len(),
                        format_percent(hit),
                        model_name,
                    );
                } else {
                    print!(
                        " Tokens: {} | Turns: {} | Cache: {} | {}",
                        format_tokens(summary.total_usage.total_tokens()),
                        summary.turns.len(),
                        format_percent(hit),
                        model_name,
                    );
                }
                use std::io::Write;
                std::io::stdout().flush()?;
            }
        } else {
            idle_count += 1;
            if idle_count >= args.idle_timeout {
                println!("\n\n Idle timeout reached ({}s). Exiting.", args.idle_timeout);
                break;
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    Ok(())
}
