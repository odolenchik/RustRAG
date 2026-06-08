use anyhow::Result;

pub fn run_json(workspace_path: Option<&str>) -> Result<()> {
    crate::show_info_json(workspace_path)
}
