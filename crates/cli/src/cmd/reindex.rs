use anyhow::Result;

pub fn run(path: &str) -> Result<()> {
    crate::reindex_workspace(path)
}
