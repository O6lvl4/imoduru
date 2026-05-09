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
   cargo run --quiet -- crawl "<URL>" --depth <DEPTH> --rate-limit 300 --pdf --output /tmp/imoduru-result.json
   ```
   - Default depth: 2 (adjust based on user request)
   - Always use `--pdf` to extract PDF text (critical info is often in PDFs)
   - Add `--stealth` if the site might have bot protection
   - Add `--fingerprint rotate` for extra stealth

2. **Read the crawl result**:
   ```bash
   cat /tmp/imoduru-result.json
   ```

3. **Answer the user's question** using the crawled page text as context.

## Citation rules (MANDATORY)

Every factual claim in your answer MUST include a source citation. Format:

> [情報] — 出典: ページタイトル (URL)

or for PDF sources:

> [情報] — 出典: PDFファイル名 (URL)

Rules:
- Cite the specific page URL or PDF URL where the information was found
- If info comes from HTML text, cite the page URL
- If info comes from a PDF, cite the PDF URL
- If multiple sources support the same claim, cite all of them
- If the answer is NOT in the crawled data, explicitly say "収集したデータには該当する情報がありません" and suggest the user contact the relevant organization or increase crawl depth
- NEVER present information without a source — if you can't cite it, don't say it
- Distinguish between "explicitly stated" vs "inferred" information

Example answer format:

> **壊れた事務椅子はそのまま持ち込めます。**
>
> - 受入区分: 不燃（リサイクル施設）
>   — 出典: 家庭ごみ受入基準一覧 (https://...863274.pdf)
> - 搬入時は分別した状態で持ち込むこと
>   — 出典: 生活系廃棄物の持ち込みについて (https://...405845.html)
> - 分解の必要なし（受入基準に分解指示の記載なし）
>   — 出典: 同上PDF内の事務椅子の項目に「自宅で使用したものに限る」とのみ記載

## Notes

- imoduru respects robots.txt by default
- Rate limiting is 300ms between requests by default (polite crawling)
- Stealth mode available for sites with anti-bot protection
- PDF extraction may fail on some CJK-heavy PDFs — note this if relevant data might be missing
