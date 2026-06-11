# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.7.x   | :white_check_mark: |
| < 0.7   | :x:                |

## Reporting a Vulnerability

We take the security of RustRag seriously. If you believe you've found a security vulnerability, please report it responsibly.

### How to Report

**DO NOT** open a public GitHub issue for security vulnerabilities. Instead, choose one of these options:

1. **Email**: Send details to `odolenchik@gmail.com`
2. **GitHub Security Advisories**: Use [Private Vulnerability Reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-vulnerability) via the "Security" tab

### What to Include

When reporting a vulnerability, please include:

- A description of the vulnerability
- Steps to reproduce (proof-of-concept script or commands)
- Potential impact and attack scenario
- Any suggested fixes (if you have them)

### Response Timeline

- **Acknowledgment**: Within 48 hours
- **Assessment**: Within 1 week
- **Fix target**: Depends on severity
  - Critical (RCE, SSRF, auth bypass): within 7 days
  - High: within 30 days
  - Medium/Low: next scheduled release

### What to Expect

1. We'll acknowledge your report and assess the impact
2. We may ask for additional information to fully understand the vulnerability
3. We'll work on a fix and notify you when it's ready
4. Once fixed, we'll credit you (unless you prefer anonymity) in the advisory

## Current Security Measures

RustRag includes several built-in security features:

- **SSRF Protection**: LLM endpoint URLs are validated before client creation, blocking non-http(s) schemes and internal/private IP addresses
- **Path Canonicalization**: CLI commands use `std::fs::canonicalize()` to prevent path traversal attacks
- **.gitignore**: Sensitive directories (`.rustrag/`, `.fastembed_cache/`) and build artifacts are excluded from version control

## Security Best Practices for Users

1. Only set `LLAMA_ENDPOINT` to trusted LLM servers
2. Run the HTTP API server on `127.0.0.1` only — it has no built-in authentication in current versions
3. Do not share `.rustrag/index.jsonl` or `.rustrag/embed_cache.jsonl` if your code contains sensitive information, as indexed source code is stored in plaintext

## Security-Related Dependencies

| Dependency | Purpose | Audit Frequency |
|------------|---------|-----------------|
| `reqwest 0.12` | HTTP client for LLM API and model download | Every update via `cargo audit` |
| `axum 0.7` | Web framework | Every update via `cargo audit` |
| `tower-http 0.6` | HTTP middleware (CORS) | Every update via `cargo audit` |

We run `cargo audit --deny warnings` as part of our CI pipeline to check for known vulnerabilities in dependencies.

## Contact

For any security-related questions, contact: `odolenchik@gmail.com`
