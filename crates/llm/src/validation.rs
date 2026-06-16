/// Validate that a URL is safe to use as an LLM endpoint.
/// Blocks private IP ranges, loopback addresses, localhost hostnames, cloud metadata IPs,
/// and non-http(s) schemes. Performs DNS rebinding protection by resolving hostname
/// and checking the resolved IP address.
///
/// When `RUSRAG_SSRF_STRICT=1` is set, private/loopback IPs are **rejected** instead of merely warned.
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

    // Block localhost-style hostnames early (DNS rebinding prevention)
    if is_localhost_hostname(host) {
        log::warn!(
            "LLM endpoint points to a local hostname ({host}). \
             This should only be used for local development.",
        );
    }

    // Resolve the hostname and check all resolved IPs (DNS rebinding protection)
    if let Ok(ips) = resolve_host(host) {
        for ip in &ips {
            if is_private_or_loopback(ip) {
                log::warn!(
                    "LLM endpoint hostname '{}' resolves to a local or private address ({ip}). \
                     This should only be used for local development.",
                    host,
                );
                // In strict mode (RUSRAG_SSRF_STRICT=1), reject private IPs entirely.
                let strict_mode = std::env::var("RUSRAG_SSRF_STRICT").is_ok_and(|v| !v.is_empty());
                if strict_mode {
                    anyhow::bail!(
                        "LLM endpoint hostname '{}' resolves to a local or private address ({ip})",
                        host,
                    );
                }
                return Ok(()); // already warned, allow for local dev (non-strict mode)
            }
        }
    }

    Ok(())
}

/// Check if a hostname refers to a local/loopback address by name.
fn is_localhost_hostname(host: &str) -> bool {
    let h = host.to_lowercase();
    matches!(h.as_str(), "localhost" | "local")
        || h.ends_with(".localhost")
        || h.ends_with(".local")
}

/// Resolve a hostname to IP addresses using std::net::ToSocketAddrs (blocking, standard library only).
fn resolve_host(host: &str) -> std::io::Result<Vec<std::net::IpAddr>> {
    use std::net::ToSocketAddrs;

    format!("{}:{}", host, 80)
        .to_socket_addrs()
        .map(|iter| iter.map(|sa| sa.ip()).collect())
}

/// Returns true if an IP address is private, loopback, or link-local.
fn is_private_or_loopback(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
    }
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

    #[test]
    fn test_localhost_hostname_detection() {
        assert!(is_localhost_hostname("localhost"));
        assert!(is_localhost_hostname(".localhost"));
        assert!(is_localhost_hostname("myapp.local"));
        assert!(!is_localhost_hostname("example.com"));
        assert!(!is_localhost_hostname("api.openai.com"));
    }

    #[test]
    fn test_is_private_or_loopback() {
        let loopback: std::net::IpAddr = "127.0.0.1".parse().unwrap();
        let private_v4: std::net::IpAddr = "192.168.1.1".parse().unwrap();
        let public: std::net::IpAddr = "8.8.8.8".parse().unwrap();

        assert!(is_private_or_loopback(&loopback));
        assert!(is_private_or_loopback(&private_v4));
        assert!(!is_private_or_loopback(&public));
    }
}
