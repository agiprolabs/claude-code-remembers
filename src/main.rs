mod api;
mod consolidate;
mod context;
mod daemon;
mod db;
mod ingest;
mod ipc;
mod mcp;

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use tokio::sync::Notify;
use tracing::{error, info};

use crate::consolidate::consolidation_loop;
use crate::daemon::DaemonState;

#[derive(Parser, Debug)]
#[command(name = "claude-remember", about = "Active memory daemon for Claude Code")]
struct Args {
    /// Path to the project directory
    #[arg(long)]
    project: PathBuf,

    /// Path to the SQLite database file
    #[arg(long)]
    db: PathBuf,

    /// Path to the Unix domain socket (ignored in MCP mode)
    #[arg(long)]
    socket: Option<PathBuf>,

    /// Run as MCP server over stdio instead of Unix socket
    #[arg(long)]
    mcp: bool,

    /// Idle timeout in seconds (default: 7200 = 2 hours)
    #[arg(long, default_value = "7200")]
    idle_timeout: u64,

    /// Consolidation interval in seconds (default: 1800 = 30 minutes)
    #[arg(long, default_value = "1800")]
    consolidation_interval: u64,
}

#[tokio::main]
async fn main() {
    // Initialize logging — stderr only (stdout is for MCP protocol)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("claude_remember=info".parse().unwrap()),
        )
        .init();

    let args = Args::parse();

    // Resolve relative paths
    let project = args.project.canonicalize().unwrap_or(args.project);
    let db = if args.db.is_relative() {
        std::env::current_dir()
            .unwrap_or_default()
            .join(&args.db)
    } else {
        args.db
    };

    info!(
        "Starting claude-remember for project: {}",
        project.display()
    );

    // Ensure DB directory exists
    if let Some(parent) = db.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            error!("Failed to create DB directory: {e}");
            std::process::exit(1);
        }
    }

    // Open database
    let conn = match rusqlite::Connection::open(&db) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to open database: {e}");
            std::process::exit(1);
        }
    };

    // Initialize schema
    if let Err(e) = db::schema::initialize(&conn) {
        error!("Failed to initialize schema: {e}");
        std::process::exit(1);
    }

    // Initialize Haiku client (graceful degradation if no credentials)
    let api = match api::haiku::HaikuClient::from_env() {
        Ok(client) => {
            info!("Haiku API client initialized");
            client
        }
        Err(e) => {
            info!("No API credentials available ({e:?}), running in offline mode");
            api::haiku::HaikuClient::unavailable()
        }
    };

    let state = Arc::new(DaemonState::new(conn, api));

    // Spawn consolidation loop
    let consolidation_state = Arc::clone(&state);
    let consolidation_interval = args.consolidation_interval;
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(consolidation_interval));

        loop {
            interval.tick().await;

            // Phase 1: Fetch data (sync, short lock)
            let data = {
                let conn = consolidation_state.db.lock().unwrap();
                consolidation_loop::fetch_unconsolidated(&conn)
            };

            if let Some((unconsolidated, recent_insights)) = data {
                let ids: Vec<i64> = unconsolidated.iter().map(|m| m.id).collect();

                // Phase 2: Haiku analysis (async, no lock)
                if let Some(result) = consolidation_loop::analyze(
                    &consolidation_state.api,
                    &unconsolidated,
                    &recent_insights,
                )
                .await
                {
                    // Phase 3: Apply results (sync, short lock)
                    let conn = consolidation_state.db.lock().unwrap();
                    consolidation_loop::apply(&conn, result, &ids);
                }

                consolidation_state.update_last_consolidation();
            }
        }
    });

    if args.mcp {
        // MCP mode: serve over stdio
        info!("Running in MCP mode (stdio)");
        mcp::server::serve_stdio(state).await;
    } else {
        // Socket mode: traditional Unix domain socket IPC
        let socket = args.socket.unwrap_or_else(|| {
            error!("--socket is required in socket mode (use --mcp for stdio)");
            std::process::exit(1);
        });

        let activity = Arc::new(Notify::new());

        // Write pidfile
        let pid_path = socket.with_extension("pid");
        if let Err(e) = std::fs::write(&pid_path, std::process::id().to_string()) {
            error!("Failed to write pidfile: {e}");
        }

        // Spawn idle timeout watcher
        let idle_timeout = args.idle_timeout;
        let idle_activity = Arc::clone(&activity);
        let socket_path_for_cleanup = socket.clone();
        let pid_path_for_cleanup = pid_path.clone();
        tokio::spawn(async move {
            loop {
                let timeout = tokio::time::timeout(
                    std::time::Duration::from_secs(idle_timeout),
                    idle_activity.notified(),
                )
                .await;

                if timeout.is_err() {
                    info!("Idle timeout reached ({idle_timeout}s), shutting down");
                    let _ = std::fs::remove_file(&socket_path_for_cleanup);
                    let _ = std::fs::remove_file(&pid_path_for_cleanup);
                    std::process::exit(0);
                }
            }
        });

        // Start IPC server (blocks)
        info!("Daemon ready, socket: {}", socket.display());
        if let Err(e) = ipc::handler::serve(&socket, state, activity).await {
            error!("IPC server error: {e}");
            let _ = std::fs::remove_file(&socket);
            let _ = std::fs::remove_file(&pid_path);
            std::process::exit(1);
        }
    }
}
