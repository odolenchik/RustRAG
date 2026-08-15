use serde::{Deserialize, Serialize};

/// Configuration for the application.
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

/// Utility function to compute the nth Fibonacci number.
pub fn fibonacci(n: u64) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
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
    fn test_fibonacci() {
        assert_eq!(fibonacci(0), 0);
        assert_eq!(fibonacci(1), 1);
        assert_eq!(fibonacci(5), 5);
        assert_eq!(fibonacci(10), 55);
    }
}
