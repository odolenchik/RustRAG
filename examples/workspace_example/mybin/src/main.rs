use mylib::{AppConfig, fibonacci};
use serde::{Deserialize, Serialize};

/// Represents a user in the system.
#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: u32,
    name: String,
    email: String,
    active: bool,
}

/// Asynchronously fetch user data (mock implementation).
async fn fetch_user(user_id: u32) -> Result<User, reqwest::Error> {
    let url = format!("https://api.example.com/users/{}", user_id);
    reqwest::get(&url)
        .await?
        .json::<User>()
        .await
}

fn main() {
    println!("Hello, workspace binary!");

    // Use config from library
    let config = AppConfig::default();
    println!("Config: {:?}", config);

    // Compute Fibonacci
    let n = 10;
    println!("Fibonacci of {} = {}", n, fibonacci(n));

    // Create some sample data
    let user = User {
        id: 1,
        name: "John Doe".to_string(),
        email: "john@example.com".to_string(),
        active: true,
    };
    println!("User: {:?}", user);
}

// Note: This binary does not actually run the async function; it's just for demonstration.
