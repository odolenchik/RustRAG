pub mod app;
pub mod ui;

use std::path::PathBuf;

/// Launch the interactive chat TUI. If `workspace_path` is provided, uses that
/// workspace's `.rustrag/index.jsonl`; otherwise falls back to CWD.
pub fn run(workspace_path: Option<&str>) -> anyhow::Result<()> {
    let workspace_root = if let Some(p) = workspace_path {
        PathBuf::from(p)
    } else {
        std::env::current_dir()?
    };

    app::run_app(&workspace_root)
}
