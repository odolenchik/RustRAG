use anyhow::Result;
use clap::{Parser, Subcommand};
use std::sync::{Arc, Mutex};
use rust_rag_server::{build_router, AppState};
use rust_rag_server::mcp::{dispatch_request, err_response, JsonRpcRequest, McpState};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

async fn handle_mcp_socket(
    socket: tokio::net::TcpStream,
    state: Arc<Mutex<McpState>>,
) -> Result<()> {
    let mut buf = BufReader::new(socket);
    loop {
        let mut line = String::new();
        match buf.read_line(&mut line).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Parse single request or batch.
                let requests: Vec<JsonRpcRequest> = match serde_json::from_str(&trimmed) {
                    Ok(req) => vec![req], // single request
                    Err(_) => match serde_json::from_str::<Vec<JsonRpcRequest>>(&trimmed) {
                        Ok(batch) => batch, // batch of requests
                        Err(e) => {
                            // Send parse error response
                            let error_response = err_response(None, -32700, &format!("Parse error: {}", e));
                            let response_json = serde_json::to_string(&error_response).unwrap();
                            let _ = buf.get_mut().write(response_json.as_bytes()).await?;
                            let _ = buf.get_mut().write_all(b"\n").await?;
                            let _ = buf.get_mut().flush().await?;
                            continue;
                        }
                    },
                };

                // Dispatch each request.
                let mut responses = Vec::new();
                for req in requests {
                    let response = dispatch_request(req, &state).await;
                    if let Some(resp) = response {
                        responses.push(resp);
                    }
                }

                // Send response(s)
                if responses.len() == 1 {
                    let response_json = serde_json::to_string(&responses[0]).unwrap();
                    let _ = buf.get_mut().write_all(response_json.as_bytes()).await?;
                    let _ = buf.get_mut().write_all(b"\n").await?;
                } else {
                    let response_json = serde_json::to_string(&responses).unwrap();
                    let _ = buf.get_mut().write_all(response_json.as_bytes()).await?;
                    let _ = buf.get_mut().write_all(b"\n").await?;
                }
                let _ = buf.get_mut().flush().await?;
            }
            Err(_) => break,
        }
    }

    Ok(())
}
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
    /// Port to listen on for TCP MCP server (if not set, uses stdio)
    #[arg(long)]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Serve(args) => run_server(args.port, args.rate_limit).await?,
        Command::Mcp(args) => run_mcp(args.path.as_deref(), args.port).await?,
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

async fn run_mcp(workspace_path: Option<&str>, port: Option<u16>) -> Result<()> {
    let workspace_root = std::env::var("RUSRAG_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            if let Some(p) = workspace_path {
                std::path::PathBuf::from(p)
            } else {
                std::env::current_dir().expect("no CWD")
            }
        });

    if let Some(port) = port {
        println!("Starting RustRAG MCP TCP server on port {}", port);
        println!("Workspace: {}", workspace_root.display());

        let state = Arc::new(Mutex::new(McpState::new(&workspace_root)));
        let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await?;
        loop {
            let (socket, addr) = listener.accept().await?;
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_mcp_socket(socket, state_clone).await {
                    eprintln!("Error handling MCP connection from {}: {}", addr, e);
                }
            });
        }
    } else {
        println!("Starting RustRAG MCP server (stdio)");
        rust_rag_server::mcp::run_mcp_server(&workspace_root).await?;
    }
    Ok(())
}
