use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::config::Config;
use crate::cost::calculator::cache_hit_ratio;
use crate::cost::{calculate_usage_cost, CostBreakdown};
use crate::parser::reader::{self, decode_project_path};
use crate::parser::session_index::{self, SessionFile};
use crate::parser::types::SessionSummary;

/// Cached data for a single parsed session file.
pub struct CachedSession {
    pub file_size: u64,
    pub session_id: String,
    pub project_path: String,
    pub summary: SessionSummary,
    pub cost: CostBreakdown,
    pub cache_hit: f64,
}

struct CacheInner {
    /// Per-file parse cache. Key = absolute file path, invalidated when file_size changes.
    sessions: HashMap<PathBuf, Arc<CachedSession>>,
    /// Cached directory scan result with timestamp.
    discovered_files: Vec<SessionFile>,
    last_discovery: Option<Instant>,
    /// Cached project list derived from discovery.
    projects: Vec<String>,
}

/// Two-layer session cache: discovery scan (directory listing) + per-file parse results.
///
/// JSONL files are append-only, so file size is a reliable invalidation key.
/// Discovery is re-run every 2 seconds. Parse results persist until the file grows.
pub struct SessionCache {
    inner: RwLock<CacheInner>,
    claude_dir: PathBuf,
    config: Arc<Config>,
}

/// How often to re-scan the projects directory for new/removed session files.
const DISCOVERY_TTL_MS: u64 = 2000;

impl SessionCache {
    pub fn new(claude_dir: PathBuf, config: Arc<Config>) -> Self {
        Self {
            inner: RwLock::new(CacheInner {
                sessions: HashMap::new(),
                discovered_files: Vec::new(),
                last_discovery: None,
                projects: Vec::new(),
            }),
            claude_dir,
            config,
        }
    }

    /// Refresh discovery + re-parse any changed files. Called at the start of each request.
    pub async fn refresh(&self) {
        let needs_discovery = {
            let inner = self.inner.read().await;
            match inner.last_discovery {
                None => true,
                Some(t) => t.elapsed().as_millis() > DISCOVERY_TTL_MS as u128,
            }
        };

        if needs_discovery {
            self.refresh_discovery().await;
        }

        self.refresh_sessions().await;
    }

    /// Re-scan the projects directory for session files.
    async fn refresh_discovery(&self) {
        let discovered = match session_index::discover_sessions(&self.claude_dir) {
            Ok(files) => files,
            Err(_) => return,
        };

        // Derive unique project list
        let mut project_set: HashMap<String, ()> = HashMap::new();
        for sf in &discovered {
            let path = decode_project_path(&sf.project_dir_name);
            let short = crate::output::table::shorten_project(&path);
            project_set.entry(short).or_default();
        }
        let mut projects: Vec<String> = project_set.into_keys().collect();
        projects.sort();

        let mut inner = self.inner.write().await;

        // Evict cached sessions whose files no longer exist
        let discovered_paths: std::collections::HashSet<&Path> =
            discovered.iter().map(|sf| sf.path.as_path()).collect();
        inner
            .sessions
            .retain(|path, _| discovered_paths.contains(path.as_path()));

        inner.discovered_files = discovered;
        inner.last_discovery = Some(Instant::now());
        inner.projects = projects;
    }

    /// Check each discovered file for size changes; re-parse only what changed.
    async fn refresh_sessions(&self) {
        // Read phase: collect files that need re-parsing
        let to_parse: Vec<(PathBuf, String, String, u64)> = {
            let inner = self.inner.read().await;
            inner
                .discovered_files
                .iter()
                .filter_map(|sf| {
                    let current_size = std::fs::metadata(&sf.path).ok()?.len();
                    // Check if we have a cached entry with matching size
                    if let Some(cached) = inner.sessions.get(&sf.path) {
                        if cached.file_size == current_size {
                            return None; // Cache hit — skip
                        }
                    }
                    let project_path = decode_project_path(&sf.project_dir_name);
                    Some((
                        sf.path.clone(),
                        sf.session_id.clone(),
                        project_path,
                        current_size,
                    ))
                })
                .collect()
        };

        if to_parse.is_empty() {
            return;
        }

        // Parse phase: do I/O without holding the lock
        let mut parsed: Vec<(PathBuf, Arc<CachedSession>)> = Vec::with_capacity(to_parse.len());
        for (path, session_id, project_path, file_size) in to_parse {
            let entries = match reader::parse_session_file(&path, false) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let summary =
                reader::summarize_session(&entries, session_id.clone(), project_path.clone());

            let model_name = summary.model.as_deref().unwrap_or("claude-opus-4-6");
            let pricing = self
                .config
                .pricing_for_model(model_name)
                .cloned()
                .unwrap_or_default();

            let cost = calculate_usage_cost(&summary.total_usage, &pricing);
            let hit = cache_hit_ratio(&summary.total_usage);

            parsed.push((
                path,
                Arc::new(CachedSession {
                    file_size,
                    session_id,
                    project_path,
                    summary,
                    cost,
                    cache_hit: hit,
                }),
            ));
        }

        // Write phase: insert parsed results
        let mut inner = self.inner.write().await;
        for (path, cached) in parsed {
            inner.sessions.insert(path, cached);
        }
    }

    /// Get all cached sessions as Arc references (cheap clones).
    pub async fn get_all(&self) -> Vec<Arc<CachedSession>> {
        let inner = self.inner.read().await;
        inner.sessions.values().cloned().collect()
    }

    /// Get a single session by UUID prefix match.
    pub async fn get_by_id(&self, id: &str) -> Option<Arc<CachedSession>> {
        let inner = self.inner.read().await;
        inner
            .sessions
            .values()
            .find(|cs| cs.session_id.starts_with(id))
            .cloned()
    }

    /// Get the most recent session matching a slug (exact or substring).
    /// When multiple sessions share a slug, returns the one with the latest start_time.
    pub async fn get_by_slug_best(&self, slug: &str) -> Option<Arc<CachedSession>> {
        let inner = self.inner.read().await;
        let slug_lower = slug.to_lowercase();

        inner
            .sessions
            .values()
            .filter(|cs| {
                cs.summary.slug.as_deref().is_some_and(|s| {
                    let s_lower = s.to_lowercase();
                    s_lower == slug_lower || s_lower.contains(&slug_lower)
                })
            })
            .max_by_key(|cs| cs.summary.start_time)
            .cloned()
    }

    /// Get the cached project list.
    pub async fn get_projects(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.projects.clone()
    }
}
