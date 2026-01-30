//! F* MCP Server - HTTP MCP front-end for F*'s IDE protocol.

mod fstar;
mod mcp;
mod session;

use mcp::create_fstar_server;
use pmcp::server::streamable_http_server::StreamableHttpServer;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "fstar_mcp=info,pmcp=info".into()),
        )
        .init();

    let port: u16 = std::env::var("FSTAR_MCP_PORT")
        .unwrap_or_else(|_| "3000".to_string())
        .parse()
        .unwrap_or(3000);

    let addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port);

    info!("Starting F* MCP server on {}", addr);

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
    println!("╠════════════════════════════════════════════════════════════╣");
    println!("║ Available Tools:                                           ║");
    println!("║ • create_fstar     - Create F* session and typecheck      ║");
    println!("║ • typecheck_buffer - Typecheck code in existing session   ║");
    println!("║ • update_buffer    - Add file to virtual file system      ║");
    println!("║ • lookup_symbol    - Get symbol type/documentation        ║");
    println!("║ • autocomplete     - Get completion suggestions           ║");
    println!("║ • restart_solver   - Restart Z3 SMT solver                ║");
    println!("║ • close_session    - Close F* session                     ║");
    println!("╚════════════════════════════════════════════════════════════╝");
    println!();
    println!("Press Ctrl+C to stop the server");

    // Keep the server running
    server_handle
        .await
        .map_err(|e| pmcp::Error::Internal(e.to_string()))?;

    Ok(())
}
