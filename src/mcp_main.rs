use anyhow::{Result, bail};
use rmcp::{ServiceExt, transport::stdio};

mod mcp_server;

use mcp_server::WorldsimMcpServer;

#[tokio::main]
async fn main() {
    if let Err(error) = run_main().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run_main() -> Result<()> {
    if let Some(command) = std::env::args().nth(1) {
        bail!("unknown singulari-world-mcp command: {command}");
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("singulari_world_mcp=info")
        .init();

    tracing::info!("Singulari World MCP stdio server starting");
    let service = WorldsimMcpServer::new()
        .serve(stdio())
        .await
        .inspect_err(|error| tracing::error!("MCP server error: {error}"))?;
    service.waiting().await?;
    Ok(())
}
