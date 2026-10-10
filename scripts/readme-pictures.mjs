// Takes the README's three pictures (docs/sessions.png, docs/target-editor.png, docs/condition-editor.png):
// the real window (src/) in Playwright's Chromium, light theme, 902 px wide with a 1 px frame, with a
// stand-in backend holding the sample data below. The rows' status words are built from strings.js as
// the service builds them (`status.rs`), so the pictures follow the texts.
//
//   node scripts/readme-pictures.mjs

import { createServer } from "node:http";
import { createRequire } from "node:module";
import { existsSync, readFileSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(ROOT, "src");

// Playwright: the project's own if it has one, else the one that comes with playwright-cli.
function playwright() {
  const places = [ROOT, join(process.env.APPDATA || "", "npm", "node_modules", "@playwright", "cli", "node_modules"), "C:/Program Files/nodejs/node_modules/@playwright/cli/node_modules"];
  for (const place of places) {
    for (const name of ["playwright", "playwright-core"]) {
      try {
        return createRequire(join(place, "x.js"))(name);
      } catch {}
    }
  }
  throw new Error("Playwright not found: npm install -g @playwright/cli, or npm i -D playwright");
}

// The stand-in for the Tauri backend, run in the page before its scripts.
const STAND_IN = () => {
  const t = (path, values = {}) => {
    const text = path.split(".").reduce((node, key) => node?.[key], window.STRINGS) ?? path;
    return text.replace(/\{(\w+)\}/g, (whole, key) => (key in values ? String(values[key]) : whole));
  };
  const week = (from, to) => [0, 1, 2, 3, 4, 5, 6].map((d) => (d < 5 ? [[from, to]] : []));
  const target = (id, name, filters, allowed, actions = {}) => ({
    id, name, filters, allowed, programs: [], store_apps: [], titles: [], allowed_titles: [],
    actions: { window_titles_minimize: false, obsidian_web_viewer: true, steam_browser: false, ...actions },
  });
  const overview = () => ({
    targets: [
      target("work", "Everything but work", ["*"], ["docs.rs", "github.com", "mail.google.com"]),
      target("news", "News", ["news.ycombinator.com", "nos.nl", "nu.nl"], []),
      target("social", "Social media", ["instagram.com", "reddit.com", "tiktok.com", "x.com", "youtube.com"], ["music.youtube.com"]),
    ],
    conditions: [
      { id: "key", name: "Key out", type: "flash_drive", serial: "4C530001", drive: "SanDisk Cruzer" },
      { id: "hours", name: "Work hours", type: "schedule", days: week(510, 1050) },
    ],
    sessions: [
      { id: "deep", name: "Deep work", targets: ["work"], condition: "hours", allowance: null, lock: { method: "timed", kind: "for", value: "2h" } },
      {
        id: "soc", name: "Social", targets: ["social", "news"], condition: null,
        allowance: { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" },
        lock: { method: "condition", condition: "key" },
      },
    ],
    running: [{
      id: "deep", name: "Deep work", targets: [], remaining: 4980, remaining_words: "83 minutes", lock_condition: null, until_restart: false,
      lock_text: null, stop_delay: null, stop_delay_words: null, stop_remaining: null, stop_remaining_words: null, password: false,
      locked: true, condition: { type: "schedule", days: week(510, 1050) }, enforcing: true, condition_change: 14400,
      condition_change_words: "4 hours", lock_change: null, lock_change_words: null, allowance: null,
    }],
    unlock: { delay: 0, delay_words: "0 seconds", remaining: 0, remaining_words: "0 seconds", way: null, way_open: false, way_left: null, way_left_words: null },
    clock_synced: true,
    status: [
      {
        id: "deep", state: "locked",
        condition: { text: t("sessions.now.activeFor", { change: "4 hours" }) },
        allowance: { text: t("sessions.now.neverPaused") },
        lock: { text: t("sessions.now.timeLeft", { left: "83 minutes" }) },
      },
      {
        id: "soc", state: "not_running",
        condition: { text: t("sessions.now.always") },
        allowance: { text: t("sessions.now.budget", { time: "30 minutes" }) },
        lock: { text: t("sessions.now.by"), condition: { id: "key", name: "Key out" } },
      },
    ],
  });
  const answers = {
    status: overview,
    describe_schedule: () => "Mon-Fri 08:30-17:30",
    describe_duration: ({ text }) => text,
    describe_until: () => null,
    describe_allowance: () => ({ words: "30 minutes a day" }),
    deletion_fallout: () => ({ without_it: [], always: [], unlocked: [], shrinks: [], removed: [] }),
    installed_apps: () => [],
    connected_drives: () => [],
  };
  window.__TAURI__ = {
    core: { invoke: async (command, args) => (answers[command] ? answers[command](args || {}) : null) },
    event: { listen: async () => () => {} },
  };
};

const PICTURES = [
  ["sessions", "#/", 762],
  ["target-editor", "#/targets/edit/social", 902],
  ["condition-editor", "#/conditions/edit/hours", 902],
];

const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".woff": "font/woff", ".svg": "image/svg+xml", ".png": "image/png" };
const server = createServer((request, response) => {
  const path = decodeURIComponent(new URL(request.url, "http://x").pathname);
  const file = join(SRC, path.endsWith("/") ? `${path}index.html` : path);
  if (!file.startsWith(SRC) || !existsSync(file)) {
    response.writeHead(404);
    return response.end();
  }
  response.writeHead(200, { "content-type": types[extname(file)] || "application/octet-stream" });
  response.end(readFileSync(file));
});
await new Promise((done) => server.listen(0, "127.0.0.1", done));

const browser = await playwright().chromium.launch();
const page = await browser.newPage({ colorScheme: "light" });
await page.addInitScript(STAND_IN);
for (const [name, hash, height] of PICTURES) {
  await page.setViewportSize({ width: 902, height });
  await page.goto(`http://127.0.0.1:${server.address().port}/index.html${hash}`);
  await page.waitForSelector("body.ready");
  // The frame the README's pictures have.
  await page.addStyleTag({ content: "html { border: 1px solid #c6c6c6; box-sizing: border-box; height: 100%; }" });
  await page.waitForTimeout(1500);
  await page.screenshot({ path: join(ROOT, "docs", `${name}.png`) });
  console.log(`docs/${name}.png`);
}
await browser.close();
server.close();
