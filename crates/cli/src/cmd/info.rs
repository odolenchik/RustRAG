use anyhow::Result;

pub fn run(workspace_path: Option<&str>) -> Result<()> {
    crate::show_info(workspace_path)
}
