use anyhow::Result;

pub fn run(path: &str) -> Result<()> {
    crate::index_workspace(path)
}
