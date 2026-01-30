//! F* MCP Server - HTTP MCP front-end for F*'s IDE protocol.

mod fstar;
mod mcp;
mod session;

use mcp::create_fstar_server;
use pmcp::server::streamable_http_server::StreamableHttpServer;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// Global verbose flag for detailed F* I/O logging
pub static VERBOSE: AtomicBool = AtomicBool::new(false);

/// Check if verbose mode is enabled
pub fn is_verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Check for --verbose flag
    let args: Vec<String> = std::env::args().collect();
    let verbose = args.iter().any(|a| a == "--verbose" || a == "-v");
    VERBOSE.store(verbose, Ordering::Relaxed);

    // Initialize logging - use debug level if verbose
    let default_filter = if verbose {
        "fstar_mcp=debug,pmcp=debug"
    } else {
        "fstar_mcp=info,pmcp=info"
    };
    
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .init();

    let port: u16 = std::env::var("FSTAR_MCP_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);

    info!("Starting F* MCP server on {}", addr);
    if verbose {
        info!("Verbose mode enabled - logging all F* I/O");
    }

    // Create the MCP server with tools
    let server = create_fstar_server()?;

    // Wrap server in Arc<Mutex<>> for sharing
    let server = Arc::new(Mutex::new(server));

    // Create the streamable HTTP server
    let http_server = StreamableHttpServer::new(addr, server);

    // Start the server
    let (bound_addr, server_handle) = http_server
        .start()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║              F* MCP SERVER RUNNING                        ║");
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ Address: http://{:43} ║", bound_addr);
    println!("║ Mode:    Stateful (with session management)               ║");
    if verbose {
    println!("║ Verbose: ON (logging all F* I/O)                          ║");
    }
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ Available Tools:                                           ║");
    println!("║ • create_fstar      - Create F* session and typecheck      ║");
    println!("║ • list_sessions     - List active sessions with status     ║");
    println!("║ • typecheck_buffer  - Typecheck code (supports lax flag)   ║");
    println!("║ • update_buffer     - Add file to virtual file system      ║");
    println!("║ • lookup_symbol     - Get symbol info at position          ║");
    println!("║ • lookup_by_name    - Get symbol info by name              ║");
    println!("║ • get_proof_context - Get proof goals from tactics         ║");
    println!("║ • autocomplete      - Get completion suggestions           ║");
    println!("║ • restart_solver    - Restart Z3 SMT solver                ║");
    println!("║ • close_session     - Close F* session                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();
    println!("Press Ctrl+C to stop the server");

    // Keep the server running
    server_handle
        .await
        .map_err(|e| pmcp::Error::Internal(e.to_string()))?;

    Ok(())
}
