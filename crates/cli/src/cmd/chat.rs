use anyhow::Result;

pub fn run(workspace_path: Option<&str>) -> Result<()> {
    rust_rag_tui::run(workspace_path)?;
    Ok(())
}
