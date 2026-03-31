//! F* MCP Server - MCP front-end for F*'s IDE protocol.
//!
//! Supports two transports:
//! - **stdio** (default): For use with Claude Code and other MCP clients
//! - **HTTP**: With `--http` flag, runs a streamable HTTP server

mod fstar;
mod mcp;
mod session;

use mcp::{create_fstar_server, SESSION_MANAGER};
use session::DEFAULT_SWEEP_PERIOD_SECS;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::info;

/// Global verbose flag for detailed F* I/O logging
pub static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Check if verbose mode is enabled
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();

    // Check for --verbose flag
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    VERBOSE.store(verbose, Ordering::Relaxed);

    // Check for http flag
    let use_http = args.iter().any(|a| a == "--http");

    // Initialize logging - use stderr so stdout stays clean for stdio transport
    let default_filter = if verbose {
        "fstar_mcp=debug,pmcp=debug"
    } else {
        "fstar_mcp=info,pmcp=info"
    };

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .init();

    let server = create_fstar_server()?;

    if use_http {
        run_http(server, verbose).await
    } else {
        run_stdio(server).await
    }
}

/// Run the server with stdio transport (default, for Claude Code)
async fn run_stdio(server: pmcp::Server) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting F* MCP server on stdio");
    server.run_stdio().await?;
    Ok(())
}

/// Run the server with HTTP transport
async fn run_http(server: pmcp::Server, verbose: bool) -> Result<(), Box<dyn std::error::Error>> {
    use pmcp::server::streamable_http_server::{StreamableHttpServer, StreamableHttpServerConfig};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let sweep_period: u64 = std::env::var("FSTAR_MCP_SWEEP_PERIOD")
        .unwrap_or_else(|_| DEFAULT_SWEEP_PERIOD_SECS.to_string())
        .parse()
        .unwrap_or(DEFAULT_SWEEP_PERIOD_SECS);

    let port: u16 = std::env::var("FSTAR_MCP_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);

    info!("Starting F* MCP server on {}", addr);
    if verbose {
        info!("Verbose mode enabled - logging all F* I/O");
    }
    info!("Session sweep period: {} seconds", sweep_period);

    let server = Arc::new(Mutex::new(server));

    let config = StreamableHttpServerConfig {
        session_id_generator: None,
        enable_json_response: true,
        event_store: None,
        on_session_initialized: Some(Box::new(|session_id| {
            tracing::debug!(mcp_session = %session_id, "MCP session initialized");
        })),
        on_session_closed: Some(Box::new(|session_id| {
            tracing::info!(mcp_session = %session_id, "MCP session closed, marking F* sessions for deletion");
            let session_id = session_id.to_string();
            tokio::spawn(async move {
                SESSION_MANAGER
                    .mark_sessions_for_deletion(&session_id)
                    .await;
            });
        })),
        http_middleware: None,
    };

    let http_server = StreamableHttpServer::with_config(addr, server, config);

    let (bound_addr, server_handle) = http_server
        .start()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    let sweeper_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(sweep_period));
        loop {
            interval.tick().await;
            let count = SESSION_MANAGER.sweep_marked_sessions().await;
            if count > 0 {
                tracing::info!(count = count, "Swept marked sessions");
            }
        }
    });

    eprintln!("╔════════════════════════════════════════════════════════════╗");
    eprintln!("║              F* MCP SERVER RUNNING                        ║");
    eprintln!("╠════════════════════════════════════════════════════════════╣");
    eprintln!("║ Address: http://{:43} ║", bound_addr);
    eprintln!("║ Mode:    Stateful (with session management)               ║");
    eprintln!("║ Sweep:   Every {} seconds{:30} ║", sweep_period, "");
    if verbose {
        eprintln!("║ Verbose: ON (logging all F* I/O)                          ║");
    }
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ Available Tools:                                           ║");
    println!("║ • create_session    - Create F* session and typecheck      ║");
    println!("║ • list_sessions     - List active sessions with status     ║");
    println!("║ • typecheck_buffer  - Typecheck code (supports lax flag)   ║");
    println!("║ • update_buffer     - Add file to virtual file system      ║");
    println!("║ • lookup_symbol     - Get symbol info at position          ║");
    println!("║ • get_proof_context - Get proof goals from tactics         ║");
    println!("║ • restart_solver    - Restart Z3 SMT solver                ║");
    println!("║ • close_session     - Close F* session                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();
    println!("Press Ctrl+C to stop the server");

    tokio::select! {
        result = server_handle => {
            result.map_err(|e| pmcp::Error::Internal(e.to_string()))?;
        }
        _ = sweeper_handle => {}
    }

    Ok(())
}
