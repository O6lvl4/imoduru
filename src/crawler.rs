use crate::bridge::Playwright;
use crate::extract;
use crate::store::Page;
use anyhow::Result;
use rayon::ThreadPoolBuilder;
use std::collections::HashSet;
use std::sync::Mutex;
use url::Url;

struct Job {
    url: Url,
    depth: usize,
}

/// BFS crawl from seed URL.
///
/// Uses rayon thread pool for parallel HTML extraction and a single
/// Playwright bridge (mutex-guarded) for fetching. Playwright pages
/// run concurrently inside the Node process, but the bridge protocol
/// is sequential — this is fine because the bottleneck is network I/O
/// inside the Node process, not the Rust→Node pipe.
///
/// For truly concurrent fetching, the bridge could be extended to
/// multiplex (send N requests, collect N responses), but for typical
/// crawl sizes (10–200 pages) the sequential approach is fast enough.
pub fn crawl(
    pw: &mut Playwright,
    seed: &Url,
    path_prefix: &str,
    max_depth: usize,
    workers: usize,
) -> Result<Vec<Page>> {
    let pool = ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()?;

    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: Vec<Job> = vec![Job {
        url: seed.clone(),
        depth: 0,
    }];
    let pages: Mutex<Vec<Page>> = Mutex::new(Vec::new());

    visited.insert(seed.as_str().to_string());

    while !queue.is_empty() {
        eprintln!("[imoduru] level with {} URLs to fetch", queue.len());

        // Fetch all URLs in current BFS level sequentially via bridge
        let mut fetched: Vec<(Job, String, u16)> = Vec::new();
        for job in queue.drain(..) {
            match pw.fetch(job.url.as_str()) {
                Ok(resp) => {
                    let html = resp.html.unwrap_or_default();
                    let status = resp.status.unwrap_or(0);
                    eprintln!("  [{}] {} ({} bytes)", status, job.url, html.len());
                    fetched.push((job, html, status));
                }
                Err(e) => {
                    eprintln!("  [ERR] {} — {e}", job.url);
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

                    let page = Page {
                        url: base.to_string(),
                        title,
                        text,
                        depth: job.depth,
                        status: *status,
                        links: links.iter().map(|u| u.to_string()).collect(),
                    };

                    pages.lock().unwrap().push(page);

                    if job.depth < max_depth {
                        let mut nl = new_links.lock().unwrap();
                        for link in links {
                            nl.push(Job {
                                url: link,
                                depth: job.depth + 1,
                            });
                        }
                    }
                });
            }
        });

        // Deduplicate and enqueue new links
        let new_links = new_links.into_inner().unwrap();
        for job in new_links {
            let key = job.url.as_str().to_string();
            if visited.insert(key) {
                queue.push(job);
            }
        }
    }

    let mut pages = pages.into_inner().unwrap();
    pages.sort_by(|a, b| a.url.cmp(&b.url));
    Ok(pages)
}
