use anyhow::Result;

pub fn run(workspace_path: Option<&str>) -> Result<()> {
    crate::clean_workspace(workspace_path)
}
