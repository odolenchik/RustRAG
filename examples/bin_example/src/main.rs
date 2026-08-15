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

/// Processing pipeline for blog posts.
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

/// Represents a post in the blog system.
#[derive(Debug, Serialize, Deserialize)]
struct Post {
    id: u32,
    title: String,
    content: String,
    author_id: u32,
    published: bool,
}

fn main() {
    println!("Hello, binary example!");

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

    // Calculate factorial (using external crate? we could call a function from lib_example if added as dependency)
    // For simplicity, we just print.
    println!("User: {:?}", user);
    println!("Post: {:?}", post);
}

// Note: This binary does not actually run the async function; it's just for demonstration.
