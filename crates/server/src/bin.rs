use anyhow::Result;
use clap::{Parser, Subcommand};
use rust_rag_server::{build_router, AppState};

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start HTTP API server (default)
    Serve(ServeArgs),
    /// Start MCP stdio server
    Mcp(McpArgs),
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Port to listen on
    #[arg(short, long, default_value_t = 8090u16)]
    port: u16,

    /// Max requests per minute for rate limiting (default: 60)
    #[arg(long, default_value_t = 60)]
    rate_limit: u32,

    /// Maximum context size in bytes sent to the LLM (env override: RUSRAG_MAX_CONTEXT_SIZE)
    #[arg(long)]
    max_context_size: Option<usize>,
}

#[derive(clap::Args)]
struct McpArgs {
    /// Path to workspace (defaults to current directory)
    path: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve(args) => run_server(args.port, args.rate_limit).await?,
        Command::Mcp(args) => run_mcp(args.path.as_deref()).await?,
    }

    Ok(())
}

async fn run_server(port: u16, rate_limit: u32) -> Result<()> {
    let workspace_root = std::env::var("RUSRAG_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or(std::env::current_dir().expect("no CWD"));

    println!("Starting RustRAG server on port {}", port);
    println!(
        "Workspace: {} | Rate limit: {} req/min",
        workspace_root.display(),
        rate_limit
    );

    let state = AppState::from_workspace(&workspace_root, rate_limit)?;
    let app = build_router(state);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    axum::serve(listener, app).await?;

    Ok(())
}

async fn run_mcp(workspace_path: Option<&str>) -> Result<()> {
    let workspace_root = std::env::var("RUSRAG_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if let Some(p) = workspace_path {
                std::path::PathBuf::from(p)
            } else {
                std::env::current_dir().expect("no CWD")
            }
        });

    println!("Starting RustRAG MCP server (stdio)");
    rust_rag_server::mcp::run_mcp_server(&workspace_root).await?;
    Ok(())
}
