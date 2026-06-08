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
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Index(args) => {
            if args.force {
                let workspace_root = std::path::PathBuf::from(&args.path);
                let store_path = workspace_root.join(".rustrag");
                if store_path.exists() {
                    println!("Removing old index at {}", store_path.display());
                    std::fs::remove_dir_all(&store_path)?;
                }
            }
            rust_rag_cli::index_workspace(&args.path)
        }
        Command::Reindex(args) => rust_rag_cli::reindex_workspace(&args.path),
        Command::Info(args) => rust_rag_cli::show_info(args.path.as_deref()),
        Command::Clean(args) => rust_rag_cli::clean_workspace(args.path.as_deref()),
        Command::Ask(args) => {
            let workspace = args.path.as_deref();
            if args.stream {
                tokio::runtime::Runtime::new()?.block_on(rust_rag_cli::ask_stream(&args.query, workspace))
            } else {
                rust_rag_cli::ask(&args.query, workspace)
            }
        }
       Command::Chat(args) => return rust_rag_tui::run(args.path.as_deref()),
        Command::Symbol(args) => rust_rag_cli::search_symbol(&args.query, args.path.as_deref()),
    }
}
