use anyhow::Result;

pub fn run(query: &str) -> Result<()> {
    crate::ask(query, None)
}
