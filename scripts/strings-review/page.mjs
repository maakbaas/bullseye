// The review page (`build.mjs` writes it): one card per text of strings.js with its screenshot, the
// text boxed in red, and a box to edit it; beside them the whole file, rebuilt as you type, to copy
// back over src/strings.js. Edits are kept in the browser (localStorage) until reset, so closing the
// page loses nothing. One file: pictures, data and code are all in it.

// The comment lines right above a text in the file: what its placeholders stand for, where it is used.
function commentAbove(fileText, start) {
  const lines = fileText.slice(0, start).split("\n");
  lines.pop(); // the text's own line
  const out = [];
  while (lines.length && /^\s*\/\//.test(lines[lines.length - 1])) out.unshift(lines.pop().replace(/^\s*\/\/\s?/, ""));
  return out.join(" ");
}

const HOW = {
  text: "",
  value: "The value already in a field.",
  hidden: "Not drawn here: a tooltip (shown on hover) or a label only screen readers read. Boxed: what it belongs to.",
  "attr:placeholder": "A placeholder: shown in the field while it is empty.",
  "attr:aria-label": "Read by screen readers (aria-label), not shown. Boxed: what it names.",
  "attr:title": "A tooltip of the browser's own (title), shown on hover.",
};
const howOf = (place) => {
  if (!place) return "";
  const said = HOW[place.how] ?? `In the ${place.how.replace("attr:", "")} attribute of the boxed element, not drawn as text.`;
  const extra = [place.q >= 3 ? "Shown behind the open dialog." : "", place.partly ? "Partly out of view." : ""].filter(Boolean).join(" ");
  return [said, extra].filter(Boolean).join(" ");
};

export function reviewPage({ fileText, items, images }) {
  const data = {
    fileText,
    images,
    items: items.map((item) => ({ ...item, comment: commentAbove(fileText, item.start), how: howOf(item.place) })),
  };
  const json = JSON.stringify(data).replace(/</g, "\\u003c");
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Bullseye strings review</title>
<style>
  :root {
    --bg: #f4f4f4; --layer: #ffffff; --line: #e0e0e0; --strong: #8d8d8d; --text: #161616; --muted: #525252;
    --accent: #0f62fe; --error: #da1e28; --warn: #8e6a00; --changed: #0f62fe; --mono: "IBM Plex Mono", Consolas, monospace;
    --done: #defbe6; --done-line: #a7f0ba; --ok: #198038;
    color-scheme: light dark;
  }
  @media (prefers-color-scheme: dark) {
    :root { --bg: #161616; --layer: #262626; --line: #393939; --strong: #6f6f6f; --text: #f4f4f4; --muted: #c6c6c6; --accent: #78a9ff; --error: #fa4d56; --warn: #f1c21b; --changed: #78a9ff; --done: #044317; --done-line: #0e6027; --ok: #24a148; }
  }
  * { box-sizing: border-box; }
  body { margin: 0; font: 14px/1.45 "IBM Plex Sans", "Segoe UI", system-ui, sans-serif; background: var(--bg); color: var(--text); }
  header { position: sticky; top: 0; z-index: 5; background: var(--layer); border-bottom: 1px solid var(--line); padding: 12px 16px; display: flex; flex-wrap: wrap; gap: 8px 16px; align-items: center; }
  header h1 { font-size: 18px; margin: 0; font-weight: 600; }
  header .count { color: var(--muted); }
  header input[type=search] { font: inherit; padding: 6px 10px; border: 1px solid var(--strong); background: var(--bg); color: var(--text); min-width: 220px; flex: 1 1 220px; max-width: 420px; }
  header select { font: inherit; padding: 4px 6px; border: 1px solid var(--strong); background: var(--bg); color: var(--text); }
  header label { display: inline-flex; gap: 6px; align-items: center; color: var(--muted); }
  .layout { display: grid; grid-template-columns: minmax(0, 1fr) minmax(360px, 34vw); gap: 12px; padding: 12px; align-items: start; }
  .layout.folded { grid-template-columns: minmax(0, 1fr); }
  .layout.folded aside { display: none; }
  @media (max-width: 1100px) { .layout { grid-template-columns: 1fr; } aside { position: static !important; height: 70vh !important; } }
  #list { display: grid; grid-template-columns: repeat(auto-fill, minmax(250px, 1fr)); gap: 10px; align-items: start; }
  @media (max-width: 560px) { #list { grid-template-columns: 1fr; } }
  .card { background: var(--layer); border: 1px solid var(--line); padding: 8px; display: flex; flex-direction: column; gap: 6px; min-width: 0; font-size: 13px; }
  .card.done { background: var(--done); border-color: var(--done-line); }
  .card.changed { box-shadow: inset 3px 0 0 var(--changed); }
  .card.bad { border-color: var(--error); box-shadow: inset 3px 0 0 var(--error); }
  .path { font: 11px var(--mono); color: var(--muted); overflow-wrap: anywhere; display: flex; gap: 8px; justify-content: space-between; align-items: baseline; }
  .path b { color: var(--text); font-weight: 600; }
  .comment { font-size: 11px; color: var(--muted); font-style: italic; }
  figure { margin: 0; position: relative; width: 228px; max-width: 100%; align-self: center; border: 1px solid var(--line); overflow: hidden; cursor: zoom-in; }
  figure.big { width: 456px; cursor: zoom-out; }
  .card:has(figure.big) { grid-column: span 2; }
  figure img { display: block; width: 100%; height: auto; }
  figure .box { position: absolute; border: 2px solid #da1e28; outline: 1px solid rgba(255,255,255,.9); pointer-events: none; }
  figure.clean .box { display: none; }
  .scene { font-size: 11px; color: var(--muted); }
  .scene em { color: var(--text); font-style: normal; }
  .none { padding: 16px; border: 1px dashed var(--strong); color: var(--muted); font-size: 13px; }
  .edit-row { display: flex; gap: 6px; align-items: stretch; }
  .mark { flex: 0 0 76px; padding: 4px; font-size: 13px; font-weight: 600; line-height: 1.2; }
  .mark.on { background: var(--ok); border-color: var(--ok); color: #fff; }
  textarea.edit { flex: 1; min-width: 0; font: inherit; line-height: 1.4; min-height: 3.2em; padding: 6px; resize: vertical; border: 1px solid var(--strong); background: var(--bg); color: var(--text); }
  textarea.edit:focus, #file:focus, header input:focus { outline: 2px solid var(--accent); outline-offset: -2px; }
  .original { font-size: 11px; color: var(--muted); }
  .original span { font-family: var(--mono); }
  .why { font-size: 12px; color: var(--error); }
  .warn { font-size: 12px; color: var(--warn); }
  .row { display: flex; gap: 8px; align-items: center; justify-content: space-between; flex-wrap: wrap; }
  button { font: inherit; padding: 6px 12px; border: 1px solid var(--strong); background: var(--layer); color: var(--text); cursor: pointer; }
  button.primary { background: var(--accent); border-color: var(--accent); color: #fff; }
  button.small { padding: 2px 8px; font-size: 12px; }
  button:disabled { opacity: .5; cursor: default; }
  aside { position: sticky; top: 72px; height: calc(100vh - 88px); background: var(--layer); border: 1px solid var(--line); padding: 12px; display: flex; flex-direction: column; gap: 8px; }
  aside h2 { margin: 0; font-size: 15px; }
  aside p { margin: 0; color: var(--muted); font-size: 12px; }
  #file { flex: 1; width: 100%; font: 12px/1.45 var(--mono); white-space: pre; overflow: auto; padding: 8px; border: 1px solid var(--strong); background: var(--bg); color: var(--text); resize: none; }
  .hidden { display: none !important; }
</style>
</head>
<body>
<header>
  <h1>Bullseye strings</h1>
  <span class="count"><span id="shown"></span> shown · <span id="changed">0</span> changed · <span id="reviewed">0</span> reviewed · <span id="placed"></span> with a picture</span>
  <input type="search" id="find" placeholder="Filter by key or text">
  <label>Review <select id="review-filter"><option value="">All</option><option value="yes">Reviewed</option><option value="no">Not reviewed</option></select></label>
  <label><input type="checkbox" id="only-changed"> Only changed</label>
  <label><input type="checkbox" id="only-missing"> Only without a picture</label>
  <label><input type="checkbox" id="clean"> Hide boxes</label>
  <button id="fold">Hide strings.js</button>
</header>
<div class="layout">
  <main id="list"></main>
  <aside>
    <div class="row"><h2>src/strings.js</h2><span id="state" class="original"></span></div>
    <p>Rebuilt as you edit; only the edited texts change, comments and layout stay. A reviewed text gets <code>// reviewed</code> at the end of its line (with its key when a line holds more than one text), and a file with those marks starts with them marked. Copy it over src/strings.js. Edits and marks are kept in this browser; Reset all undoes the edits only.</p>
    <div class="row">
      <span>
        <button class="primary" id="copy">Copy</button>
        <button id="download">Download</button>
      </span>
      <button id="reset">Reset all</button>
    </div>
    <textarea id="file" readonly spellcheck="false"></textarea>
  </aside>
</div>
<script id="data" type="application/json">${json}</script>
<script>
(() => {
  const data = JSON.parse(document.getElementById("data").textContent);
  const { fileText, items, images } = data;
  const STORE = "bullseye-strings-review";
  const $ = (id) => document.getElementById(id);
  const el = (tag, props = {}, ...children) => {
    const node = Object.assign(document.createElement(tag), props);
    node.append(...children.filter((c) => c != null));
    return node;
  };
  const placeholders = (s) => [...new Set([...s.matchAll(/\\{(\\w+)\\}/g)].map((m) => m[1]))];

  // Edits, by key: kept only while the file still has the text they were made on.
  let saved = {};
  try { saved = JSON.parse(localStorage.getItem(STORE) || "{}"); } catch {}
  for (const item of items) {
    const edit = saved[item.path];
    item.now = edit && edit.from === item.value ? edit.to : item.value;
  }
  // The lines of the file, and the texts on each (a text by number has both forms on one line).
  const MARK = " // reviewed";
  const lineStarts = [0];
  for (let i = 0; i < fileText.length; i++) if (fileText[i] === "\\n") lineStarts.push(i + 1);
  const byLine = new Map();
  for (const item of items) {
    let n = 0;
    while (n + 1 < lineStarts.length && lineStarts[n + 1] <= item.start) n++;
    item.key = item.path.split(".").pop();
    if (!byLine.has(n)) byLine.set(n, []);
    byLine.get(n).push(item);
  }
  // A line's mark: "// reviewed" for all its texts, "// reviewed: one, other" for those named.
  const lineText = (n) => fileText.slice(lineStarts[n], n + 1 < lineStarts.length ? lineStarts[n + 1] : fileText.length).replace(/\\r?\\n$/, "");
  for (const [n, on] of byLine) {
    const line = lineText(n);
    const at = line.lastIndexOf(MARK);
    const rest = at >= 0 && at > line.lastIndexOf('"') ? line.slice(at + MARK.length) : null;
    const named = rest && rest.startsWith(": ") ? rest.slice(2).split(",").map((k) => k.trim()) : null;
    for (const item of on) item.inFile = rest === "" || (!!named && named.includes(item.key));
  }
  let ticks = {};
  try { ticks = JSON.parse(localStorage.getItem(STORE + "-reviewed") || "{}"); } catch {}
  for (const item of items) item.checked = item.path in ticks ? ticks[item.path] : item.inFile;
  const keep = () => {
    const out = {};
    for (const item of items) if (item.now !== item.value) out[item.path] = { from: item.value, to: item.now };
    const ticked = {};
    for (const item of items) if (item.checked !== item.inFile) ticked[item.path] = item.checked;
    try {
      localStorage.setItem(STORE, JSON.stringify(out));
      localStorage.setItem(STORE + "-reviewed", JSON.stringify(ticked));
    } catch {}
  };

  // What is wrong with an edit: strings.rs reads no escapes, so no double quote, backslash or line
  // break; and every placeholder the text had must stay (the page fills them in).
  const problem = (item) => {
    if (/["\\\\\\r\\n]/.test(item.now)) return 'No " or \\\\ or line breaks: strings.js has no escapes.';
    const lost = placeholders(item.value).filter((p) => !item.now.includes("{" + p + "}"));
    if (lost.length) return "Keep " + lost.map((p) => "{" + p + "}").join(", ") + ": the page fills it in.";
    return "";
  };
  const warning = (item) => {
    const extra = placeholders(item.now).filter((p) => !placeholders(item.value).includes(p));
    return extra.length ? "Unknown placeholder " + extra.map((p) => "{" + p + "}").join(", ") + ": it would show as typed." : "";
  };

  // The file with every edit in its place; a text that cannot be written keeps what it had.
  const file = () => {
    let out = "";
    for (let n = 0; n < lineStarts.length; n++) {
      const from = lineStarts[n];
      const whole = fileText.slice(from, n + 1 < lineStarts.length ? lineStarts[n + 1] : fileText.length);
      const on = byLine.get(n);
      if (!on) {
        out += whole;
        continue;
      }
      const end = whole.endsWith("\\r\\n") ? "\\r\\n" : whole.endsWith("\\n") ? "\\n" : "";
      let line = whole.slice(0, whole.length - end.length);
      // The mark it had goes; it is written again below as ticked now.
      const at = line.lastIndexOf(MARK);
      if (at >= 0 && at > line.lastIndexOf('"')) line = line.slice(0, at);
      // Right to left, so a text's new length does not move the next one's place.
      for (const item of [...on].sort((a, b) => b.start - a.start)) {
        line = line.slice(0, item.start - from) + (problem(item) ? item.value : item.now) + line.slice(item.end - from);
      }
      const checked = on.filter((item) => item.checked);
      if (checked.length) line += MARK + (checked.length === on.length ? "" : ": " + checked.map((item) => item.key).join(", "));
      out += line + end;
    }
    return out;
  };

  const cards = [];
  for (const item of items) {
    const place = item.place;
    let figure;
    if (place) {
      const img = el("img", { src: images[place.shot], alt: "", loading: "lazy", width: 456, height: 684 });
      // A box per line the text is drawn on.
      const pad = 3;
      const boxes = place.rects.map((r) => {
        const box = el("div", { className: "box" });
        Object.assign(box.style, {
          left: ((r.x - pad) / 456) * 100 + "%",
          top: ((r.y - pad) / 684) * 100 + "%",
          width: ((r.w + pad * 2) / 456) * 100 + "%",
          height: ((r.h + pad * 2) / 684) * 100 + "%",
        });
        return box;
      });
      figure = el("figure", {}, img, ...boxes);
    } else {
      figure = el("div", { className: "none", textContent: item.note || "Not found in any scene of the window." });
    }
    const edit = el("textarea", { className: "edit", value: item.now, spellcheck: true, rows: Math.min(6, Math.ceil(item.value.length / 60) + 1) });
    const why = el("div", { className: "why hidden" });
    const warn = el("div", { className: "warn hidden" });
    const undo = el("button", { className: "small", textContent: "Undo" });
    const mark = el("button", { className: "mark", title: "Reviewed or not: a mark in the file" });
    mark.addEventListener("click", () => {
      item.checked = !item.checked;
      show();
      update();
    });
    if (figure.tagName === "FIGURE") figure.addEventListener("click", () => figure.classList.toggle("big"));
    const card = el(
      "section",
      { className: "card" },
      el("div", { className: "path" }, el("b", { textContent: item.path }), undo),
      item.comment ? el("div", { className: "comment", textContent: item.comment }) : null,
      figure,
      place ? el("div", { className: "scene" }, el("em", { textContent: place.scene }), item.how ? " · " + item.how : "") : null,
      el("div", { className: "edit-row" }, edit, mark),
      why,
      warn,
      el("div", { className: "original" }, "Was: ", el("span", { textContent: item.value === "" ? "(empty)" : item.value })),
    );
    const show = () => {
      const bad = problem(item);
      const odd = warning(item);
      card.classList.toggle("changed", item.now !== item.value);
      // Green once it is looked at: reviewed, or edited.
      card.classList.toggle("done", item.checked || item.now !== item.value);
      mark.classList.toggle("on", item.checked);
      mark.textContent = item.checked ? "✓ Reviewed" : "Mark reviewed";
      card.classList.toggle("bad", !!bad);
      why.textContent = bad;
      why.classList.toggle("hidden", !bad);
      warn.textContent = odd;
      warn.classList.toggle("hidden", !odd);
      undo.disabled = item.now === item.value;
    };
    edit.addEventListener("input", () => {
      item.now = edit.value;
      // An edited text has been looked at: reviewed (Undo leaves the tick as it is).
      if (item.now !== item.value && !item.checked) {
        item.checked = true;
      }
      show();
      update();
    });
    undo.addEventListener("click", () => {
      item.now = item.value;
      edit.value = item.value;
      show();
      update();
    });
    show();
    cards.push({ item, card, show, edit });
    $("list").append(card);
  }

  const update = () => {
    $("file").value = file();
    const changed = items.filter((i) => i.now !== i.value).length;
    const bad = items.filter((i) => problem(i)).length;
    $("changed").textContent = changed;
    $("reviewed").textContent = items.filter((i) => i.checked).length;
    $("state").textContent = bad ? bad + " edit" + (bad === 1 ? "" : "s") + " left out until fixed" : "";
    $("state").className = bad ? "why" : "original";
    keep();
    filter();
  };
  const filter = () => {
    const words = $("find").value.trim().toLowerCase();
    let shown = 0;
    for (const { item, card } of cards) {
      const hit =
        (!words || item.path.toLowerCase().includes(words) || item.now.toLowerCase().includes(words) || item.value.toLowerCase().includes(words)) &&
        (!$("only-changed").checked || item.now !== item.value) &&
        (!$("review-filter").value || item.checked === ($("review-filter").value === "yes")) &&
        (!$("only-missing").checked || !item.place);
      card.classList.toggle("hidden", !hit);
      if (hit) shown++;
    }
    $("shown").textContent = shown + " of " + items.length;
  };
  $("placed").textContent = items.filter((i) => i.place).length;
  const fold = (folded) => {
    document.querySelector(".layout").classList.toggle("folded", folded);
    $("fold").textContent = folded ? "Show strings.js" : "Hide strings.js";
    try { localStorage.setItem(STORE + "-folded", folded ? "1" : ""); } catch {}
  };
  let folded = false;
  try { folded = localStorage.getItem(STORE + "-folded") === "1"; } catch {}
  fold(folded);
  $("fold").addEventListener("click", () => fold(!document.querySelector(".layout").classList.contains("folded")));
  for (const id of ["find", "only-changed", "only-missing", "review-filter"]) $(id).addEventListener("input", filter);
  $("clean").addEventListener("change", () => document.querySelectorAll("figure").forEach((f) => f.classList.toggle("clean", $("clean").checked)));
  $("copy").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText($("file").value); }
    catch { $("file").select(); document.execCommand("copy"); }
    $("copy").textContent = "Copied";
    setTimeout(() => ($("copy").textContent = "Copy"), 1500);
  });
  $("download").addEventListener("click", () => {
    const a = el("a", { href: URL.createObjectURL(new Blob([$("file").value], { type: "text/javascript" })), download: "strings.js" });
    a.click();
    URL.revokeObjectURL(a.href);
  });
  $("reset").addEventListener("click", () => {
    if (!confirm("Undo every edit?")) return;
    for (const c of cards) {
      c.item.now = c.item.value;
      c.edit.value = c.item.value;
      c.show();
    }
    update();
  });
  update();
})();
</script>
</body>
</html>
`;
}
