use serde::{Deserialize, Serialize};

/// A simple configuration struct.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// Server port
    pub port: u16,
    /// Enable debug mode
    pub debug: bool,
    /// Maximum number of workers
    pub max_workers: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            debug: false,
            max_workers: 4,
        }
    }
}

/// Utility function to compute factorial.
pub fn factorial(n: u64) -> u64 {
    match n {
        0 => 1,
        _ => n * factorial(n - 1),
    }
}

/// Check if a string is a palindrome.
pub fn is_palindrome(s: &str) -> bool {
    let cleaned: String = s.chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase();
    cleaned.chars().eq(cleaned.chars().rev())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.port, 8080);
        assert!(!cfg.debug);
        assert_eq!(cfg.max_workers, 4);
    }

    #[test]
    fn test_factorial() {
        assert_eq!(factorial(0), 1);
        assert_eq!(factorial(5), 120);
    }

    #[test]
    fn test_palindrome() {
        assert!(is_palindrome("racecar"));
        assert!(is_palindrome("A man, a plan, a canal: Panama"));
        assert!(!is_palindrome("hello"));
    }
}
