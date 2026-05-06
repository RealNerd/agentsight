use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::broadcast;

use crate::config::Config;
use crate::server;
use crate::server::cache::SessionCache;

pub struct DashboardArgs {
    pub port: u16,
    pub no_open: bool,
    pub show_cost: bool,
}

pub fn run(claude_dir: &Path, config: &Config, args: &DashboardArgs) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(claude_dir, config, args))
}

async fn run_async(claude_dir: &Path, config: &Config, args: &DashboardArgs) -> Result<()> {
    let (watch_tx, _) = broadcast::channel(64);
    let config = Arc::new(config.clone());

    let cache = Arc::new(SessionCache::new(claude_dir.to_path_buf(), config.clone()));

    // Eagerly populate the cache so the first page load is fast
    cache.refresh().await;

    let state = server::state::AppState {
        config: config.clone(),
        show_cost: args.show_cost,
        watch_tx: watch_tx.clone(),
        cache,
    };

    // Spawn background watcher for SSE
    let watcher_dir = claude_dir.to_path_buf();
    let watcher_config = config;
    let watcher_cost = args.show_cost;
    tokio::spawn(async move {
        server::watcher::run_watcher(watcher_dir, watcher_config, watcher_cost, watch_tx).await;
    });

    let url = format!("http://127.0.0.1:{}", args.port);

    if !args.no_open {
        let open_url = url.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            let _ = open::that(open_url);
        });
    }

    server::start_server(state, args.port).await
}
