#[allow(dead_code)]
mod adaptive;
mod bridge;
mod crawler;
mod extract;
mod mcp;
#[allow(dead_code)]
mod proxy;
mod robots;
mod store;

use anyhow::Result;
use clap::{Parser, Subcommand};
use crawler::CrawlConfig;

#[derive(Parser)]
#[command(
    name = "imoduru",
    version,
    about = "Recursive web crawler — pull everything like a sweet potato vine"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Crawl a URL recursively
    Crawl(Box<CrawlArgs>),
    /// Start MCP server (JSON-RPC over stdio)
    Mcp {
        /// Request timeout (ms)
        #[arg(short, long, default_value_t = 30000)]
        timeout: u64,
    },
}

#[derive(Parser)]
struct CrawlArgs {
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

    /// Download and extract text from linked PDFs
    #[arg(long)]
    pdf: bool,

    /// Directory to save raw PDF files
    #[arg(long)]
    pdf_dir: Option<String>,

    /// Output file (JSON)
    #[arg(short, long, default_value = "imoduru-out.json")]
    output: String,

    /// Also save raw HTML per page
    #[arg(long)]
    save_html: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Crawl(args) => run_crawl(*args),
        Commands::Mcp { timeout } => mcp::run_mcp_server(timeout),
    }
}

fn run_crawl(args: CrawlArgs) -> Result<()> {
    let seed = url::Url::parse(&args.url)?;
    let prefix = args.prefix.unwrap_or_else(|| {
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
    eprintln!("[imoduru] depth:       {}", args.depth);
    eprintln!("[imoduru] workers:     {}", args.workers);
    eprintln!("[imoduru] rate_limit:  {}ms", args.rate_limit);
    eprintln!(
        "[imoduru] robots:      {}",
        if args.ignore_robots {
            "ignored"
        } else {
            "obeyed"
        }
    );
    eprintln!("[imoduru] stealth:     {}", args.stealth);
    eprintln!("[imoduru] fingerprint: {}", args.fingerprint);

    let mut pw = bridge::Playwright::spawn(args.timeout)?;
    eprintln!("[imoduru] playwright bridge ready");

    // Proxy setup
    let proxy_rotator = if let Some(ref proxy_file) = args.proxy_file {
        let proxies = proxy::load_proxy_file(proxy_file)?;
        eprintln!(
            "[imoduru] loaded {} proxies from {}",
            proxies.len(),
            proxy_file
        );
        Some(proxy::ProxyRotator::new(proxies))
    } else if let Some(ref proxy_url) = args.proxy {
        eprintln!("[imoduru] proxy: {proxy_url}");
        Some(proxy::ProxyRotator::new(vec![proxy_url.clone()]))
    } else {
        None
    };

    let proxy_json = proxy_rotator
        .as_ref()
        .and_then(|r| r.next().map(|p| p.to_bridge_json()));

    if args.stealth || proxy_json.is_some() || args.fingerprint != "default" {
        pw.configure(args.stealth, &args.fingerprint, proxy_json)?;
        eprintln!(
            "[imoduru] bridge configured (stealth={}, fp={}, proxy={})",
            args.stealth,
            args.fingerprint,
            proxy_rotator
                .as_ref()
                .map(|r| format!("{} proxies", r.len()))
                .unwrap_or_else(|| "none".into())
        );
    }

    let _adaptive_store = if let Some(ref db_path) = args.adaptive_db {
        let store = adaptive::AdaptiveStore::open(db_path)?;
        eprintln!("[imoduru] adaptive store: {db_path}");
        Some(store)
    } else {
        None
    };

    let config = CrawlConfig {
        max_depth: args.depth,
        workers: args.workers,
        rate_limit_ms: args.rate_limit,
        max_retries: args.max_retries,
        obey_robots: !args.ignore_robots,
        timeout_ms: args.timeout,
        checkpoint_path: args.checkpoint,
        fetch_pdfs: args.pdf || args.pdf_dir.is_some(),
        pdf_dir: args.pdf_dir,
        ..Default::default()
    };

    let mut result = crawler::crawl(&mut pw, &seed, &prefix, &config)?;

    if !args.save_html {
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
        let pdf_link_count: usize = result.pages.iter().map(|p| p.pdf_links.len()).sum();
        let pdf_fetched: usize = result.pages.iter().map(|p| p.pdfs.len()).sum();
        let pdf_text_chars: usize = result
            .pages
            .iter()
            .flat_map(|p| &p.pdfs)
            .map(|pdf| pdf.text.len())
            .sum();
        if pdf_link_count > 0 {
            eprintln!("  PDFs:    {} links found", pdf_link_count);
            if pdf_fetched > 0 {
                eprintln!(
                    "  PDFs:    {} fetched, {} chars extracted",
                    pdf_fetched, pdf_text_chars
                );
            }
        }
    }

    let json = serde_json::to_string_pretty(&result)?;
    std::fs::write(&args.output, &json)?;
    eprintln!("[imoduru] saved to {}", args.output);

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
