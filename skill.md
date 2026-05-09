---
name: imoduru
description: Crawl a website recursively and answer questions based on its content
user_invocable: true
---

# imoduru — web crawl & Q&A skill

When the user provides a URL and asks to crawl it or asks questions about a website's content:

1. **Crawl the target URL** using imoduru CLI:
   ```bash
   cd /Users/o6lvl4/workspace/github.com/O6lvl4/imoduru
   cargo run --quiet -- crawl "<URL>" --depth <DEPTH> --rate-limit 300 --output /tmp/imoduru-result.json
   ```
   - Default depth: 2 (adjust based on user request)
   - Add `--stealth` if the site might have bot protection
   - Add `--fingerprint rotate` for extra stealth

2. **Read the crawl result**:
   ```bash
   cat /tmp/imoduru-result.json
   ```

3. **Answer the user's question** using the crawled page text as context.
   - Cite specific pages (URL + title) when answering
   - If PDF links are found, mention them as additional resources
   - If the answer isn't in the crawled data, say so and suggest increasing depth

## Examples

- "https://example.com/docs/ のページを全部取得して、APIの使い方を教えて"
  → crawl with depth 2, then answer from collected text

- "このサイトの料金表を探して" (after a prior crawl)
  → search the already-crawled data for pricing info

## Notes

- imoduru respects robots.txt by default
- Rate limiting is 300ms between requests by default (polite crawling)
- Stealth mode available for sites with anti-bot protection
