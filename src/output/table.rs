use chrono::{DateTime, Duration, Utc};
use comfy_table::{Cell, ContentArrangement, Table};

use crate::commands::timeline::{ConcurrencySlot, TimelineSession};
use crate::cost::CostBreakdown;
use crate::output::{format_cost, format_percent, format_tokens};
use crate::parser::types::SessionSummary;

/// Data for a single row in the sessions list.
pub struct SessionRow<'a> {
    pub summary: &'a SessionSummary,
    pub cost: &'a CostBreakdown,
    pub cache_hit: f64,
}

/// Render the sessions list as a terminal table.
pub fn render_sessions_table(rows: &[SessionRow], show_cost: bool) {
    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);

    if show_cost {
        table.set_header(vec![
            "Session", "Project", "Date", "Model", "Tokens", "Cost", "Cache", "Turns",
        ]);
    } else {
        table.set_header(vec![
            "Session", "Project", "Date", "Model", "Tokens", "Cache", "Turns",
        ]);
    }

    for row in rows {
        let summary = row.summary;
        let date = summary
            .start_time
            .map(|t| t.format("%b %d").to_string())
            .unwrap_or_else(|| "—".to_string());

        let project = shorten_project(&summary.project_path);
        let model = summary.model.as_deref().unwrap_or("unknown").to_string();
        let slug = summary.slug.as_deref().unwrap_or(&summary.session_id[..8]);

        let tokens = format_tokens(summary.total_usage.total_tokens());
        let cache = format_percent(row.cache_hit);
        let turns = summary.turns.len().to_string();

        if show_cost {
            table.add_row(vec![
                Cell::new(slug),
                Cell::new(&project),
                Cell::new(&date),
                Cell::new(shorten_model(&model)),
                Cell::new(&tokens),
                Cell::new(format_cost(row.cost.total())),
                Cell::new(&cache),
                Cell::new(&turns),
            ]);
        } else {
            table.add_row(vec![
                Cell::new(slug),
                Cell::new(&project),
                Cell::new(&date),
                Cell::new(shorten_model(&model)),
                Cell::new(&tokens),
                Cell::new(&cache),
                Cell::new(&turns),
            ]);
        }
    }

    println!("{table}");
}

/// Render a single session's detail view.
pub fn render_session_detail(
    summary: &SessionSummary,
    cost: &CostBreakdown,
    cache_hit: f64,
    show_cost: bool,
) {
    let duration = match (summary.start_time, summary.end_time) {
        (Some(start), Some(end)) => {
            let mins = (end - start).num_minutes();
            format!("{}m", mins)
        }
        _ => "—".to_string(),
    };

    let date_range = match (summary.start_time, summary.end_time) {
        (Some(start), Some(end)) => {
            format!(
                "{} — {} ({})",
                start.format("%b %d, %Y %H:%M"),
                end.format("%H:%M"),
                duration
            )
        }
        (Some(start), None) => start.format("%b %d, %Y %H:%M").to_string(),
        _ => "—".to_string(),
    };

    println!(
        " Session: {}",
        summary.slug.as_deref().unwrap_or(&summary.session_id)
    );
    println!(" Project: {}", summary.project_path);
    println!(" Date:    {}", date_range);
    println!(
        " Model:   {}",
        summary.model.as_deref().unwrap_or("unknown")
    );
    if let Some(branch) = &summary.git_branch {
        println!(" Branch:  {}", branch);
    }
    println!(" Turns:   {}", summary.turns.len());
    println!();

    // Token breakdown
    println!(" ── Token Breakdown ────────────────────────────────");
    let mut breakdown_table = Table::new();
    breakdown_table.set_content_arrangement(ContentArrangement::Dynamic);

    if show_cost {
        breakdown_table.set_header(vec!["Category", "Tokens", "Cost"]);
    } else {
        breakdown_table.set_header(vec!["Category", "Tokens", "% of Total"]);
    }

    let total = summary.total_usage.total_tokens() as f64;

    let rows: Vec<(&str, u64, f64)> = vec![
        ("Input", summary.total_usage.input_tokens, cost.input_cost),
        (
            "Cache creation",
            summary.total_usage.cache_creation_input_tokens,
            cost.cache_creation_cost,
        ),
        (
            "Cache read",
            summary.total_usage.cache_read_input_tokens,
            cost.cache_read_cost,
        ),
        (
            "Output",
            summary.total_usage.output_tokens,
            cost.output_cost,
        ),
    ];

    for (label, tokens, cost_val) in &rows {
        let pct = if total > 0.0 {
            *tokens as f64 / total * 100.0
        } else {
            0.0
        };
        if show_cost {
            breakdown_table.add_row(vec![
                Cell::new(label),
                Cell::new(format_tokens(*tokens)),
                Cell::new(format_cost(*cost_val)),
            ]);
        } else {
            breakdown_table.add_row(vec![
                Cell::new(label),
                Cell::new(format_tokens(*tokens)),
                Cell::new(format!("{:.1}%", pct)),
            ]);
        }
    }

    if show_cost {
        breakdown_table.add_row(vec![
            Cell::new("Total"),
            Cell::new(format_tokens(summary.total_usage.total_tokens())),
            Cell::new(format_cost(cost.total())),
        ]);
    } else {
        breakdown_table.add_row(vec![
            Cell::new("Total"),
            Cell::new(format_tokens(summary.total_usage.total_tokens())),
            Cell::new("100.0%"),
        ]);
    }

    println!("{breakdown_table}");
    println!();

    // Cache efficiency
    println!(" ── Cache Efficiency ───────────────────────────────");
    println!("  Cache hit ratio:    {}", format_percent(cache_hit));
    let cache_read = summary.total_usage.cache_read_input_tokens;
    let cache_write = summary.total_usage.cache_creation_input_tokens;
    if cache_read + cache_write > 0 {
        println!(
            "  Tokens from cache:  {} / {} input",
            format_tokens(cache_read),
            format_tokens(summary.total_usage.input_tokens + cache_write + cache_read)
        );
    }
    println!();

    // Tool usage
    if !summary.tool_calls.is_empty() {
        println!(" ── Tool Usage ─────────────────────────────────────");
        let mut tools: Vec<_> = summary.tool_calls.iter().collect();
        tools.sort_by(|a, b| b.1.cmp(a.1));
        for (tool, count) in tools {
            println!("  {:<18} {} calls", tool, count);
        }
    }
}

/// Render an ASCII Gantt-style timeline of sessions.
pub fn render_timeline(
    sessions: &[TimelineSession],
    concurrency: &[ConcurrencySlot],
    axis_start: DateTime<Utc>,
    axis_end: DateTime<Utc>,
    granularity: Duration,
    show_cost: bool,
) {
    let total_seconds = (axis_end - axis_start).num_seconds().max(1);
    let bar_width: usize = 60;

    // Format the header date
    let date_label = axis_start.format("%b %-d, %Y").to_string();
    println!();
    println!(
        " ── Timeline: {} ──────────────────────────────────────",
        date_label
    );
    println!();

    // Build time axis labels
    let gran_secs = granularity.num_seconds().max(1);
    let num_labels = (total_seconds / gran_secs) as usize + 1;
    let label_format = if granularity.num_hours() >= 24 {
        "%b %d"
    } else {
        "%H:%M"
    };

    // Print time axis header
    let mut axis_line = String::from("         ");
    let mut tick_line = String::from("         ");
    for i in 0..num_labels {
        let t = axis_start + granularity * i as i32;
        let label = t.format(label_format).to_string();
        let pos = ((i as f64 / num_labels.max(1) as f64) * bar_width as f64) as usize;
        // Pad to position
        while axis_line.len() < 9 + pos {
            axis_line.push(' ');
        }
        axis_line.push_str(&label);
        while tick_line.len() < 9 + pos {
            tick_line.push('─');
        }
        tick_line.push('┼');
    }
    println!("{}", axis_line);
    println!("{}", tick_line);

    // Group sessions by project
    let mut project_order: Vec<String> = Vec::new();
    let mut by_project: std::collections::HashMap<String, Vec<&TimelineSession>> =
        std::collections::HashMap::new();
    for s in sessions {
        by_project.entry(s.project.clone()).or_default().push(s);
        if !project_order.contains(&s.project) {
            project_order.push(s.project.clone());
        }
    }

    // Render each project group
    for project in &project_order {
        let proj_sessions = &by_project[project];
        println!(" {}", project);

        for s in proj_sessions {
            // Calculate bar position and width
            let start_offset = (s.start - axis_start).num_seconds().max(0) as f64;
            let end_offset = (s.end - axis_start).num_seconds().max(0) as f64;

            let bar_start = ((start_offset / total_seconds as f64) * bar_width as f64) as usize;
            let bar_end = ((end_offset / total_seconds as f64) * bar_width as f64) as usize;
            let bar_len = (bar_end - bar_start).max(1);

            // Build the bar line
            let mut line = String::from("         ");
            for _ in 0..bar_start {
                line.push(' ');
            }
            line.push('[');
            for _ in 0..bar_len.saturating_sub(2) {
                line.push('█');
            }
            line.push(']');

            // Pad to annotation column
            while line.len() < 9 + bar_width + 2 {
                line.push(' ');
            }

            // Right-side annotation: tokens + cache hit
            let tokens_str = format_tokens(s.tokens);
            let cache_str = format_percent(s.cache_hit);

            if show_cost {
                let cost_str = s.cost.map(format_cost).unwrap_or_default();
                line.push_str(&format!("{:>8}  {}  {}", tokens_str, cache_str, cost_str));
            } else {
                line.push_str(&format!("{:>8}  {}", tokens_str, cache_str));
            }

            println!("{}", line);
        }
    }

    // Print overlap/concurrency row
    println!("{}", tick_line);

    if !concurrency.is_empty() {
        let mut overlap_line = String::from(" Overlap  ");
        let mut burn_line = String::from(" Burn/hr  ");

        for slot in concurrency {
            let pos = {
                let offset = (slot.time - axis_start).num_seconds().max(0) as f64;
                ((offset / total_seconds as f64) * bar_width as f64) as usize
            };
            while overlap_line.len() < 10 + pos {
                overlap_line.push(' ');
            }
            overlap_line.push_str(&format!("{:<6}", slot.count));

            while burn_line.len() < 10 + pos {
                burn_line.push(' ');
            }
            // Tokens per hour estimate
            let gran_hours = granularity.num_minutes().max(1) as f64 / 60.0;
            let burn_per_hr = slot.tokens as f64 / gran_hours;
            burn_line.push_str(&format!("{:<6}", compact_tokens(burn_per_hr as u64)));
        }

        println!("{}", overlap_line);
        println!("{}", burn_line);
    }
}

/// Format a token count compactly (e.g., 42K, 1.2M).
fn compact_tokens(count: u64) -> String {
    if count >= 1_000_000 {
        format!("{:.1}M", count as f64 / 1_000_000.0)
    } else if count >= 1_000 {
        format!("{}K", count / 1_000)
    } else {
        count.to_string()
    }
}

/// Shorten a project path to just the last two components.
pub fn shorten_project(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        path.to_string()
    }
}

/// Shorten a model name for table display.
/// Filters out `<synthetic>` (used by Claude Code for internal entries).
pub fn shorten_model(model: &str) -> &str {
    if model == "<synthetic>" {
        return "unknown";
    }
    if model.len() > 20 {
        if let Some(pos) = model.rfind("-20") {
            return &model[..pos];
        }
    }
    model
}
