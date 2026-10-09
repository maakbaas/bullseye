// Builds the strings review page: every text of src/strings.js with a screenshot of the window where
// it is shown, the text boxed in red, an edit box, and the whole file kept up to date with the edits,
// to paste back over src/strings.js.
//
//   node scripts/strings-review/build.mjs            all scenes
//   node scripts/strings-review/build.mjs 3-10       only scenes 3 to 10 (to try a scene out)
//   node scripts/strings-review/build.mjs --page     only the page, with the last build's pictures
//
// How: the real window (src/index.html) runs in Playwright's Chromium at its smallest size, 456 × 684,
// with a stand-in backend (`mock.js`), in each state of `scenes.mjs`. Every text is marked invisibly
// with its number (`mark.js`), so where a text is drawn is known exactly, not guessed from its words.
// For each text the best place found is kept: drawn as text first, then a field's value, then an
// attribute (a placeholder, a label only screen readers read, a tooltip not shown) on its element.
//
// Writes scripts/strings-review/out/strings-review.html, one file with its pictures in it.

import { createServer } from "node:http";
import { createRequire } from "node:module";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readStrings } from "./strings-file.mjs";
import { scenes } from "./scenes.mjs";
import { reviewPage } from "./page.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(HERE, "..", "..");
const SRC = join(ROOT, "src");
const OUT = join(HERE, "out");
const WIDTH = 456;
const HEIGHT = 684;

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

// src/ over http, as the window loads it.
function serve() {
  const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".woff": "font/woff", ".svg": "image/svg+xml", ".png": "image/png", ".ico": "image/x-icon" };
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
  return new Promise((done) => server.listen(0, "127.0.0.1", () => done(server)));
}

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

// One step of a scene (`scenes.mjs` says which there are).
async function step(tab, s) {
  if (s.click) await tab.locator(s.click).first().click({ timeout: 5000 });
  else if (s.hover) await tab.locator(s.hover).first().hover({ timeout: 5000 });
  else if (s.fill) await tab.locator(s.fill[0]).first().fill(s.fill[1], { timeout: 5000 });
  else if (s.type) await tab.locator(s.type[0]).first().pressSequentially(s.type[1], { timeout: 5000 });
  else if (s.press) await tab.keyboard.press(s.press);
  else if (s.pick) {
    await tab.locator(s.pick[0]).first().click({ timeout: 5000 });
    await sleep(300);
    await tab.locator(s.pick[0]).first().locator(`cds-dropdown-item[value="${s.pick[1]}"]`).first().click({ timeout: 5000 });
  } else if (s.open) await tab.locator(s.open).first().click({ timeout: 5000 });
  else if (s.eval) await tab.evaluate(`(async () => { ${s.eval} })()`);
  else if (s.scroll) await tab.evaluate((selector) => document.querySelector(selector)?.scrollIntoView({ block: "start", behavior: "instant" }), s.scroll);
  else if (s.wait) return sleep(s.wait);
  else throw new Error(`unknown step ${JSON.stringify(s)}`);
  await sleep(s.after ?? 350);
}

async function main() {
  const { text: fileText, entries } = readStrings(join(SRC, "strings.js"));
  if (process.argv[2] === "--page") return pageOnly(fileText, entries);
  const [from, to] = (process.argv[2] || "").split("-").map(Number);
  const chosen = scenes.map((scene, n) => ({ ...scene, n })).filter((scene) => !process.argv[2] || (scene.n >= from && scene.n <= (to ?? from)));
  const { chromium } = playwright();
  const server = await serve();
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch();
  const mark = readFileSync(join(HERE, "mark.js"), "utf8");
  const mock = readFileSync(join(HERE, "mock.js"), "utf8");

  const best = new Map(); // text number -> { q, shot, rect, how, scene }
  const shots = []; // png buffers
  const problems = [];

  for (const scene of chosen) {
    const context = await browser.newContext({ viewport: { width: WIDTH, height: HEIGHT }, colorScheme: "light", deviceScaleFactor: 1 });
    const page = await context.newPage();
    page.on("pageerror", (e) => problems.push(`${scene.n} ${scene.title}: page error: ${e.message}`));
    // A text the page asks for that strings.js does not have (`t` says so in the console).
    page.on("console", (m) => m.type() === "error" && m.text().includes("strings.js has no text") && problems.push(`${scene.n} ${scene.title}: ${m.text()}`));
    await page.addInitScript(mark);
    await page.addInitScript(`window.__SETUP = ${JSON.stringify(scene.setup || "")};`);
    await page.addInitScript(mock);
    try {
      await page.goto(`${base}/index.html${scene.hash || "#/"}`);
      await page.waitForSelector("body.ready", { timeout: 10000 });
      await sleep(700);
      // The page's order of texts must be the file's, or the numbers name the wrong texts.
      if (scene === chosen[0]) {
        const paths = await page.evaluate(() => window.__paths);
        if (paths.join("\n") !== entries.map((e) => e.path).join("\n")) throw new Error("strings.js read in another order than the page reads it");
      }
      for (const s of scene.steps || []) await step(page, s);
      // Nothing left hovered or focused by the steps (a tooltip would show), unless the scene wants it.
      if (!scene.keep) {
        await page.mouse.move(2, HEIGHT - 2);
        await page.evaluate(() => !document.getElementById("confirm").open && document.activeElement?.blur());
      }
      await sleep(scene.settle ?? 500);
      await capture(page, scene, best, shots);
    } catch (e) {
      problems.push(`${scene.n} ${scene.title}: ${e.message.split("\n")[0]}`);
    }
    await context.close();
    process.stdout.write(`${scene.n} ${scene.title}: ${best.size} of ${entries.length} texts\n`);
  }
  await browser.close();
  server.close();

  mkdirSync(OUT, { recursive: true });
  const used = [...new Set([...best.values()].map((b) => b.shot))];
  const images = Object.fromEntries(used.map((n) => [n, `data:image/png;base64,${shots[n].toString("base64")}`]));
  const items = entries.map((entry, id) => ({ id, path: entry.path, start: entry.start, end: entry.end, value: entry.value, place: best.get(id) || null, note: noteOf(entry) }));
  writeFileSync(join(OUT, "strings-review.html"), reviewPage({ fileText, items, images }));
  const missing = items.filter((item) => !item.place && item.value !== "");
  console.log(`\n${best.size} of ${entries.length} texts placed; ${used.length} pictures`);
  if (missing.length) console.log(`Not placed:\n  ${missing.map((m) => m.path).join("\n  ")}`);
  if (problems.length) console.log(`\nProblems:\n  ${problems.join("\n  ")}`);
  console.log(`\n${join(OUT, "strings-review.html")}`);
}

// The texts this scene shows better than any scene before it: brought into view one after another
// (the page, or the dialog's body, scrolled), with a screenshot whenever one is in view.
async function capture(page, scene, best, shots) {
  const better = (place) => !best.has(place.id) || place.q < best.get(place.id).q;
  let places = await page.evaluate(() => window.__scan());
  // Per text, its best kind of place in this scene; only those are wanted.
  const wanted = new Map();
  for (const place of places) if (better(place) && (!wanted.has(place.id) || place.q < wanted.get(place.id))) wanted.set(place.id, place.q);
  for (let round = 0; wanted.size && round < 12; round++) {
    places = await page.evaluate(() => window.__scan());
    // The smallest place of a text first: a field rather than the component around it.
    const inView = places.filter((p) => wanted.get(p.id) === p.q && p.inView).sort((a, b) => a.rect.w * a.rect.h - b.rect.w * b.rect.h);
    if (inView.length) {
      const shot = shots.push(await page.screenshot()) - 1;
      for (const place of inView) {
        if (!wanted.has(place.id)) continue;
        best.set(place.id, { q: place.q, how: place.how, rects: place.rects, shot, scene: scene.title });
        wanted.delete(place.id);
      }
    }
    // The next one that is not in view, brought there; one that cannot be is taken as it is.
    const next = places.find((p) => wanted.get(p.id) === p.q && !p.inView);
    if (!next) break;
    await page.evaluate((key) => window.__reveal(key), next.key);
    await sleep(250);
    const again = (await page.evaluate(() => window.__scan())).find((p) => p.key === next.key);
    if (again && !again.inView && again.visible) {
      const shot = shots.push(await page.screenshot()) - 1;
      best.set(again.id, { q: again.q + 0.5, how: again.how, rects: again.rects, shot, scene: scene.title, partly: true });
      wanted.delete(again.id);
    } else if (!again || !again.visible) wanted.delete(next.id);
  }
}

// The page again with the pictures of the last build (`--page`): for a change to the page itself, or to
// strings.js (its texts are matched to their pictures by key).
function pageOnly(fileText, entries) {
  const file = join(OUT, "strings-review.html");
  const html = readFileSync(file, "utf8");
  const last = JSON.parse(html.match(/<script id="data" type="application\/json">([\s\S]*?)<\/script>/)[1]);
  const places = new Map(last.items.map((item) => [item.path, item.place]));
  const items = entries.map((entry, id) => ({ id, path: entry.path, start: entry.start, end: entry.end, value: entry.value, place: places.get(entry.path) || null, note: noteOf(entry) }));
  writeFileSync(file, reviewPage({ fileText, items, images: last.images }));
  console.log(`${file}: ${items.filter((i) => i.place).length} of ${items.length} texts with a picture, from the last build`);
}

// Where a text that no scene shows is used, for the page to say: the CLI only, or nowhere.
const SOURCES = (() => {
  const read = (dir, out = []) => {
    for (const name of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, name.name);
      if (name.isDirectory()) read(path, out);
      else if (/\.(rs|js)$/.test(name.name)) out.push([path, readFileSync(path, "utf8")]);
    }
    return out;
  };
  return { js: read(join(SRC, "js")), rs: read(join(ROOT, "src-tauri", "src")) };
})();
// Texts that are never drawn inside the page, and where they are seen instead.
const ELSEWHERE = {
  "app.name": "The tray icon's tooltip (lib.rs) and the page's own title: not drawn inside the window. The title bar's name is set in ui.rs.",
};
function noteOf(entry) {
  if (ELSEWHERE[entry.path]) return ELSEWHERE[entry.path];
  if (entry.value === "") return "Empty: nothing is shown for it.";
  const quoted = `"${entry.path}"`;
  const inJs = SOURCES.js.some(([, text]) => text.includes(quoted));
  const inRs = SOURCES.rs.some(([path, text]) => text.includes(quoted) && !path.includes("tests"));
  if (inRs && !inJs) return "Used by the command line and the service (strings.rs), not the window.";
  return "";
}


main().catch((e) => {
  console.error(e);
  process.exit(1);
});
