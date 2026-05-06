use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::time::{self, Duration};

use crate::config::Config;
use crate::cost::calculator::cache_hit_ratio;
use crate::cost::calculate_usage_cost;
use crate::output::json::{session_to_json, WatchSnapshotJson};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index;
use crate::parser::types::SessionSummary;

struct WatchedSession {
    session_id: String,
    project_path: String,
    last_size: u64,
    ever_changed: bool,
    discovered_tick: u64,
    summary: Option<SessionSummary>,
    cost: Option<crate::cost::CostBreakdown>,
    cache_hit: f64,
}

/// Background task that polls session files and broadcasts snapshots via SSE.
pub async fn run_watcher(
    claude_dir: PathBuf,
    config: Arc<Config>,
    show_cost: bool,
    tx: broadcast::Sender<WatchSnapshotJson>,
) {
    let mut tracked: HashMap<PathBuf, WatchedSession> = HashMap::new();
    let mut tick: u64 = 0;
    let mut interval = time::interval(Duration::from_secs(2));
    // Track last broadcast total_tokens to skip duplicate snapshots
    let mut last_broadcast_tokens: u64 = 0;
    let mut last_broadcast_count: usize = 0;

    loop {
        interval.tick().await;

        // Re-discover sessions every 5 ticks
        if tick.is_multiple_of(5) {
            if let Ok(session_files) = session_index::discover_sessions(&claude_dir) {
                let mut seen: std::collections::HashSet<PathBuf> =
                    std::collections::HashSet::new();

                for sf in &session_files {
                    seen.insert(sf.path.clone());

                    if !tracked.contains_key(&sf.path) {
                        let project_path = decode_project_path(&sf.project_dir_name);
                        let current_size = std::fs::metadata(&sf.path)
                            .map(|m| m.len())
                            .unwrap_or(0);

                        tracked.insert(
                            sf.path.clone(),
                            WatchedSession {
                                session_id: sf.session_id.clone(),
                                project_path,
                                last_size: current_size,
                                ever_changed: false,
                                discovered_tick: tick,
                                summary: None,
                                cost: None,
                                cache_hit: 0.0,
                            },
                        );
                    }
                }

                tracked.retain(|path, _| seen.contains(path));
            }
        }

        let mut any_changed = false;

        for (path, ws) in tracked.iter_mut() {
            if ws.discovered_tick == tick {
                continue;
            }

            let current_size = match std::fs::metadata(path) {
                Ok(m) => m.len(),
                Err(_) => continue,
            };

            if current_size != ws.last_size {
                ws.last_size = current_size;
                ws.ever_changed = true;
                any_changed = true;

                if let Ok(entries) = reader::parse_session_file(path, false) {
                    let summary = reader::summarize_session(
                        &entries,
                        ws.session_id.clone(),
                        ws.project_path.clone(),
                    );

                    let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
                    let pricing = config
                        .pricing_for_model(model_name)
                        .cloned()
                        .unwrap_or(crate::config::ModelPricing {
                            input_per_million: 5.0,
                            output_per_million: 25.0,
                            cache_creation_per_million: 6.25,
                            cache_read_per_million: 0.5,
                        });

                    let cost = calculate_usage_cost(&summary.total_usage, &pricing);
                    let hit = cache_hit_ratio(&summary.total_usage);

                    ws.cost = Some(cost);
                    ws.cache_hit = hit;
                    ws.summary = Some(summary);
                }
            }
        }

        if any_changed {
            let active: Vec<_> = tracked
                .values()
                .filter(|ws| ws.ever_changed && ws.summary.is_some())
                .map(|ws| {
                    let s = ws.summary.as_ref().unwrap();
                    let c = ws.cost.clone().unwrap_or_default();
                    session_to_json(s, &c, ws.cache_hit, show_cost)
                })
                .collect();

            let total_tokens: u64 = tracked
                .values()
                .filter(|ws| ws.ever_changed && ws.summary.is_some())
                .map(|ws| {
                    ws.summary
                        .as_ref()
                        .unwrap()
                        .total_usage
                        .total_tokens()
                })
                .sum();

            // Skip broadcast if tokens and session count are unchanged
            if total_tokens == last_broadcast_tokens && active.len() == last_broadcast_count {
                tick += 1;
                continue;
            }
            last_broadcast_tokens = total_tokens;
            last_broadcast_count = active.len();

            let total_cost = if show_cost {
                Some(
                    tracked
                        .values()
                        .filter(|ws| ws.ever_changed && ws.cost.is_some())
                        .map(|ws| ws.cost.as_ref().unwrap().total())
                        .sum(),
                )
            } else {
                None
            };

            let snapshot = WatchSnapshotJson {
                timestamp: chrono::Utc::now().to_rfc3339(),
                active_sessions: active,
                total_tokens,
                total_cost,
            };

            // Ignore send errors (no subscribers)
            let _ = tx.send(snapshot);
        }

        tick += 1;
    }
}
