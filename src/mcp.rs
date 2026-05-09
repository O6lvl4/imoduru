use crate::bridge::Playwright;
use crate::crawler::{self, CrawlConfig};
use crate::store::CrawlResult;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::sync::Mutex;

/// In-memory store of crawl results, keyed by origin+prefix.
struct State {
    pw: Playwright,
    results: HashMap<String, CrawlResult>,
}

// -- JSON-RPC types

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: Option<String>,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Serialize)]
struct JsonRpcResponse {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

fn ok_response(id: Value, result: Value) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: Some(result),
        error: None,
    }
}

fn err_response(id: Value, code: i32, msg: &str) -> JsonRpcResponse {
    JsonRpcResponse {
        jsonrpc: "2.0",
        id,
        result: None,
        error: Some(JsonRpcError {
            code,
            message: msg.to_string(),
        }),
    }
}

// -- Tool definitions

fn tool_definitions() -> Value {
    json!({
        "tools": [
            {
                "name": "crawl",
                "description": "Recursively crawl a URL and all linked pages under the same path prefix. Returns structured text content, links, and PDF links for each page.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Seed URL to start crawling from"
                        },
                        "depth": {
                            "type": "integer",
                            "description": "Maximum crawl depth (default: 2)",
                            "default": 2
                        },
                        "prefix": {
                            "type": "string",
                            "description": "Path prefix to restrict crawl scope (default: auto-detected from URL)"
                        },
                        "stealth": {
                            "type": "boolean",
                            "description": "Enable stealth mode for anti-bot bypass",
                            "default": false
                        },
                        "fetch_pdfs": {
                            "type": "boolean",
                            "description": "Download linked PDFs and extract their text content",
                            "default": false
                        }
                    },
                    "required": ["url"]
                }
            },
            {
                "name": "query",
                "description": "Search previously crawled pages by keyword. Returns matching page titles, URLs, and relevant text excerpts.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "keyword": {
                            "type": "string",
                            "description": "Keyword to search for in crawled page text"
                        },
                        "url_filter": {
                            "type": "string",
                            "description": "Only search pages whose URL contains this substring"
                        }
                    },
                    "required": ["keyword"]
                }
            },
            {
                "name": "list_pages",
                "description": "List all previously crawled pages with their titles and URLs.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "get_page",
                "description": "Get the full text content of a specific crawled page by URL.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "URL of the page to retrieve"
                        }
                    },
                    "required": ["url"]
                }
            }
        ]
    })
}

// -- Tool handlers

fn handle_crawl(state: &Mutex<State>, params: &Value) -> Result<Value> {
    let url_str = params["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'url' parameter"))?;
    let depth = params["depth"].as_u64().unwrap_or(2) as usize;
    let stealth = params["stealth"].as_bool().unwrap_or(false);
    let fetch_pdfs = params["fetch_pdfs"].as_bool().unwrap_or(false);

    let seed = url::Url::parse(url_str)?;
    let prefix = params["prefix"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
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

    let config = CrawlConfig {
        max_depth: depth,
        workers: 4,
        rate_limit_ms: 500,
        max_retries: 2,
        obey_robots: true,
        fetch_pdfs,
        ..Default::default()
    };

    let mut st = state.lock().unwrap();

    if stealth {
        let _ = st.pw.configure(true, "rotate", None);
    }

    let result = crawler::crawl(&mut st.pw, &seed, &prefix, &config)?;

    let summary = json!({
        "pages_crawled": result.stats.pages_fetched,
        "pages_failed": result.stats.pages_failed,
        "total_bytes": result.stats.total_bytes,
        "elapsed_ms": result.stats.elapsed_ms,
        "pages": result.pages.iter().map(|p| {
            let mut page = json!({
                "url": p.url,
                "title": p.title,
                "text": p.text,
                "pdf_links": p.pdf_links,
                "depth": p.depth,
            });
            if !p.pdfs.is_empty() {
                page["pdfs"] = json!(p.pdfs.iter().map(|pdf| json!({
                    "url": pdf.url,
                    "text": pdf.text,
                    "bytes": pdf.bytes,
                })).collect::<Vec<_>>());
            }
            page
        }).collect::<Vec<_>>(),
    });

    let key = format!("{}|{}", seed.origin().ascii_serialization(), prefix);
    st.results.insert(key, result);

    Ok(summary)
}

fn handle_query(state: &Mutex<State>, params: &Value) -> Result<Value> {
    let keyword = params["keyword"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'keyword' parameter"))?
        .to_lowercase();
    let url_filter = params["url_filter"].as_str().unwrap_or("");

    let st = state.lock().unwrap();
    let mut matches = Vec::new();

    for result in st.results.values() {
        for page in &result.pages {
            if !url_filter.is_empty() && !page.url.contains(url_filter) {
                continue;
            }
            // Search in page text
            let text_lower = page.text.to_lowercase();
            if text_lower.contains(&keyword) {
                let excerpt = extract_excerpt(&page.text, &keyword, 200);
                matches.push(json!({
                    "url": page.url,
                    "title": page.title,
                    "source": "html",
                    "excerpt": excerpt,
                }));
            }
            // Search in attached PDF texts
            for pdf in &page.pdfs {
                let pdf_lower = pdf.text.to_lowercase();
                if pdf_lower.contains(&keyword) {
                    let excerpt = extract_excerpt(&pdf.text, &keyword, 200);
                    matches.push(json!({
                        "url": pdf.url,
                        "title": page.title,
                        "source": "pdf",
                        "excerpt": excerpt,
                    }));
                }
            }
        }
    }

    Ok(json!({ "matches": matches, "count": matches.len() }))
}

fn handle_list_pages(state: &Mutex<State>) -> Result<Value> {
    let st = state.lock().unwrap();
    let mut pages = Vec::new();
    for result in st.results.values() {
        for page in &result.pages {
            pages.push(json!({
                "url": page.url,
                "title": page.title,
                "depth": page.depth,
                "text_length": page.text.len(),
                "pdf_links_count": page.pdf_links.len(),
            }));
        }
    }
    Ok(json!({ "pages": pages, "total": pages.len() }))
}

fn handle_get_page(state: &Mutex<State>, params: &Value) -> Result<Value> {
    let url = params["url"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("missing 'url' parameter"))?;

    let st = state.lock().unwrap();
    for result in st.results.values() {
        for page in &result.pages {
            if page.url == url {
                return Ok(json!({
                    "url": page.url,
                    "title": page.title,
                    "text": page.text,
                    "links": page.links,
                    "pdf_links": page.pdf_links,
                    "depth": page.depth,
                    "status": page.status,
                }));
            }
        }
    }

    anyhow::bail!("page not found: {url}")
}

fn extract_excerpt(text: &str, keyword: &str, context_chars: usize) -> String {
    let lower = text.to_lowercase();
    let Some(pos) = lower.find(keyword) else {
        return String::new();
    };
    let start = pos.saturating_sub(context_chars);
    let end = (pos + keyword.len() + context_chars).min(text.len());
    let mut excerpt = text[start..end].to_string();
    if start > 0 {
        excerpt = format!("...{excerpt}");
    }
    if end < text.len() {
        excerpt = format!("{excerpt}...");
    }
    excerpt
}

// -- MCP server main loop

pub fn run_mcp_server(timeout: u64) -> Result<()> {
    let pw = Playwright::spawn(timeout)?;
    let state = Mutex::new(State {
        pw,
        results: HashMap::new(),
    });

    let stdin = io::stdin();
    let stdout = io::stdout();

    eprintln!("[imoduru-mcp] server ready");

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = err_response(Value::Null, -32700, &format!("parse error: {e}"));
                writeln!(stdout.lock(), "{}", serde_json::to_string(&resp)?)?;
                continue;
            }
        };

        let id = req.id.unwrap_or(Value::Null);
        let _ = req.jsonrpc; // acknowledge

        let resp = match req.method.as_str() {
            "initialize" => ok_response(
                id,
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": "imoduru",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            ),

            "notifications/initialized" => continue, // no response needed

            "tools/list" => ok_response(id, tool_definitions()),

            "tools/call" => {
                let tool_name = req.params["name"].as_str().unwrap_or("");
                let args = &req.params["arguments"];

                match tool_name {
                    "crawl" => match handle_crawl(&state, args) {
                        Ok(v) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v)? }] }),
                        ),
                        Err(e) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true }),
                        ),
                    },
                    "query" => match handle_query(&state, args) {
                        Ok(v) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v)? }] }),
                        ),
                        Err(e) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true }),
                        ),
                    },
                    "list_pages" => match handle_list_pages(&state) {
                        Ok(v) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v)? }] }),
                        ),
                        Err(e) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true }),
                        ),
                    },
                    "get_page" => match handle_get_page(&state, args) {
                        Ok(v) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v)? }] }),
                        ),
                        Err(e) => ok_response(
                            id,
                            json!({ "content": [{ "type": "text", "text": format!("Error: {e}") }], "isError": true }),
                        ),
                    },
                    _ => err_response(id, -32601, &format!("unknown tool: {tool_name}")),
                }
            }

            _ => err_response(id, -32601, &format!("unknown method: {}", req.method)),
        };

        writeln!(stdout.lock(), "{}", serde_json::to_string(&resp)?)?;
        stdout.lock().flush()?;
    }

    // Cleanup
    state.lock().unwrap().pw.shutdown()?;
    Ok(())
}
