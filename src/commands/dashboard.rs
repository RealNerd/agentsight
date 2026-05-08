use anyhow::{bail, Result};
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::Arc;
use tokio::sync::{broadcast, Notify};

use crate::config::Config;
use crate::server;
use crate::server::cache::SessionCache;

pub struct DashboardArgs {
    pub port: u16,
    pub no_open: bool,
    pub show_cost: bool,
    pub replace: bool,
}

pub fn run(claude_dir: &Path, config: &Config, args: &DashboardArgs) -> Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(run_async(claude_dir, config, args))
}

enum ProbeResult {
    PortFree,
    AgentsightRunning { pid: u32 },
    OtherProcess,
}

/// Probe the port to see what's running there.
///
/// Connects via raw TCP, sends an HTTP/1.1 GET to `/api/v1/health`, and
/// checks for `"service":"agentsight"` in the JSON response.
fn probe_existing_dashboard(port: u16) -> ProbeResult {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
        Ok(s) => s,
        Err(_) => return ProbeResult::PortFree,
    };

    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();

    let request = format!(
        "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        port
    );

    if stream.write_all(request.as_bytes()).is_err() {
        return ProbeResult::OtherProcess;
    }

    let mut buf = vec![0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return ProbeResult::OtherProcess,
    };

    let response = String::from_utf8_lossy(&buf[..n]);

    // Look for the JSON body after the HTTP headers
    if let Some(body_start) = response.find("\r\n\r\n") {
        let body = &response[body_start + 4..];
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
            if json.get("service").and_then(|v| v.as_str()) == Some("agentsight") {
                let pid = json.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                return ProbeResult::AgentsightRunning { pid };
            }
        }
    }

    ProbeResult::OtherProcess
}

/// Send a shutdown request to an existing agentsight instance.
fn request_shutdown(port: u16) -> Result<()> {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(2))).ok();

    let request = format!(
        "POST /api/v1/shutdown HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        port
    );
    stream.write_all(request.as_bytes())?;

    // Read response to ensure the request was processed
    let mut buf = vec![0u8; 1024];
    let _ = stream.read(&mut buf);

    Ok(())
}

/// Wait for the port to become free, polling every 100ms up to 3 seconds.
fn wait_for_port_free(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    let addr: SocketAddr = ([127, 0, 0, 1], port).into();

    for _ in 0..30 {
        std::thread::sleep(Duration::from_millis(100));
        if TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err() {
            return true;
        }
    }
    false
}

async fn run_async(claude_dir: &Path, config: &Config, args: &DashboardArgs) -> Result<()> {
    // Probe the port before attempting to bind (skip for port 0 = ephemeral)
    if args.port != 0 {
        match probe_existing_dashboard(args.port) {
            ProbeResult::PortFree => {} // proceed normally
            ProbeResult::AgentsightRunning { pid } => {
                if args.replace {
                    eprintln!(
                        "Shutting down existing dashboard (pid {}) on port {}...",
                        pid, args.port
                    );
                    request_shutdown(args.port)?;
                    if !wait_for_port_free(args.port) {
                        bail!(
                            "Timed out waiting for port {} to free after shutdown request",
                            args.port
                        );
                    }
                    // Fall through to start fresh
                } else {
                    let url = format!("http://127.0.0.1:{}", args.port);
                    eprintln!("Dashboard already running at {}", url);
                    if !args.no_open {
                        let _ = open::that(&url);
                    }
                    return Ok(());
                }
            }
            ProbeResult::OtherProcess => {
                bail!(
                    "Port {} is in use by another application.\n\n\
                     Try:  agentsight dashboard --port 0    (auto-select a free port)\n\
                           agentsight dashboard --port 8080 (choose a specific port)",
                    args.port
                );
            }
        }
    }

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
        shutdown_tx: Arc::new(Notify::new()),
    };

    // Spawn background watcher for SSE
    let watcher_dir = claude_dir.to_path_buf();
    let watcher_config = config;
    let watcher_cost = args.show_cost;
    tokio::spawn(async move {
        server::watcher::run_watcher(watcher_dir, watcher_config, watcher_cost, watch_tx).await;
    });

    let open_browser = !args.no_open;
    server::start_server(state, args.port, open_browser).await
}
