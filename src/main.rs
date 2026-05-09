mod adaptive;
mod bridge;
mod crawler;
mod extract;
mod proxy;
mod robots;
mod store;

use anyhow::Result;
use clap::Parser;
use crawler::CrawlConfig;

#[derive(Parser)]
#[command(name = "imoduru", version, about = "Recursive web crawler — pull everything like a sweet potato vine")]
struct Cli {
    /// Seed URL to start crawling from
    url: String,

    /// Maximum crawl depth (0 = seed page only)
    #[arg(short, long, default_value_t = 3)]
    depth: usize,

    /// Restrict crawl to URLs matching this path prefix
    #[arg(short, long)]
    prefix: Option<String>,

    /// Number of parallel parse workers
    #[arg(short, long, default_value_t = 4)]
    workers: usize,

    /// Rate limit: minimum delay between requests (ms)
    #[arg(short, long, default_value_t = 500)]
    rate_limit: u64,

    /// Max retries per URL on failure
    #[arg(long, default_value_t = 2)]
    max_retries: u32,

    /// Ignore robots.txt
    #[arg(long)]
    ignore_robots: bool,

    /// Request timeout (ms)
    #[arg(short, long, default_value_t = 30000)]
    timeout: u64,

    /// Enable stealth mode (anti-bot bypass, Cloudflare handling)
    #[arg(short, long)]
    stealth: bool,

    /// Fingerprint rotation mode: "default" or "rotate"
    #[arg(long, default_value = "default")]
    fingerprint: String,

    /// Proxy URL (e.g., http://host:port or http://user:pass@host:port)
    #[arg(long)]
    proxy: Option<String>,

    /// Proxy list file (one proxy per line, rotates per-request)
    #[arg(long)]
    proxy_file: Option<String>,

    /// Adaptive selector database path
    #[arg(long)]
    adaptive_db: Option<String>,

    /// Checkpoint file for resume support
    #[arg(long)]
    checkpoint: Option<String>,

    /// Output file (JSON)
    #[arg(short, long, default_value = "imoduru-out.json")]
    output: String,

    /// Also save raw HTML per page
    #[arg(long)]
    save_html: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let seed = url::Url::parse(&cli.url)?;
    let prefix = cli.prefix.unwrap_or_else(|| {
        let p = seed.path().to_string();
        if p.ends_with('/') {
            p
        } else {
            match p.rsplit_once('/') {
                Some((parent, _)) => format!("{parent}/"),
                None => "/".to_string(),
            }
        }
    });

    eprintln!("[imoduru] seed:        {seed}");
    eprintln!("[imoduru] prefix:      {prefix}");
    eprintln!("[imoduru] depth:       {}", cli.depth);
    eprintln!("[imoduru] workers:     {}", cli.workers);
    eprintln!("[imoduru] rate_limit:  {}ms", cli.rate_limit);
    eprintln!("[imoduru] robots:      {}", if cli.ignore_robots { "ignored" } else { "obeyed" });
    eprintln!("[imoduru] stealth:     {}", cli.stealth);
    eprintln!("[imoduru] fingerprint: {}", cli.fingerprint);

    let mut pw = bridge::Playwright::spawn(cli.timeout)?;
    eprintln!("[imoduru] playwright bridge ready");

    // -- Proxy setup
    let proxy_rotator = if let Some(ref proxy_file) = cli.proxy_file {
        let proxies = proxy::load_proxy_file(proxy_file)?;
        eprintln!("[imoduru] loaded {} proxies from {}", proxies.len(), proxy_file);
        Some(proxy::ProxyRotator::new(proxies))
    } else if let Some(ref proxy_url) = cli.proxy {
        eprintln!("[imoduru] proxy: {proxy_url}");
        Some(proxy::ProxyRotator::new(vec![proxy_url.clone()]))
    } else {
        None
    };

    // -- Configure bridge (stealth, fingerprint, proxy)
    let proxy_json = proxy_rotator.as_ref().and_then(|r| {
        r.next().map(|p| p.to_bridge_json())
    });

    if cli.stealth || proxy_json.is_some() || cli.fingerprint != "default" {
        pw.configure(cli.stealth, &cli.fingerprint, proxy_json)?;
        eprintln!("[imoduru] bridge configured (stealth={}, fp={}, proxy={})",
            cli.stealth, cli.fingerprint,
            proxy_rotator.as_ref().map(|r| format!("{} proxies", r.len())).unwrap_or_else(|| "none".into()));
    }

    // -- Adaptive selector store
    let _adaptive_store = if let Some(ref db_path) = cli.adaptive_db {
        let store = adaptive::AdaptiveStore::open(db_path)?;
        eprintln!("[imoduru] adaptive store: {db_path}");
        Some(store)
    } else {
        None
    };

    let config = CrawlConfig {
        max_depth: cli.depth,
        workers: cli.workers,
        rate_limit_ms: cli.rate_limit,
        max_retries: cli.max_retries,
        obey_robots: !cli.ignore_robots,
        timeout_ms: cli.timeout,
        checkpoint_path: cli.checkpoint,
        ..Default::default()
    };

    let mut result = crawler::crawl(&mut pw, &seed, &prefix, &config)?;

    // Strip raw HTML unless --save-html
    if !cli.save_html {
        for page in &mut result.pages {
            page.html.clear();
        }
    }

    eprintln!("\n[imoduru] === crawl complete ===");
    eprintln!("  pages:   {}", result.stats.pages_fetched);
    eprintln!("  failed:  {}", result.stats.pages_failed);
    eprintln!("  retried: {}", result.stats.pages_retried);
    eprintln!("  robots:  {} skipped", result.stats.pages_skipped_robots);
    eprintln!("  bytes:   {}", format_bytes(result.stats.total_bytes));
    eprintln!("  time:    {:.1}s", result.stats.elapsed_ms as f64 / 1000.0);
    if !result.pages.is_empty() {
        let pdf_count: usize = result.pages.iter().map(|p| p.pdf_links.len()).sum();
        if pdf_count > 0 {
            eprintln!("  PDFs:    {} links found", pdf_count);
        }
    }

    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(&cli.output, &json)?;
    eprintln!("[imoduru] saved to {}", cli.output);

    pw.shutdown()?;
    Ok(())
}

fn format_bytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{:.1} MB", n as f64 / (1024.0 * 1024.0))
    }
}
