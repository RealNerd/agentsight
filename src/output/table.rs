use comfy_table::{Cell, ContentArrangement, Table};

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
        let model = summary
            .model
            .as_deref()
            .unwrap_or("unknown")
            .to_string();
        let slug = summary
            .slug
            .as_deref()
            .unwrap_or(&summary.session_id[..8]);

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
        ("Output", summary.total_usage.output_tokens, cost.output_cost),
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
            format_tokens(
                summary.total_usage.input_tokens + cache_write + cache_read
            )
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
pub fn shorten_model(model: &str) -> &str {
    if model.len() > 20 {
        if let Some(pos) = model.rfind("-20") {
            return &model[..pos];
        }
    }
    model
}
