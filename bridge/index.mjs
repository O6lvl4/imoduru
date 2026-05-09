// imoduru-bridge: Playwright fetch server
// Reads JSON requests from stdin, returns JSON responses to stdout.
// Protocol: one JSON object per line (ndjson).
//
// Request:  { "id": 1, "method": "fetch", "params": { "url": "...", "stealth": false, "wait_for": null, "timeout": 30000 } }
// Response: { "id": 1, "ok": true, "html": "...", "url": "...", "status": 200 }
// Request:  { "id": 0, "method": "shutdown" }

import { chromium } from "playwright";
import { createInterface } from "readline";

const browser = await chromium.launch({
  headless: true,
  args: [
    "--disable-blink-features=AutomationControlled",
    "--disable-dev-shm-usage",
    "--no-sandbox",
  ],
});

const context = await browser.newContext({
  userAgent:
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
  locale: "ja-JP",
  timezoneId: "Asia/Tokyo",
  viewport: { width: 1920, height: 1080 },
});

// Stealth: hide webdriver flag
await context.addInitScript(() => {
  Object.defineProperty(navigator, "webdriver", { get: () => false });
  // Override permissions query
  const origQuery = window.navigator.permissions.query;
  window.navigator.permissions.query = (params) =>
    params.name === "notifications"
      ? Promise.resolve({ state: Notification.permission })
      : origQuery(params);
});

function reply(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

async function handleFetch(id, params) {
  const page = await context.newPage();
  try {
    const timeout = params.timeout ?? 30000;
    const resp = await page.goto(params.url, {
      waitUntil: params.wait_until ?? "domcontentloaded",
      timeout,
    });
    if (params.wait_for) {
      await page.waitForSelector(params.wait_for, { timeout });
    }
    // Small delay for any lazy-loaded content
    if (params.delay) {
      await page.waitForTimeout(params.delay);
    }
    const html = await page.content();
    const status = resp ? resp.status() : 0;
    const finalUrl = page.url();
    reply({ id, ok: true, html, url: finalUrl, status });
  } catch (err) {
    reply({ id, ok: false, error: err.message });
  } finally {
    await page.close();
  }
}

const rl = createInterface({ input: process.stdin });

rl.on("line", async (line) => {
  let req;
  try {
    req = JSON.parse(line);
  } catch {
    reply({ id: null, ok: false, error: "invalid json" });
    return;
  }

  if (req.method === "shutdown") {
    await browser.close();
    process.exit(0);
  }

  if (req.method === "fetch") {
    await handleFetch(req.id, req.params ?? {});
    return;
  }

  reply({ id: req.id, ok: false, error: `unknown method: ${req.method}` });
});

// Signal ready
reply({ id: null, ok: true, ready: true });
