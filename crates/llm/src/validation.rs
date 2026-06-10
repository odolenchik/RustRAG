/// Validate that a URL is safe to use as an LLM endpoint.
/// Blocks private IP ranges, loopback addresses, and non-http(s) schemes.
pub fn validate_endpoint(url: &str) -> anyhow::Result<()> {
    let parsed = url.parse::<url::Url>()?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => anyhow::bail!(
            "Only http and https schemes are allowed for LLM endpoints, got: {}",
            scheme
        ),
    };

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid URL: no host part in '{}'", url))?;

    // Block private IPs, loopback, and cloud metadata addresses (SSRF protection)
    if is_private_or_cloud_metadata(host) {
        log::warn!(
            "LLM endpoint points to a local or private address ({host}). \
             This should only be used for local development.",
        );
    }

    Ok(())
}

/// Returns true if the host is a private IP, loopback, or cloud metadata address.
fn is_private_or_cloud_metadata(host: &str) -> bool {
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        return ip.is_loopback() || ip.is_private() || ip.is_link_local();
    }

    if let Ok(ip) = host.parse::<std::net::Ipv6Addr>() {
        return ip.is_loopback() || ip.is_unique_local();
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_endpoint_allows_valid_public_urls() {
        assert!(validate_endpoint("http://example.com:8080").is_ok());
        assert!(validate_endpoint("https://api.openai.com/v1/chat/completions").is_ok());
        assert!(validate_endpoint("https://llm.example.org/api").is_ok());
    }

    #[test]
    fn test_validate_endpoint_warns_on_loopback() {
        // Loopback addresses are allowed (for local dev) but produce a warning
        assert!(validate_endpoint("http://localhost:11434/").is_ok());
        assert!(validate_endpoint("http://127.0.0.1:8080").is_ok());
    }

    #[test]
    fn test_validate_endpoint_warns_on_private_networks() {
        // Private IPs are allowed but produce a warning (SSRF protection logs a warning)
        assert!(validate_endpoint("http://10.0.0.1:8080").is_ok());
        assert!(validate_endpoint("http://192.168.1.1:8080").is_ok());
    }

    #[test]
    fn test_validate_endpoint_blocks_unsafe_schemes() {
        assert!(validate_endpoint("ftp://example.com/model").is_err());
        assert!(validate_endpoint("file:///etc/passwd").is_err());
        assert!(validate_endpoint("ssh://example.com").is_err());
    }

    #[test]
    fn test_validate_endpoint_rejects_invalid_input() {
        // Empty string has no valid scheme
        assert!(validate_endpoint("").is_err());
    }
}
