pub async fn run(
    stream: bool,
    json: bool,
    query: &str,
    workspace_root: Option<&str>,
) -> anyhow::Result<()> {
    if json && stream {
        crate::ask_stream_json(query, workspace_root).await
    } else if json {
        crate::ask_json(query, workspace_root)
    } else if stream {
        crate::ask_stream(query, workspace_root).await
    } else {
        crate::ask(query, workspace_root)
    }
}
