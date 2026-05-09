use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub url: String,
    pub title: Option<String>,
    pub text: String,
    pub html: String,
    pub depth: usize,
    pub status: u16,
    pub links: Vec<String>,
    pub pdf_links: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlStats {
    pub pages_fetched: usize,
    pub pages_failed: usize,
    pub pages_skipped_robots: usize,
    pub pages_retried: usize,
    pub total_bytes: usize,
    pub status_codes: HashMap<u16, usize>,
    pub elapsed_ms: u64,
}

impl CrawlStats {
    pub fn new() -> Self {
        Self {
            pages_fetched: 0,
            pages_failed: 0,
            pages_skipped_robots: 0,
            pages_retried: 0,
            total_bytes: 0,
            status_codes: HashMap::new(),
            elapsed_ms: 0,
        }
    }

    pub fn record_status(&mut self, status: u16) {
        *self.status_codes.entry(status).or_insert(0) += 1;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    pub pages: Vec<Page>,
    pub stats: CrawlStats,
}

/// Checkpoint: serializable snapshot of in-progress crawl state.
#[derive(Debug, Serialize, Deserialize)]
pub struct Checkpoint {
    pub visited: Vec<String>,
    pub queue: Vec<(String, usize)>,
    pub pages: Vec<Page>,
    pub stats: CrawlStats,
}
