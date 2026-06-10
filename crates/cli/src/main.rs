use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "rust-rag", about = "Local RAG tool for Rust projects")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Index a Cargo workspace
    Index(IndexArgs),
    /// Re-index a workspace (overwrites existing index)
    Reindex(ReindexArgs),
    /// Show index metadata/info
    Info(InfoArgs),
    /// Clean/remove .rustrag directory
    Clean(CleanArgs),
    /// Ask a question about an indexed workspace
    Ask(AskArgs),
    /// Start interactive chat session (WIP)
    Chat(ChatArgs),
    /// Download the embedding model (bge-small-en-v1.5) from HuggingFace
    Download(DownloadArgs),

    /// Search for a symbol by name in the indexed workspace
    Symbol(SymbolArgs),
}

#[derive(clap::Args)]
struct IndexArgs {
    /// Path to the Cargo workspace to index
    path: String,

    /// Force a full re-index even if files haven't changed
    #[arg(long, default_value = "false")]
    force: bool,
}

#[derive(clap::Args)]
struct ReindexArgs {
    /// Path to the Cargo workspace to re-index
    path: String,
}

#[derive(clap::Args)]
struct InfoArgs {
    /// Path to the workspace whose info should be displayed (defaults to current directory)
    #[arg(short, long)]
    path: Option<String>,

    /// Output results as JSON
    #[arg(long, default_value = "false")]
    json: bool,
}

#[derive(clap::Args)]
struct CleanArgs {
    /// Path to the workspace whose .rustrag directory should be removed (defaults to current directory)
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(clap::Args)]
struct AskArgs {
    /// Query string
    query: String,

    /// Path to the workspace whose index should be used (defaults to current directory)
    #[arg(short, long)]
    path: Option<String>,

    /// Stream the LLM response incrementally instead of waiting for full response
    #[arg(long, default_value = "false")]
    stream: bool,

    /// Output results as JSON
    #[arg(long, default_value = "false")]
    json: bool,
}

#[derive(clap::Args)]
struct ChatArgs {
    /// Path to the workspace whose index should be used (defaults to current directory)
    #[arg(short, long)]
    path: Option<String>,
}

#[derive(clap::Args)]
struct SymbolArgs {
    /// Name or partial name of the symbol to search for
    query: String,

    /// Path to the workspace whose index should be used (defaults to current directory)
    #[arg(short, long)]
    path: Option<String>,

    /// Output results as JSON
    #[arg(long, default_value = "false")]
    json: bool,
}

#[derive(clap::Args)]
struct DownloadArgs {
    /// Directory to save the model files (defaults to ~/.cache/huggingface/hub/)
    path: Option<String>,
}

/// Resolve a workspace path argument to an absolute, canonicalized path.
fn resolve_workspace_path(path_arg: Option<&str>) -> Result<Option<std::path::PathBuf>> {
    match path_arg {
        Some(p) => std::fs::canonicalize(p)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("Invalid workspace path '{}': {}", p, e)),
        None => Ok(None),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Index(args) => {
            if args.force {
                let store_path = std::path::PathBuf::from(&args.path).join(".rustrag");
                if store_path.exists() {
                    println!("Removing old index at {}", store_path.display());
                    std::fs::remove_dir_all(&store_path)?;
                }
            }
            rust_rag_cli::index_workspace(&args.path)
        }
        Command::Reindex(args) => rust_rag_cli::reindex_workspace(&args.path),
        Command::Info(args) => {
            let path: Option<String> = resolve_workspace_path(args.path.as_deref())?
                .map(|p| p.to_string_lossy().to_string());
            if args.json {
                rust_rag_cli::show_info_json(path.as_deref())
            } else {
                rust_rag_cli::show_info(path.as_deref())
            }
        }
        Command::Clean(args) => {
            let path: Option<String> = resolve_workspace_path(args.path.as_deref())?
                .map(|p| p.to_string_lossy().to_string());
            rust_rag_cli::clean_workspace(path.as_deref())
        }
        Command::Ask(args) => {
            let workspace: Option<String> = resolve_workspace_path(args.path.as_deref())?
                .map(|p| p.to_string_lossy().to_string());
            if args.json && args.stream {
                rust_rag_cli::ask_stream_json(&args.query, workspace.as_deref()).await
            } else if args.json {
                rust_rag_cli::ask_json(&args.query, workspace.as_deref())
            } else if args.stream {
                rust_rag_cli::ask_stream(&args.query, workspace.as_deref()).await
            } else {
                rust_rag_cli::ask(&args.query, workspace.as_deref())
            }
        }
        Command::Chat(args) => {
            let path: Option<String> = resolve_workspace_path(args.path.as_deref())?
                .map(|p| p.to_string_lossy().to_string());
            return rust_rag_tui::run(path.as_deref());
        }
        Command::Download(args) => {
            let target = if let Some(path) = args.path {
                path
            } else {
                // Default to ~/.cache/huggingface/hub/ (standard HF cache location)
                dirs::cache_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("huggingface")
                    .join("hub")
                    .to_string_lossy()
                    .to_string()
            };
            rust_rag_cli::download_model(&target)
        }
        Command::Symbol(args) => {
            let workspace: Option<String> = resolve_workspace_path(args.path.as_deref())?
                .map(|p| p.to_string_lossy().to_string());
            if args.json {
                rust_rag_cli::search_symbol_json(&args.query, workspace.as_deref())
            } else {
                rust_rag_cli::search_symbol(&args.query, workspace.as_deref())
            }
        }
    }
}
