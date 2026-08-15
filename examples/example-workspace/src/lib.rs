use serde::{Deserialize, Serialize};

/// Configuration for the application
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// Database connection string
    pub database_url: String,
    
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
            database_url: "postgresql://localhost/myapp".to_string(),
            port: 8080,
            debug: false,
            max_workers: 4,
        }
    }
}

/// Database connection pool manager
pub struct DatabasePool {
    config: AppConfig,
    // In a real implementation, this would hold actual pool connections
    initialized: bool,
}

impl DatabasePool {
    /// Create a new database pool from configuration
    pub fn new(config: AppConfig) -> Self {
        Self {
            config,
            initialized: false,
        }
    }
    
    /// Initialize the connection pool
    pub fn initialize(&mut self) -> Result<(), String> {
        // Simulate initialization
        if self.config.database_url.is_empty() {
            return Err("Database URL cannot be empty".to_string());
        }
        
        self.initialized = true;
        Ok(())
    }
    
    /// Check if the pool is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    
    /// Get a connection from the pool
    pub fn get_connection(&self) -> Result<DatabaseConnection, String> {
        if !self.initialized {
            return Err("Database pool not initialized".to_string());
        }
        
        Ok(DatabaseConnection::new())
    }
}

/// Represents a database connection
pub struct DatabaseConnection {
    id: u64,
}

impl DatabaseConnection {
    /// Create a new database connection
    pub fn new() -> Self {
        static mut NEXT_ID: u64 = 0;
        let id = unsafe {
            NEXT_ID += 1;
            NEXT_ID
        };
        
        Self { id }
    }
    
    /// Execute a query
    pub fn execute_query(&self, query: &str) -> Result<QueryResult, String> {
        if query.trim().is_empty() {
            return Err("Query cannot be empty".to_string());
        }
        
        Ok(QueryResult {
            rows_affected: 0,
            data: Vec::new(),
        })
    }
}

/// Result of a database query
pub struct QueryResult {
    /// Number of rows affected by the query
    pub rows_affected: u64,
    
    /// Data returned by the query
    pub data: Vec<String>,
}

/// Utility functions for string manipulation
pub mod utils {
    /// Convert a string to title case
    pub fn to_title_case(s: &str) -> String {
        s.split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ")
    }
    
    /// Check if a string is a palindrome
    pub fn is_palindrome(s: &str) -> bool {
        let cleaned: String = s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
        
        cleaned.chars().eq(cleaned.chars().rev())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_app_config_default() {
        let config = AppConfig::default();
        assert_eq!(config.port, 8080);
        assert!(!config.debug);
        assert_eq!(config.max_workers, 4);
    }
    
    #[test]
    fn test_database_pool_initialization() {
        let mut pool = DatabasePool::new(AppConfig::default());
        assert!(!pool.is_initialized());
        
        let result = pool.initialize();
        assert!(result.is_ok());
        assert!(pool.is_initialized());
    }
    
    #[test]
    fn test_utils_title_case() {
        assert_eq!(utils::to_title_case("hello world"), "Hello World");
        assert_eq!(utils::to_title_case("RUST RAG"), "Rust Rag");
    }
    
    #[test]
    fn test_utils_palindrome() {
        assert!(utils::is_palindrome("racecar"));
        assert!(utils::is_palindrome("A man, a plan, a canal: Panama"));
        assert!(!utils::is_palindrome("hello"));
    }
}
