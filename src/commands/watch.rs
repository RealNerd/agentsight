use anyhow::Result;
use comfy_table::{Cell, ContentArrangement, Table};
use crossterm::{cursor, execute, terminal};
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::config::Config;
use crate::cost::calculate_usage_cost;
use crate::cost::calculator::cache_hit_ratio;
use crate::output::table::{shorten_model, shorten_project};
use crate::output::{format_cost, format_percent, format_tokens};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::SessionSummary;

pub struct WatchArgs {
    pub session: Option<String>,
    pub idle_timeout: u64,
    pub active_window: u64,
    pub json: bool,
    pub show_cost: bool,
    pub verbose: bool,
}

/// Per-file tracking state for a watched session.
struct WatchedSession {
    session_id: String,
    project_path: String,
    last_size: u64,
    last_modified: SystemTime,
    /// Set to true once the file size changes from its value at discovery time.
    ever_changed: bool,
    /// Tick when this session was first discovered (skip polling on this tick).
    discovered_tick: u64,
    summary: Option<SessionSummary>,
    cost: Option<crate::cost::CostBreakdown>,
    cache_hit: f64,
}

/// Data needed to render one row in the watch table.
pub struct WatchRow {
    pub project: String,
    pub session_id: String,
    pub slug: Option<String>,
    pub tokens: u64,
    pub turns: usize,
    pub cache_hit: f64,
    pub model: Option<String>,
    pub cost: Option<f64>,
    /// "active" or "idle 2m 15s" etc.
    pub status: String,
}

pub fn run(claude_dir: &Path, config: &Config, args: &WatchArgs) -> Result<()> {
    let mut tracked: HashMap<PathBuf, WatchedSession> = HashMap::new();
    let mut prev_lines: u16 = 0;
    let mut global_idle: u64 = 0;
    let mut tick: u64 = 0;
    let active_window = Duration::from_secs(args.active_window);

    loop {
        // Re-discover sessions every 5 ticks (or on first tick)
        if tick.is_multiple_of(5) {
            if let Ok(session_files) = session_index::discover_sessions(claude_dir) {
                let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

                for sf in &session_files {
                    // Apply --session filter
                    if let Some(ref filter) = args.session {
                        if !sf.session_id.starts_with(filter.as_str()) {
                            continue;
                        }
                    }

                    seen.insert(sf.path.clone());

                    if !tracked.contains_key(&sf.path) {
                        let project_path = decode_project_path(&sf.project_dir_name);
                        let meta = std::fs::metadata(&sf.path);
                        let current_size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                        let last_modified = meta
                            .and_then(|m| m.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH);

                        tracked.insert(
                            sf.path.clone(),
                            WatchedSession {
                                session_id: sf.session_id.clone(),
                                project_path,
                                last_size: current_size,
                                last_modified,
                                ever_changed: false,
                                discovered_tick: tick,
                                summary: None,
                                cost: None,
                                cache_hit: 0.0,
                            },
                        );
                    }
                }

                // Remove sessions whose files no longer exist
                tracked.retain(|path, _| seen.contains(path));
            }
        }

        let now = SystemTime::now();
        let mut any_changed = false;

        // Check each tracked file for changes (skip files just discovered this tick)
        for (path, ws) in tracked.iter_mut() {
            if ws.discovered_tick == tick {
                continue;
            }

            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            let current_size = meta.len();

            if current_size != ws.last_size {
                ws.last_size = current_size;
                ws.ever_changed = true;
                ws.last_modified = SystemTime::now();
                any_changed = true;

                // Re-parse this session
                if let Ok(entries) = reader::parse_session_file(path, args.verbose) {
                    let summary = reader::summarize_session(
                        &entries,
                        ws.session_id.clone(),
                        ws.project_path.clone(),
                    );

                    let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
                    let pricing = config
                        .pricing_for_model(model_name)
                        .cloned()
                        .unwrap_or_default();

                    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
                    let hit = cache_hit_ratio(&summary.total_usage);

                    ws.cost = Some(cost);
                    ws.cache_hit = hit;
                    ws.summary = Some(summary);
                }
            }
        }

        // Build rows for all sessions that have changed since watch started
        let rows = build_rows(&tracked, now, active_window, args.show_cost);

        if args.json {
            // NDJSON mode: emit a snapshot only when something changed
            if any_changed {
                let items: Vec<_> = tracked
                    .values()
                    .filter(|ws| ws.ever_changed && ws.summary.is_some())
                    .filter_map(|ws| {
                        let s = ws.summary.as_ref()?;
                        let c = ws.cost.clone().unwrap_or_default();
                        Some((s.clone_for_json(), c, ws.cache_hit))
                    })
                    .collect();

                crate::output::json::print_watch_snapshot_json(&items, args.show_cost);
            }
        } else if !rows.is_empty() {
            // Terminal table mode: render in place (skip when no sessions to show)
            render_table_inplace(&rows, args.show_cost, &mut prev_lines)?;
        }

        // Global idle detection
        if any_changed {
            global_idle = 0;
        } else {
            global_idle += 1;
            if global_idle >= args.idle_timeout {
                if !args.json {
                    println!("\n Idle timeout reached ({}s). Exiting.", args.idle_timeout);
                }
                break;
            }
        }

        tick += 1;
        std::thread::sleep(Duration::from_secs(1));
    }

    Ok(())
}

/// Build rows for all sessions that changed since watch started, sorted by total tokens descending.
fn build_rows(
    tracked: &HashMap<PathBuf, WatchedSession>,
    now: SystemTime,
    active_threshold: Duration,
    show_cost: bool,
) -> Vec<WatchRow> {
    let mut rows: Vec<WatchRow> = tracked
        .values()
        .filter(|ws| ws.ever_changed && ws.summary.is_some())
        .map(|ws| {
            let summary = ws.summary.as_ref().unwrap();
            let idle_secs = now
                .duration_since(ws.last_modified)
                .unwrap_or(Duration::ZERO)
                .as_secs();

            let status = if idle_secs <= active_threshold.as_secs() {
                "active".to_string()
            } else {
                format_idle_duration(idle_secs)
            };

            WatchRow {
                project: ws.project_path.clone(),
                session_id: ws.session_id.clone(),
                slug: summary.slug.clone(),
                tokens: summary.total_usage.total_tokens(),
                turns: summary.turns.len(),
                cache_hit: ws.cache_hit,
                model: summary.model.clone(),
                cost: if show_cost {
                    ws.cost.as_ref().map(|c| c.total())
                } else {
                    None
                },
                status,
            }
        })
        .collect();

    rows.sort_by_key(|r| std::cmp::Reverse(r.tokens));
    rows
}

/// Format an idle duration as a human-readable string like "idle 2m 15s".
pub fn format_idle_duration(secs: u64) -> String {
    if secs < 60 {
        format!("idle {secs}s")
    } else if secs < 3600 {
        format!("idle {}m {}s", secs / 60, secs % 60)
    } else {
        format!("idle {}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Build a display label for a session, appending a short ID prefix when
/// multiple rows share the same slug to disambiguate them.
fn session_label(row: &WatchRow, all_rows: &[WatchRow]) -> String {
    let base = row
        .slug
        .as_deref()
        .unwrap_or_else(|| &row.session_id[..8.min(row.session_id.len())]);

    // Check if any other row has the same slug
    let slug_collides = row.slug.is_some()
        && all_rows
            .iter()
            .any(|r| r.slug == row.slug && r.session_id != row.session_id);

    if slug_collides {
        let prefix = &row.session_id[..4.min(row.session_id.len())];
        format!("{} ({})", base, prefix)
    } else {
        base.to_string()
    }
}

/// Render the watch table with a Status column.
pub fn render_watch_table(rows: &[WatchRow], show_cost: bool) -> String {
    if rows.is_empty() {
        return " No active sessions".to_string();
    }

    let mut table = Table::new();
    table.set_content_arrangement(ContentArrangement::Dynamic);

    if show_cost {
        table.set_header(vec![
            "Project", "Session", "Tokens", "Turns", "Cache", "Model", "Cost", "Status",
        ]);
    } else {
        table.set_header(vec![
            "Project", "Session", "Tokens", "Turns", "Cache", "Model", "Status",
        ]);
    }

    for row in rows {
        let project = shorten_project(&row.project);
        let session = session_label(row, rows);
        let model = row.model.as_deref().unwrap_or("unknown");

        if show_cost {
            table.add_row(vec![
                Cell::new(&project),
                Cell::new(&session),
                Cell::new(format_tokens(row.tokens)),
                Cell::new(row.turns.to_string()),
                Cell::new(format_percent(row.cache_hit)),
                Cell::new(shorten_model(model)),
                Cell::new(format_cost(row.cost.unwrap_or(0.0))),
                Cell::new(&row.status),
            ]);
        } else {
            table.add_row(vec![
                Cell::new(&project),
                Cell::new(&session),
                Cell::new(format_tokens(row.tokens)),
                Cell::new(row.turns.to_string()),
                Cell::new(format_percent(row.cache_hit)),
                Cell::new(shorten_model(model)),
                Cell::new(&row.status),
            ]);
        }
    }

    table.to_string()
}

/// Render the table in-place, clearing previous output.
fn render_table_inplace(rows: &[WatchRow], show_cost: bool, prev_lines: &mut u16) -> Result<()> {
    let mut stdout = io::stdout();

    if *prev_lines > 0 {
        // Subsequent renders: move up and clear previous output
        execute!(
            stdout,
            cursor::MoveUp(*prev_lines),
            terminal::Clear(terminal::ClearType::FromCursorDown)
        )?;
    } else {
        // First render: clear screen and move to top
        execute!(
            stdout,
            terminal::Clear(terminal::ClearType::All),
            cursor::MoveTo(0, 0)
        )?;
    }

    let output = render_watch_table(rows, show_cost);
    let line_count = output.lines().count() as u16;

    writeln!(stdout, "{output}")?;
    stdout.flush()?;

    *prev_lines = line_count + 1; // +1 for the trailing newline
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────

/// Minimal clone of SessionSummary for JSON serialization in watch mode.
/// We need owned data since we're borrowing from the HashMap.
trait CloneForJson {
    fn clone_for_json(&self) -> crate::parser::types::SessionSummary;
}

impl CloneForJson for crate::parser::types::SessionSummary {
    fn clone_for_json(&self) -> crate::parser::types::SessionSummary {
        crate::parser::types::SessionSummary {
            session_id: self.session_id.clone(),
            slug: self.slug.clone(),
            project_path: self.project_path.clone(),
            model: self.model.clone(),
            git_branch: self.git_branch.clone(),
            start_time: self.start_time,
            end_time: self.end_time,
            turns: Vec::new(), // Turns aren't needed for watch JSON; use len from WatchRow
            total_usage: self.total_usage.clone(),
            tool_calls: self.tool_calls.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(project: &str, tokens: u64, turns: usize, show_cost: bool) -> WatchRow {
        WatchRow {
            project: project.to_string(),
            session_id: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".to_string(),
            slug: Some("test-session".to_string()),
            tokens,
            turns,
            cache_hit: 0.75,
            model: Some("claude-opus-4-6".to_string()),
            cost: if show_cost { Some(1.23) } else { None },
            status: "active".to_string(),
        }
    }

    #[test]
    fn test_render_zero_sessions() {
        let output = render_watch_table(&[], false);
        assert_eq!(output, " No active sessions");
    }

    #[test]
    fn test_render_one_session() {
        let rows = vec![make_row("/Users/me/project", 50_000, 10, false)];
        let output = render_watch_table(&rows, false);
        assert!(output.contains("me/project"));
        assert!(output.contains("test-session"));
        assert!(output.contains("50,000"));
        assert!(output.contains("75.0%"));
        assert!(output.contains("Status"));
        assert!(output.contains("active"));
    }

    #[test]
    fn test_render_three_sessions() {
        let rows = vec![
            make_row("/Users/me/project-a", 100_000, 20, false),
            make_row("/Users/me/project-b", 50_000, 10, false),
            make_row("/Users/me/project-c", 25_000, 5, false),
        ];
        let output = render_watch_table(&rows, false);
        assert!(output.contains("me/project-a"));
        assert!(output.contains("me/project-b"));
        assert!(output.contains("me/project-c"));
    }

    #[test]
    fn test_render_with_cost() {
        let rows = vec![make_row("/Users/me/project", 50_000, 10, true)];
        let output = render_watch_table(&rows, true);
        assert!(output.contains("Cost"));
        assert!(output.contains("$1.23"));
    }

    #[test]
    fn test_status_active_vs_idle() {
        let mut tracked: HashMap<PathBuf, WatchedSession> = HashMap::new();
        let now = SystemTime::now();

        // Session active 10s ago — within 60s threshold
        tracked.insert(
            PathBuf::from("/tmp/active.jsonl"),
            WatchedSession {
                session_id: "active-session".to_string(),
                project_path: "/Users/me/active".to_string(),
                last_size: 100,
                last_modified: now - Duration::from_secs(10),
                ever_changed: true,
                discovered_tick: 0,
                summary: Some(crate::parser::types::SessionSummary {
                    session_id: "active-session".to_string(),
                    project_path: "/Users/me/active".to_string(),
                    ..Default::default()
                }),
                cost: Some(crate::cost::CostBreakdown::default()),
                cache_hit: 0.5,
            },
        );

        // Session idle for 120s — outside 60s threshold
        tracked.insert(
            PathBuf::from("/tmp/stale.jsonl"),
            WatchedSession {
                session_id: "stale-session".to_string(),
                project_path: "/Users/me/stale".to_string(),
                last_size: 100,
                last_modified: now - Duration::from_secs(120),
                ever_changed: true,
                discovered_tick: 0,
                summary: Some(crate::parser::types::SessionSummary {
                    session_id: "stale-session".to_string(),
                    project_path: "/Users/me/stale".to_string(),
                    ..Default::default()
                }),
                cost: Some(crate::cost::CostBreakdown::default()),
                cache_hit: 0.3,
            },
        );

        // Session never changed — should not appear
        tracked.insert(
            PathBuf::from("/tmp/never.jsonl"),
            WatchedSession {
                session_id: "never-session".to_string(),
                project_path: "/Users/me/never".to_string(),
                last_size: 100,
                last_modified: now,
                ever_changed: false,
                discovered_tick: 0,
                summary: Some(crate::parser::types::SessionSummary {
                    session_id: "never-session".to_string(),
                    project_path: "/Users/me/never".to_string(),
                    ..Default::default()
                }),
                cost: Some(crate::cost::CostBreakdown::default()),
                cache_hit: 0.0,
            },
        );

        let active_threshold = Duration::from_secs(60);
        let rows = build_rows(&tracked, now, active_threshold, false);

        // Both ever_changed sessions appear, never-changed does not
        assert_eq!(rows.len(), 2);

        let active_row = rows
            .iter()
            .find(|r| r.session_id == "active-session")
            .unwrap();
        assert_eq!(active_row.status, "active");

        let stale_row = rows
            .iter()
            .find(|r| r.session_id == "stale-session")
            .unwrap();
        assert!(
            stale_row.status.starts_with("idle"),
            "expected idle status, got: {}",
            stale_row.status
        );
    }

    #[test]
    fn test_rows_sorted_by_tokens_descending() {
        let mut tracked: HashMap<PathBuf, WatchedSession> = HashMap::new();
        let now = SystemTime::now();

        for (i, tokens) in [500u64, 2000, 1000].iter().enumerate() {
            let id = format!("session-{i}");
            let usage = crate::parser::types::TokenUsage {
                input_tokens: *tokens,
                ..Default::default()
            };

            tracked.insert(
                PathBuf::from(format!("/tmp/{i}.jsonl")),
                WatchedSession {
                    session_id: id.clone(),
                    project_path: format!("/project/{i}"),
                    last_size: 100,
                    last_modified: now,
                    ever_changed: true,
                    discovered_tick: 0,
                    summary: Some(crate::parser::types::SessionSummary {
                        session_id: id,
                        total_usage: usage,
                        ..Default::default()
                    }),
                    cost: Some(crate::cost::CostBreakdown::default()),
                    cache_hit: 0.0,
                },
            );
        }

        let rows = build_rows(&tracked, now, Duration::from_secs(60), false);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].tokens, 2000);
        assert_eq!(rows[1].tokens, 1000);
        assert_eq!(rows[2].tokens, 500);
    }

    #[test]
    fn test_format_idle_duration() {
        assert_eq!(format_idle_duration(0), "idle 0s");
        assert_eq!(format_idle_duration(45), "idle 45s");
        assert_eq!(format_idle_duration(60), "idle 1m 0s");
        assert_eq!(format_idle_duration(135), "idle 2m 15s");
        assert_eq!(format_idle_duration(3600), "idle 1h 0m");
        assert_eq!(format_idle_duration(3900), "idle 1h 5m");
    }
}
