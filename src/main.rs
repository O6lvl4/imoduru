mod bridge;
mod crawler;
mod extract;
mod store;

use anyhow::Result;
use clap::Parser;

#[derive(Parser)]
#[command(name = "imoduru", about = "Recursive web crawler — pull everything like a sweet potato vine")]
struct Cli {
    /// Seed URL to start crawling from
    url: String,

    /// Maximum crawl depth (0 = seed page only)
    #[arg(short, long, default_value_t = 3)]
    depth: usize,

    /// Restrict crawl to URLs matching this path prefix
    #[arg(short, long)]
    prefix: Option<String>,

    /// Number of parallel fetch workers
    #[arg(short, long, default_value_t = 4)]
    workers: usize,

    /// Output file (JSON)
    #[arg(short, long, default_value = "imoduru-out.json")]
    output: String,
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

    eprintln!("[imoduru] seed:    {seed}");
    eprintln!("[imoduru] prefix:  {prefix}");
    eprintln!("[imoduru] depth:   {}", cli.depth);
    eprintln!("[imoduru] workers: {}", cli.workers);

    let mut pw = bridge::Playwright::spawn()?;
    eprintln!("[imoduru] playwright bridge ready");

    let pages = crawler::crawl(&mut pw, &seed, &prefix, cli.depth, cli.workers)?;

    eprintln!("[imoduru] crawled {} pages", pages.len());

    let json = serde_json::to_string_pretty(&pages)?;
    std::fs::write(&cli.output, &json)?;
    eprintln!("[imoduru] saved to {}", cli.output);

    pw.shutdown()?;
    Ok(())
}
