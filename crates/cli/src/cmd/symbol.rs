use anyhow::Result;

pub fn run(query: &str, workspace_root: Option<&str>) -> Result<()> {
    crate::search_symbol(query, workspace_root)
}
