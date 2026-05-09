use crate::bridge::Playwright;
use crate::extract;
use crate::robots::RobotsManager;
use crate::store::{Checkpoint, CrawlResult, CrawlStats, Page};
use anyhow::Result;
use rayon::ThreadPoolBuilder;
use sha2::{Digest, Sha256};
use std::cmp::Ordering as CmpOrd;
use std::collections::{BinaryHeap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use url::Url;

/// Crawl configuration.
pub struct CrawlConfig {
    pub max_depth: usize,
    pub workers: usize,
    /// Minimum delay between requests to the same domain (ms).
    pub rate_limit_ms: u64,
    /// Max retries per URL on failure.
    pub max_retries: u32,
    /// Respect robots.txt.
    pub obey_robots: bool,
    /// Request timeout (ms) — passed through to Playwright bridge.
    #[allow(dead_code)]
    pub timeout_ms: u64,
    /// Path to checkpoint file for resume support.
    pub checkpoint_path: Option<String>,
    /// Checkpoint save interval (number of BFS levels).
    pub checkpoint_interval: usize,
    /// Download and extract text from linked PDFs.
    pub fetch_pdfs: bool,
    /// Directory to save raw PDF files (None = don't save).
    pub pdf_dir: Option<String>,
}

impl Default for CrawlConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            workers: 4,
            rate_limit_ms: 500,
            max_retries: 2,
            obey_robots: true,
            timeout_ms: 30_000,
            checkpoint_path: None,
            checkpoint_interval: 1,
            fetch_pdfs: false,
            pdf_dir: None,
        }
    }
}

// -- Priority queue entry: higher priority = dequeued first

#[derive(Debug)]
struct Job {
    url: Url,
    depth: usize,
    priority: i32,
    seq: u64,
}

impl PartialEq for Job {
    fn eq(&self, other: &Self) -> bool {
        self.seq == other.seq
    }
}
impl Eq for Job {}

impl PartialOrd for Job {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrd> {
        Some(self.cmp(other))
    }
}

impl Ord for Job {
    fn cmp(&self, other: &Self) -> CmpOrd {
        // Higher priority first, then lower seq first (FIFO within same priority)
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.seq.cmp(&self.seq))
    }
}

static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// SHA256-based URL fingerprint for deduplication.
fn fingerprint(url: &Url) -> String {
    let mut hasher = Sha256::new();
    // Normalize: scheme + host + path + sorted query
    hasher.update(url.scheme().as_bytes());
    hasher.update(url.host_str().unwrap_or("").as_bytes());
    hasher.update(url.path().as_bytes());
    if let Some(q) = url.query() {
        let mut params: Vec<&str> = q.split('&').collect();
        params.sort();
        for p in params {
            hasher.update(p.as_bytes());
        }
    }
    format!("{:x}", hasher.finalize())
}

/// BFS crawl with Scrapling-inspired features.
pub fn crawl(
    pw: &mut Playwright,
    seed: &Url,
    path_prefix: &str,
    config: &CrawlConfig,
) -> Result<CrawlResult> {
    let start_time = Instant::now();

    let pool = ThreadPoolBuilder::new()
        .num_threads(config.workers)
        .build()?;

    let mut stats = CrawlStats::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue = BinaryHeap::new();
    let pages: Mutex<Vec<Page>> = Mutex::new(Vec::new());

    // -- Attempt to resume from checkpoint
    if let Some(ref cp_path) = config.checkpoint_path {
        if let Ok(data) = std::fs::read_to_string(cp_path) {
            if let Ok(cp) = serde_json::from_str::<Checkpoint>(&data) {
                eprintln!(
                    "[imoduru] resuming from checkpoint ({} pages, {} queued)",
                    cp.pages.len(),
                    cp.queue.len()
                );
                for url_str in &cp.visited {
                    visited.insert(url_str.clone());
                }
                for (url_str, depth) in &cp.queue {
                    if let Ok(url) = Url::parse(url_str) {
                        queue.push(Job {
                            url,
                            depth: *depth,
                            priority: -((*depth) as i32),
                            seq: next_seq(),
                        });
                    }
                }
                *pages.lock().unwrap() = cp.pages;
                stats = cp.stats;
            }
        }
    }

    // Seed URL (if not resuming)
    if visited.is_empty() {
        let fp = fingerprint(seed);
        visited.insert(fp);
        queue.push(Job {
            url: seed.clone(),
            depth: 0,
            priority: 0,
            seq: next_seq(),
        });
    }

    // -- robots.txt
    let mut robots = RobotsManager::new("imoduru");
    if config.obey_robots {
        let origin = format!("{}://{}", seed.scheme(), seed.host_str().unwrap_or(""));
        let robots_url = format!("{origin}/robots.txt");
        match pw.fetch(&robots_url) {
            Ok(resp) => {
                let content = resp.html.unwrap_or_default();
                robots.load(&origin, &content);
                let delay = robots.crawl_delay(&origin);
                if let Some(d) = delay {
                    eprintln!("[imoduru] robots.txt crawl-delay: {d}s");
                }
                eprintln!("[imoduru] robots.txt loaded for {origin}");
            }
            Err(_) => {
                eprintln!("[imoduru] robots.txt not found for {origin} (proceeding)");
            }
        }
    }

    let mut last_fetch = Instant::now();
    let rate_delay = Duration::from_millis(config.rate_limit_ms);
    let mut level_count = 0u64;

    while !queue.is_empty() {
        // Drain current level
        let mut current_level: Vec<Job> = Vec::new();
        while let Some(job) = queue.pop() {
            current_level.push(job);
        }

        eprintln!(
            "[imoduru] level {} — {} URLs to fetch",
            level_count,
            current_level.len()
        );

        let mut fetched: Vec<(Job, String, u16)> = Vec::new();
        for job in current_level {
            // robots.txt check
            if config.obey_robots && !robots.is_allowed(&job.url) {
                eprintln!("  [ROBOTS] {} — disallowed", job.url);
                stats.pages_skipped_robots += 1;
                continue;
            }

            // Rate limiting
            let elapsed = last_fetch.elapsed();
            if elapsed < rate_delay {
                std::thread::sleep(rate_delay - elapsed);
            }

            // Fetch with retry
            let mut attempts = 0;
            loop {
                attempts += 1;
                match pw.fetch(job.url.as_str()) {
                    Ok(resp) => {
                        let html = resp.html.unwrap_or_default();
                        let status = resp.status.unwrap_or(0);
                        stats.total_bytes += html.len();
                        stats.record_status(status);
                        stats.pages_fetched += 1;
                        if attempts > 1 {
                            stats.pages_retried += 1;
                        }
                        eprintln!("  [{}] {} ({} bytes)", status, job.url, html.len());
                        last_fetch = Instant::now();
                        fetched.push((job, html, status));
                        break;
                    }
                    Err(e) => {
                        if attempts <= config.max_retries {
                            eprintln!(
                                "  [RETRY {}/{}] {} — {e}",
                                attempts, config.max_retries, job.url
                            );
                            std::thread::sleep(Duration::from_millis(1000 * attempts as u64));
                            continue;
                        }
                        eprintln!("  [FAIL] {} — {e}", job.url);
                        stats.pages_failed += 1;
                        break;
                    }
                }
            }
        }

        // Parse all fetched pages in parallel with rayon
        let new_links: Mutex<Vec<Job>> = Mutex::new(Vec::new());

        pool.scope(|s| {
            for (job, html, status) in &fetched {
                s.spawn(|_| {
                    let base = &job.url;
                    let title = extract::extract_title(html);
                    let text = extract::extract_text(html);
                    let links = extract::extract_links(html, base, path_prefix);
                    let pdf_links = extract::extract_pdf_links(html, base);

                    let page = Page {
                        url: base.to_string(),
                        title,
                        text,
                        html: html.clone(),
                        depth: job.depth,
                        status: *status,
                        links: links.iter().map(|u| u.to_string()).collect(),
                        pdf_links: pdf_links.iter().map(|u| u.to_string()).collect(),
                        pdfs: Vec::new(),
                    };

                    pages.lock().unwrap().push(page);

                    if job.depth < config.max_depth {
                        let mut nl = new_links.lock().unwrap();
                        for link in links {
                            nl.push(Job {
                                url: link,
                                depth: job.depth + 1,
                                // Lower priority for deeper pages
                                priority: -((job.depth + 1) as i32),
                                seq: next_seq(),
                            });
                        }
                    }
                });
            }
        });

        // Deduplicate and enqueue
        let new_links = new_links.into_inner().unwrap();
        for job in new_links {
            let fp = fingerprint(&job.url);
            if visited.insert(fp) {
                queue.push(job);
            }
        }

        level_count += 1;

        // Checkpoint save
        if let Some(ref cp_path) = config.checkpoint_path {
            if level_count % config.checkpoint_interval as u64 == 0 {
                let queue_snapshot: Vec<(String, usize)> =
                    queue.iter().map(|j| (j.url.to_string(), j.depth)).collect();
                let cp = Checkpoint {
                    visited: visited.iter().cloned().collect(),
                    queue: queue_snapshot,
                    pages: pages.lock().unwrap().clone(),
                    stats: stats.clone(),
                };
                let tmp = format!("{cp_path}.tmp");
                std::fs::write(&tmp, serde_json::to_string(&cp)?)?;
                std::fs::rename(&tmp, cp_path)?;
                eprintln!("[imoduru] checkpoint saved");
            }
        }
    }

    // -- PDF fetch phase
    let mut pages = pages.into_inner().unwrap();

    if config.fetch_pdfs {
        // Collect all unique PDF URLs across all pages
        let mut pdf_urls: Vec<String> = pages
            .iter()
            .flat_map(|p| p.pdf_links.iter().cloned())
            .collect();
        pdf_urls.sort();
        pdf_urls.dedup();

        if !pdf_urls.is_empty() {
            eprintln!("[imoduru] fetching {} PDFs", pdf_urls.len());

            if let Some(ref dir) = config.pdf_dir {
                std::fs::create_dir_all(dir)?;
            }

            let mut pdf_map: std::collections::HashMap<String, crate::store::PdfContent> =
                std::collections::HashMap::new();

            for pdf_url in &pdf_urls {
                // Rate limit
                let elapsed = last_fetch.elapsed();
                if elapsed < rate_delay {
                    std::thread::sleep(rate_delay - elapsed);
                }

                match pw.fetch_binary(pdf_url) {
                    Ok(bytes) => {
                        let size = bytes.len();
                        eprintln!("  [PDF] {} ({} bytes)", pdf_url, size);

                        // Save raw PDF if requested
                        if let Some(ref dir) = config.pdf_dir {
                            let filename = pdf_url
                                .rsplit('/')
                                .next()
                                .unwrap_or("unknown.pdf")
                                .to_string();
                            let path = format!("{dir}/{filename}");
                            let _ = std::fs::write(&path, &bytes);
                        }

                        // Extract text (catch_unwind: pdf-extract panics on some Japanese PDFs)
                        let bytes_clone = bytes.clone();
                        let text = match std::panic::catch_unwind(|| {
                            pdf_extract::extract_text_from_mem(&bytes_clone)
                        }) {
                            Ok(Ok(t)) => t.trim().to_string(),
                            Ok(Err(e)) => {
                                eprintln!("  [PDF-ERR] extraction failed for {pdf_url}: {e}");
                                String::new()
                            }
                            Err(_) => {
                                eprintln!("  [PDF-ERR] extraction panicked for {pdf_url} (likely CJK font)");
                                String::new()
                            }
                        };

                        if !text.is_empty() {
                            eprintln!("    extracted {} chars", text.len());
                        }

                        last_fetch = Instant::now();
                        pdf_map.insert(
                            pdf_url.clone(),
                            crate::store::PdfContent {
                                url: pdf_url.clone(),
                                text,
                                bytes: size,
                            },
                        );
                    }
                    Err(e) => {
                        eprintln!("  [PDF-FAIL] {} — {e}", pdf_url);
                    }
                }
            }

            // Attach PDF contents to their parent pages
            for page in &mut pages {
                for pdf_url in &page.pdf_links {
                    if let Some(pdf) = pdf_map.get(pdf_url) {
                        page.pdfs.push(pdf.clone());
                    }
                }
            }

            eprintln!(
                "[imoduru] PDFs: {} fetched, {} extracted text",
                pdf_map.len(),
                pdf_map.values().filter(|p| !p.text.is_empty()).count()
            );
        }
    }

    stats.elapsed_ms = start_time.elapsed().as_millis() as u64;

    pages.sort_by(|a, b| a.url.cmp(&b.url));

    // Clean up checkpoint on successful completion
    if let Some(ref cp_path) = config.checkpoint_path {
        let _ = std::fs::remove_file(cp_path);
    }

    Ok(CrawlResult { pages, stats })
}
