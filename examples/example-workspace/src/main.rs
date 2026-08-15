use serde::{Deserialize, Serialize};

/// Represents a user in the system
#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: u32,
    name: String,
    email: String,
    active: bool,
}

/// Represents a post in the blog system
#[derive(Debug, Serialize, Deserialize)]
struct Post {
    id: u32,
    title: String,
    content: String,
    author_id: u32,
    published: bool,
}

/// Calculate the factorial of a number using recursion
///
/// # Arguments
///
/// * `n` - The number to calculate factorial for (must be non-negative)
///
/// # Returns
///
/// * The factorial of n
///
/// # Examples
///
/// ```
/// let result = factorial(5);
/// assert_eq!(result, 120);
/// ```
fn factorial(n: u64) -> u64 {
    match n {
        0 => 1,
        _ => n * factorial(n - 1),
    }
}

/// Asynchronously fetch user data from an API
async fn fetch_user(user_id: u32) -> Result<User, reqwest::Error> {
    let url = format!("https://api.example.com/users/{}", user_id);
    reqwest::get(&url)
        .await?
        .json::<User>()
        .await
}

/// Processing pipeline for blog posts
fn process_posts(posts: Vec<Post>) -> Vec<Post> {
    posts
        .into_iter()
        .filter(|post| post.published)
        .map(|mut post| {
            // Capitalize the title
            post.title = post.title.to_uppercase();
            post
        })
        .collect()
}

/// Main entry point
fn main() {
    println!("Hello, RustRag example workspace!");
    
    // Create some sample data
    let user = User {
        id: 1,
        name: "John Doe".to_string(),
        email: "john@example.com".to_string(),
        active: true,
    };
    
    let post = Post {
        id: 1,
        title: "Introduction to RustRag".to_string(),
        content: "RustRag is a powerful tool for analyzing Rust codebases...".to_string(),
        author_id: user.id,
        published: true,
    };
    
    // Calculate factorial
    let fact_5 = factorial(5);
    println!("5! = {}", fact_5);
    
    // Process posts
    let posts = vec![post];
    let processed = process_posts(posts);
    println!("Processed {} posts", processed.len());
    
    // Example of unsafe block (for RustRag unsafe region detection)
    let mut num = 5;
    let r1 = &num as *const i32;
    let r2 = &mut num as *mut i32;
    
    unsafe {
        println!("r1 is: {}", *r1);
        println!("r2 is: {}", *r2);
    }
}
