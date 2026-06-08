use anyhow::Result;

pub async fn run(query: &str, stream: bool) -> Result<()> {
    if stream {
        crate::ask_stream(query, None).await?;
    } else {
        crate::ask(query, None)?;
    }
    Ok(())
}
