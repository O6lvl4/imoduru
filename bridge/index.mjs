// imoduru-bridge: Playwright fetch server with stealth, fingerprint rotation, and proxy support
// Protocol: ndjson over stdin/stdout
//
// Methods:
//   configure: { "method": "configure", "params": { "proxy": {...}, "stealth": true, "fingerprint": "rotate" } }
//   fetch:     { "id": N, "method": "fetch", "params": { "url": "...", "timeout": 30000 } }
//   shutdown:  { "id": 0, "method": "shutdown" }

import { chromium } from "playwright";
import { createInterface } from "readline";

// ── Browser Fingerprint Profiles ──────────────────────────────────

const FINGERPRINTS = [
  {
    name: "chrome-mac",
    userAgent:
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    viewport: { width: 1920, height: 1080 },
    platform: "MacIntel",
    vendor: "Google Inc.",
    renderer: "ANGLE (Apple, ANGLE Metal Renderer: Apple M1 Pro, Unspecified Version)",
    languages: ["ja-JP", "ja", "en-US", "en"],
    hardwareConcurrency: 10,
    deviceMemory: 16,
  },
  {
    name: "chrome-win",
    userAgent:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    viewport: { width: 1920, height: 1080 },
    platform: "Win32",
    vendor: "Google Inc.",
    renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 3060 Direct3D11 vs_5_0 ps_5_0)",
    languages: ["ja", "en-US", "en"],
    hardwareConcurrency: 12,
    deviceMemory: 32,
  },
  {
    name: "chrome-win-2",
    userAgent:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36",
    viewport: { width: 2560, height: 1440 },
    platform: "Win32",
    vendor: "Google Inc.",
    renderer: "ANGLE (Intel, Intel(R) UHD Graphics 770 Direct3D11 vs_5_0 ps_5_0)",
    languages: ["en-US", "en", "ja"],
    hardwareConcurrency: 16,
    deviceMemory: 16,
  },
  {
    name: "edge-win",
    userAgent:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0",
    viewport: { width: 1920, height: 1080 },
    platform: "Win32",
    vendor: "Google Inc.",
    renderer: "ANGLE (NVIDIA, NVIDIA GeForce RTX 4070 Direct3D11 vs_5_0 ps_5_0)",
    languages: ["ja-JP", "ja", "en-US"],
    hardwareConcurrency: 8,
    deviceMemory: 16,
  },
  {
    name: "chrome-linux",
    userAgent:
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    viewport: { width: 1920, height: 1080 },
    platform: "Linux x86_64",
    vendor: "Google Inc.",
    renderer: "ANGLE (Mesa, llvmpipe, OpenGL 4.5)",
    languages: ["en-US", "en"],
    hardwareConcurrency: 4,
    deviceMemory: 8,
  },
];

function pickFingerprint(mode) {
  if (mode === "rotate") {
    return FINGERPRINTS[Math.floor(Math.random() * FINGERPRINTS.length)];
  }
  return FINGERPRINTS[0]; // default: chrome-mac
}

// ── Stealth Init Script ───────────────────────────────────────────

function buildStealthScript(fp) {
  return `
    // 1. navigator.webdriver
    Object.defineProperty(navigator, 'webdriver', { get: () => false });

    // 2. chrome runtime
    if (!window.chrome) window.chrome = {};
    window.chrome.runtime = {
      connect: () => {},
      sendMessage: () => {},
      id: undefined,
    };
    window.chrome.loadTimes = function() {
      return {
        commitLoadTime: Date.now() / 1000,
        connectionInfo: "h2",
        finishDocumentLoadTime: Date.now() / 1000 + 0.1,
        finishLoadTime: Date.now() / 1000 + 0.2,
        firstPaintAfterLoadTime: 0,
        firstPaintTime: Date.now() / 1000 + 0.05,
        navigationType: "Other",
        npnNegotiatedProtocol: "h2",
        requestTime: Date.now() / 1000 - 0.5,
        startLoadTime: Date.now() / 1000 - 0.4,
        wasAlternateProtocolAvailable: false,
        wasFetchedViaSpdy: true,
        wasNpnNegotiated: true,
      };
    };
    window.chrome.csi = function() {
      return { onloadT: Date.now(), pageT: Date.now() / 1000, startE: Date.now(), tran: 15 };
    };

    // 3. navigator properties
    Object.defineProperty(navigator, 'platform', { get: () => '${fp.platform}' });
    Object.defineProperty(navigator, 'vendor', { get: () => '${fp.vendor}' });
    Object.defineProperty(navigator, 'hardwareConcurrency', { get: () => ${fp.hardwareConcurrency} });
    Object.defineProperty(navigator, 'deviceMemory', { get: () => ${fp.deviceMemory} });
    Object.defineProperty(navigator, 'languages', { get: () => ${JSON.stringify(fp.languages)} });

    // 4. plugins (mimic Chrome)
    Object.defineProperty(navigator, 'plugins', {
      get: () => {
        const plugins = [
          { name: 'Chrome PDF Plugin', filename: 'internal-pdf-viewer', description: 'Portable Document Format' },
          { name: 'Chrome PDF Viewer', filename: 'mhjfbmdgcfjbbpaeojofohoefgiehjai', description: '' },
          { name: 'Native Client', filename: 'internal-nacl-plugin', description: '' },
        ];
        plugins.length = 3;
        return plugins;
      },
    });

    // 5. permissions query
    const origQuery = window.navigator.permissions.query.bind(window.navigator.permissions);
    window.navigator.permissions.query = (params) => {
      if (params.name === 'notifications') {
        return Promise.resolve({ state: Notification.permission });
      }
      return origQuery(params);
    };

    // 6. WebGL vendor/renderer spoofing
    const getParameterOrig = WebGLRenderingContext.prototype.getParameter;
    WebGLRenderingContext.prototype.getParameter = function(parameter) {
      if (parameter === 37445) return '${fp.vendor}';       // UNMASKED_VENDOR_WEBGL
      if (parameter === 37446) return '${fp.renderer}';     // UNMASKED_RENDERER_WEBGL
      return getParameterOrig.call(this, parameter);
    };
    if (typeof WebGL2RenderingContext !== 'undefined') {
      const getParameter2Orig = WebGL2RenderingContext.prototype.getParameter;
      WebGL2RenderingContext.prototype.getParameter = function(parameter) {
        if (parameter === 37445) return '${fp.vendor}';
        if (parameter === 37446) return '${fp.renderer}';
        return getParameter2Orig.call(this, parameter);
      };
    }

    // 7. Canvas fingerprint noise
    const origToDataURL = HTMLCanvasElement.prototype.toDataURL;
    HTMLCanvasElement.prototype.toDataURL = function(type) {
      if (this.width > 16 && this.height > 16) {
        const ctx = this.getContext('2d');
        if (ctx) {
          const imageData = ctx.getImageData(0, 0, Math.min(this.width, 4), Math.min(this.height, 4));
          for (let i = 0; i < imageData.data.length; i += 4) {
            imageData.data[i] ^= (${Math.floor(Math.random() * 3)});     // R
            imageData.data[i+1] ^= (${Math.floor(Math.random() * 3)});   // G
          }
          ctx.putImageData(imageData, 0, 0);
        }
      }
      return origToDataURL.call(this, type);
    };

    // 8. Iframe contentWindow
    try {
      Object.defineProperty(HTMLIFrameElement.prototype, 'contentWindow', {
        get: function() {
          return new Proxy(this.contentWindow || window, {
            get: (target, prop) => {
              if (prop === 'chrome') return window.chrome;
              return Reflect.get(target, prop);
            },
          });
        },
      });
    } catch {}

    // 9. Prevent detection via Error stack traces
    const origError = Error;
    Error = function(...args) {
      const err = new origError(...args);
      const stack = err.stack || '';
      err.stack = stack.replace(/playwright|puppeteer|selenium|webdriver/gi, 'chrome');
      return err;
    };
    Error.prototype = origError.prototype;
  `;
}

// ── Cloudflare Challenge Detection ────────────────────────────────

async function handleCloudflareChallenge(page, timeout) {
  const challengeSelectors = [
    "#cf-challenge-running",
    "#challenge-running",
    ".cf-browser-verification",
    'iframe[src*="challenges.cloudflare.com"]',
    "#turnstile-wrapper",
  ];

  for (const sel of challengeSelectors) {
    const el = await page.$(sel);
    if (el) {
      // Cloudflare challenge detected — wait for it to resolve
      try {
        await page.waitForFunction(
          (selectors) => !selectors.some((s) => document.querySelector(s)),
          challengeSelectors,
          { timeout: Math.min(timeout, 15000) }
        );
        // Extra wait for page to settle after challenge
        await page.waitForTimeout(2000);
        return true;
      } catch {
        return false;
      }
    }
  }
  return false;
}

// ── Main ──────────────────────────────────────────────────────────

let config = {
  proxy: null,
  stealth: false,
  fingerprint: "default", // "default" | "rotate"
};

let browser = null;
let context = null;

async function createBrowser() {
  const launchOpts = {
    headless: true,
    args: [
      "--disable-blink-features=AutomationControlled",
      "--disable-dev-shm-usage",
      "--no-sandbox",
      "--disable-infobars",
      "--disable-background-timer-throttling",
      "--disable-popup-blocking",
      "--disable-backgrounding-occluded-windows",
      "--disable-renderer-backgrounding",
      "--disable-component-update",
    ],
  };

  if (config.proxy) {
    launchOpts.proxy = { server: config.proxy.server };
  }

  browser = await chromium.launch(launchOpts);
  await createContext();
}

async function createContext() {
  const fp = pickFingerprint(config.fingerprint);

  const ctxOpts = {
    userAgent: fp.userAgent,
    viewport: fp.viewport,
    locale: fp.languages[0] || "ja-JP",
    timezoneId: "Asia/Tokyo",
    ignoreHTTPSErrors: true,
    extraHTTPHeaders: {
      "Accept-Language": fp.languages.join(","),
    },
  };

  if (config.proxy && config.proxy.username) {
    ctxOpts.httpCredentials = {
      username: config.proxy.username,
      password: config.proxy.password || "",
    };
  }

  context = await browser.newContext(ctxOpts);

  if (config.stealth) {
    await context.addInitScript(buildStealthScript(fp));
  } else {
    // Minimal stealth even without flag
    await context.addInitScript(() => {
      Object.defineProperty(navigator, "webdriver", { get: () => false });
    });
  }
}

function reply(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

async function handleConfigure(params) {
  if (params.proxy) config.proxy = params.proxy;
  if (params.stealth !== undefined) config.stealth = params.stealth;
  if (params.fingerprint) config.fingerprint = params.fingerprint;

  // Recreate browser with new config
  if (browser) {
    await browser.close();
  }
  await createBrowser();

  reply({ id: null, ok: true, configured: true });
}

async function handleFetch(id, params) {
  // Rotate fingerprint per-request if configured
  if (config.fingerprint === "rotate") {
    if (context) await context.close();
    await createContext();
  }

  const page = await context.newPage();
  try {
    const timeout = params.timeout ?? 30000;

    // Block unnecessary resources for speed
    if (!config.stealth) {
      await page.route("**/*.{png,jpg,jpeg,gif,svg,ico,woff,woff2,ttf}", (route) =>
        route.abort()
      );
    }

    const resp = await page.goto(params.url, {
      waitUntil: params.wait_until ?? "domcontentloaded",
      timeout,
    });

    // Handle Cloudflare challenge
    if (config.stealth) {
      await handleCloudflareChallenge(page, timeout);
    }

    if (params.wait_for) {
      await page.waitForSelector(params.wait_for, { timeout });
    }

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

// ── Startup ───────────────────────────────────────────────────────

await createBrowser();

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
    if (browser) await browser.close();
    process.exit(0);
  }

  if (req.method === "configure") {
    await handleConfigure(req.params ?? {});
    return;
  }

  if (req.method === "fetch") {
    await handleFetch(req.id, req.params ?? {});
    return;
  }

  reply({ id: req.id, ok: false, error: `unknown method: ${req.method}` });
});

reply({ id: null, ok: true, ready: true });
