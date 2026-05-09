use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub depth: usize,
    pub status: u16,
    pub links: Vec<String>,
}
