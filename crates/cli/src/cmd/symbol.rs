use anyhow::Result;

pub fn run_json(query: &str, workspace_root: Option<&str>) -> Result<()> {
    crate::search_symbol_json(query, workspace_root)
}
