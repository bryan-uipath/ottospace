// Made-up library.
pub mod auth;

pub fn greeting(name: &str) -> String {
    if name.is_empty() { "Hello".into() } else { format!("Hello, {name}") }
}
