/// A simple calculator module used for testing.
pub mod calc {
    pub fn add(a: i32, b: i32) -> i32 {
        a + b
    }

    pub fn multiply(a: i32, b: i32) -> i32 {
        a * b
    }
}

/// A struct with methods.
pub struct User {
    name: String,
}

impl User {
    /// Create a new user.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
        }
    }

    /// Return the user's name.
    pub fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}
