# imoduru

Recursive web crawler that pulls everything like a sweet potato vine.

Rust + Playwright. Give it a URL, it follows every link under the same path, extracts text from HTML and PDFs, and serves it all via MCP so AI agents can answer questions from the collected data.

## Install

```bash
git clone https://github.com/O6lvl4/imoduru.git
cd imoduru
cargo build --release
cd bridge && npm install && cd ..
npx playwright install chromium
```

## Usage

### CLI

```bash
# Basic crawl
imoduru crawl "https://example.com/docs/" --depth 2

# With PDF text extraction
imoduru crawl "https://example.com/docs/" --depth 2 --pdf

# Save raw PDFs to disk
imoduru crawl "https://example.com/docs/" --pdf --pdf-dir ./pdfs

# Stealth mode (anti-bot bypass, Cloudflare handling)
imoduru crawl "https://example.com/" --stealth --fingerprint rotate

# With proxy
imoduru crawl "https://example.com/" --proxy http://proxy:8080
imoduru crawl "https://example.com/" --proxy-file proxies.txt

# Resume interrupted crawl
imoduru crawl "https://example.com/" --checkpoint crawl.ckpt

# Full options
imoduru crawl "https://example.com/docs/" \
  --depth 3 \
  --workers 4 \
  --rate-limit 500 \
  --max-retries 2 \
  --timeout 30000 \
  --pdf \
  --stealth \
  --fingerprint rotate \
  --output result.json
```

### MCP Server

Start as an MCP server for Claude Code, Claude Desktop, or any MCP client:

```bash
imoduru mcp
```

#### Tools

| Tool | Description |
|------|-------------|
| `crawl` | Recursively crawl a URL. Params: `url`, `depth`, `prefix`, `stealth`, `fetch_pdfs` |
| `query` | Search crawled pages + PDFs by keyword. Params: `keyword`, `url_filter` |
| `list_pages` | List all crawled pages |
| `get_page` | Get full text of a specific page by URL |

#### Claude Code integration

Add to your `settings.json`:

```json
{
  "mcpServers": {
    "imoduru": {
      "command": "/path/to/imoduru",
      "args": ["mcp"]
    }
  }
}
```

Then Claude Code can:
1. `crawl` a site with PDF extraction
2. `query` the collected data
3. Answer user questions with citations

## Architecture

```
imoduru (Rust)
  ├── crawler    BFS + rayon parallel parse + priority queue
  ├── extract    HTML text/link/PDF-link extraction (scraper crate)
  ├── bridge     Playwright subprocess (Node) for page fetching
  ├── robots     robots.txt parser + compliance
  ├── proxy      Cyclic proxy rotation with error detection
  ├── adaptive   SQLite-backed adaptive selectors (similarity scoring)
  ├── mcp        MCP server (JSON-RPC 2.0 over stdio)
  └── store      Page, PdfContent, CrawlStats, Checkpoint types

bridge/ (Node)
  └── index.mjs  Playwright fetch server with stealth + fingerprint rotation
```

### How it works

1. **Crawl**: BFS from seed URL, respect path prefix, follow links level by level
2. **Fetch**: Playwright renders each page (JS support, stealth mode available)
3. **Parse**: rayon parallel HTML parsing — extract text, links, PDF links
4. **PDF**: Optional — download linked PDFs via native fetch, extract text with pdf-extract
5. **Deduplicate**: SHA256 URL fingerprinting with normalized query params
6. **Rate limit**: Configurable delay between requests, robots.txt crawl-delay support
7. **Output**: JSON with full text, links, PDF texts, crawl stats

### Stealth features

When `--stealth` is enabled:

- `navigator.webdriver` hidden
- `chrome.runtime`, `chrome.loadTimes`, `chrome.csi` spoofed
- Canvas fingerprint noise injection
- WebGL vendor/renderer spoofing
- `navigator.plugins`, `platform`, `hardwareConcurrency`, `deviceMemory` spoofed
- Cloudflare challenge detection + auto-wait
- Error stack trace scrubbing

With `--fingerprint rotate`, each request uses a different browser profile (Chrome/Edge on Mac/Win/Linux).

## Use cases

- **Help desk agent**: Crawl an organization's website, then answer questions via MCP with source citations
- **Documentation indexing**: Pull all pages under `/docs/` and make them searchable
- **Research**: Collect and structure information from multiple pages + linked PDFs
- **Competitive analysis**: Crawl product pages, pricing, specs
- **Archival**: Snapshot a site section with full text + PDF preservation

## License

MIT
