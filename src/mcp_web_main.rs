use anyhow::{Context, Result, bail};
use axum::{Router, routing::get};
use clap::{Parser, ValueEnum};
use rmcp::transport::{
    StreamableHttpServerConfig, StreamableHttpService,
    streamable_http_server::session::local::LocalSessionManager,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

mod mcp_server;

use mcp_server::{WorldsimMcpServer, WorldsimMcpToolProfile};

#[derive(Debug, Parser)]
#[command(about = "Serve Singulari World MCP over Streamable HTTP for remote ChatGPT app hosts.")]
struct Args {
    #[arg(
        long,
        env = "SINGULARI_WORLD_MCP_WEB_HOST",
        default_value = "127.0.0.1"
    )]
    host: String,
    #[arg(long, env = "SINGULARI_WORLD_MCP_WEB_PORT", default_value_t = 4187)]
    port: u16,
    #[arg(long, env = "SINGULARI_WORLD_MCP_WEB_PATH", default_value = "/mcp")]
    path: String,
    #[arg(long, value_enum, env = "SINGULARI_WORLD_MCP_WEB_PROFILE", default_value_t = WebToolProfile::Play)]
    profile: WebToolProfile,
    #[arg(long, env = "SINGULARI_WORLD_MCP_WEB_ALLOW_PUBLIC_BIND")]
    allow_public_bind: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum WebToolProfile {
    ReadOnly,
    Play,
    TrustedLocal,
}

impl WebToolProfile {
    fn to_mcp_profile(self) -> WorldsimMcpToolProfile {
        match self {
            Self::ReadOnly => WorldsimMcpToolProfile::WebReadOnly,
            Self::Play => WorldsimMcpToolProfile::WebPlay,
            Self::TrustedLocal => WorldsimMcpToolProfile::Full,
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(error) = run_main().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run_main() -> Result<()> {
    let args = Args::parse();
    validate_web_bind(&args)?;

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("singulari_world_mcp_web=info,rmcp=info")
        .init();

    let addr: SocketAddr = format!("{}:{}", args.host, args.port)
        .parse()
        .with_context(|| format!("invalid MCP web bind address: {}:{}", args.host, args.port))?;
    let path = normalize_mount_path(args.path.as_str())?;
    let ct = CancellationToken::new();
    let profile = args.profile.to_mcp_profile();
    let service: StreamableHttpService<WorldsimMcpServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(WorldsimMcpServer::with_profile(profile)),
            Arc::new(LocalSessionManager::default()),
            StreamableHttpServerConfig {
                sse_keep_alive: Some(Duration::from_secs(15)),
                stateful_mode: true,
                cancellation_token: ct.child_token(),
                ..Default::default()
            },
        );
    let router = Router::new()
        .route("/healthz", get(|| async { "ok\n" }))
        .nest_service(path.as_str(), service);
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind MCP web server at http://{addr}{path}"))?;
    tracing::info!(
        "Singulari World MCP web server listening at http://{}{} profile={:?}",
        addr,
        path,
        args.profile
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            ct.cancel();
        })
        .await
        .context("MCP web server failed")?;
    Ok(())
}

fn validate_web_bind(args: &Args) -> Result<()> {
    if args.host == "0.0.0.0" && !args.allow_public_bind {
        bail!("refusing public bind without --allow-public-bind");
    }
    Ok(())
}

fn normalize_mount_path(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("MCP web path cannot be empty");
    }
    if !trimmed.starts_with('/') {
        bail!("MCP web path must start with '/': {trimmed}");
    }
    if trimmed.len() > 1 && trimmed.ends_with('/') {
        return Ok(trimmed.trim_end_matches('/').to_owned());
    }
    Ok(trimmed.to_owned())
}
