use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

use crate::config::Config;
use crate::output::json::WatchSnapshotJson;

use super::cache::SessionCache;

/// Shared application state for all API handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub show_cost: bool,
    pub watch_tx: broadcast::Sender<WatchSnapshotJson>,
    pub cache: Arc<SessionCache>,
    pub shutdown_tx: Arc<Notify>,
}
