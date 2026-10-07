// The basics every part of the page uses: texts (strings.js), elements, the service's view of things
// (refreshed every second), the backend's words for times and schedules, and the one dialog.

const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

// A text of src/strings.js by its path, with its {placeholders} filled from `values`; one with
// `one` and `other` is chosen by `values.n`. A missing one shows as its path.
function t(path, values = {}) {
  let text = path.split(".").reduce((node, key) => (node == null ? node : node[key]), window.STRINGS);
  if (text && typeof text === "object" && "other" in text) text = values.n === 1 ? text.one : text.other;
  if (typeof text !== "string") {
    console.error(`strings.js has no text "${path}"`);
    return path;
  }
  return text.replace(/\{(\w+)\}/g, (whole, key) => (key in values ? String(values[key]) : whole));
}
// A name inside a sentence, and several of them.
const quoted = (name) => t("common.quoted", { name });
// Where an item is edited (the same pages as `KINDS[kind].editPath`).
const editHref = (kind, id) => `${{ blocks: "#/edit/", lists: "#/lists/edit/", conditions: "#/conditions/edit/" }[kind]}${encodeURIComponent(id)}`;
const names = (blocks) => joined(blocks.map((b) => quoted(b.name))).join("");

// A text with items in it, as parts: strings and `{ link, href }` (an item by its name, to its
// page). A placeholder's value may be a `ref` or a list of them; the rest is as `t`. Parts are
// plain data, so a tile can tell whether its line changed; `nodesOf` makes the elements.
const ref = (name, href) => ({ link: name, href });
// a, b and c; a and b.
const joined = (items) => items.flatMap((item, at) => (at ? [t(at === items.length - 1 ? "common.and" : "common.separator"), item] : [item]));
function tn(path, values = {}) {
  const found = [];
  const marked = {};
  for (const [key, value] of Object.entries(values)) {
    marked[key] = value !== null && typeof value === "object" ? `\u0000${found.push(value) - 1}\u0000` : value;
  }
  const split = t(path, marked).split("\u0000");
  return split.flatMap((part, at) => (at % 2 ? found[Number(part)] : part ? [part] : []));
}
const nodesOf = (parts) => [].concat(parts).map((p) => (typeof p === "string" ? p : el("cds-link", { href: p.href, inline: true }, p.link)));

// The markup's texts: data-t sets an element's text, data-t-<attribute> an attribute.
function applyStrings() {
  document.title = t("app.name");
  for (const node of document.querySelectorAll("*")) {
    for (const { name, value } of [...node.attributes]) {
      if (name === "data-t") node.textContent = t(value);
      else if (name.startsWith("data-t-")) node.setAttribute(name.slice("data-t-".length), t(value));
    }
  }
}

// ---- Small helpers.

function el(tag, attributes = {}, ...children) {
  const node = document.createElement(tag);
  for (const [name, value] of Object.entries(attributes)) {
    if (value === false || value == null) continue;
    node.setAttribute(name, value === true ? "" : value);
  }
  node.append(...children.filter((c) => c != null));
  return node;
}

// Text that only the backend writes, so the window says exactly what the CLI says: a weekly
// schedule (`Mon-Fri 08:00-17:00`) and a lock's length (`2 hours and 30 minutes`). Asked
// once per value; until the answer is in, a stand-in, and the page is drawn again when it comes.
const texts = new Map();
function described(command, args, standIn) {
  const key = command + JSON.stringify(args);
  if (!texts.has(key)) {
    texts.set(key, standIn);
    invoke(command, args)
      .then((text) => texts.set(key, text))
      .catch(() => texts.set(key, standIn))
      .finally(() => redraw());
  }
  return texts.get(key);
}
const weekText = (days) => described("describe_schedule", { week: { days } }, t("common.weeklyStandIn"));
const durationText = (text) => described("describe_duration", { text: text || "" }, text || "");
// An until lock's end written out (`Tuesday 6 October 2026, 18:00`); as typed until then, or
// once it has passed.
const untilText = (text) => {
  const end = described("describe_until", { text: text || "" }, null);
  return end ? end.when : text || "";
};
// A day as the date picker writes it: 2026-10-06, in local time.
const localDate = (day) => `${day.getFullYear()}-${String(day.getMonth() + 1).padStart(2, "0")}-${String(day.getDate()).padStart(2, "0")}`;

function lines(text) {
  return text.split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
}

const newId = () => (crypto.randomUUID ? crypto.randomUUID() : `i${Date.now()}${Math.random().toString(36).slice(2)}`);

// One message per place; a new one replaces the old.
// An error scrolls into view, also when it is the same as the last (the user tried again), except
// where `scroll` is false (the service-down notice, redrawn every second).
function notify(slot, kind, title, subtitle = "", { closable = true, scroll = true } = {}) {
  const box = $(slot);
  box.replaceChildren();
  if (!title) return;
  const note = el("cds-inline-notification", { kind, title, subtitle, "low-contrast": true, "hide-close-button": !closable });
  box.append(note);
  // An error must not appear out of sight (a tile far down a long page, a form scrolled past its top).
  // Only once the notification has drawn: before that the box is empty and has no height to scroll to.
  if (kind === "error" && scroll) {
    const show = () => requestAnimationFrame(() => note.isConnected && note.scrollIntoView({ block: "center", behavior: "smooth" }));
    if (note.updateComplete) note.updateComplete.then(show);
    else show();
  }
}

// ---- The service's view of things, asked every second while the window is shown.

// What draws the page again when something new is known (`main.js` sets it).
let redraw = () => {};
function whenRefreshed(draw) {
  redraw = draw;
}

let overview = { lists: [], conditions: [], blocks: [], running: [], unlock: { delay: 0, delay_words: "", remaining: 0, remaining_words: "" }, clock_synced: true };
let reachable = true;

async function refresh() {
  try {
    overview = await invoke("block_status");
    if (!reachable) notify("list-notices");
    reachable = true;
  } catch (e) {
    // The service did not answer: say so, and leave the rest as it was.
    reachable = false;
    notify("list-notices", "error", t("common.serviceDown"), String(e), { scroll: false });
  }
  redraw();
}

const runningOf = (id) => overview.running.find((r) => r.id === id);
const conditionOf = (block) => overview.conditions.find((c) => c.id === block.condition);
const listName = (id) => (overview.lists.find((l) => l.id === id) || { name: t("common.gone") }).name;
// The blocks that use a block list or condition, and those of them that run (which hold it).
// (A condition is also used by the blocks that use a combination of it.)
const conditionsOf = (b) => [b.condition, b.lock && b.lock.condition].filter(Boolean);
const membersOf = (conditionId) => (overview.conditions.find((c) => c.id === conditionId) || {}).members || [];
const usersOf = (id) => overview.blocks.filter((b) => b.lists.includes(id) || conditionsOf(b).some((c) => c === id || membersOf(c).includes(id)));
// How a condition is worded wherever one is named (a block's condition line, a lock's line): "drive"
// for a flash drive ("… is unplugged"), "true" for a combination with a drive anywhere in it, and
// "during" for the rest, schedules alone or combined. Works on a definition (its members are ids) and
// on the copy a running block holds (its `rules`).
const driveIn = (c) => c.type === "flash_drive" || c.type === "webhook" || (c.rules || []).some(driveIn) || (c.members || []).some((id) => driveIn(overview.conditions.find((m) => m.id === id) || {}));
const wordingOf = (c) => (c.type === "flash_drive" ? "drive" : driveIn(c) ? "true" : "during");
// (A webhook is worded like a combination with a drive: "active since <name> is true": it has no time.)
const holdersOf = (id) => usersOf(id).filter((b) => runningOf(b.id));
// Those of them that cannot be stopped now: what they use can only get stricter.
const lockedUsersOf = (id) => usersOf(id).filter((b) => runningOf(b.id) && runningOf(b.id).locked);

// Marks a field as wrong (with why) or right again; true when wrong.
function invalid(field, why) {
  field.toggleAttribute("invalid", !!why);
  if (why) field.setAttribute("invalid-text", why);
  return !!why;
}

// The one dialog: a question before anything that cannot be taken back, or a short form (a
// lock's settings, a password). Resolves false for Cancel, the X or Escape; otherwise true,
// or with `fields`, what `read` makes of them: the dialog stays open while that is null (the
// fields then say what is wrong).
let answer = null;
let reader = null;
function ask({ heading, body = "", action, kind, fields = [], read = null }) {
  if (answer) answer(false);
  $("confirm-heading").textContent = heading;
  $("confirm-body").textContent = body;
  $("confirm-body").hidden = !body;
  $("confirm-action").textContent = action;
  $("confirm-action").setAttribute("kind", kind);
  $("confirm-fields").replaceChildren(...fields);
  $("confirm-fields").hidden = fields.length === 0;
  $("confirm").setAttribute("size", fields.length ? "sm" : "xs");
  reader = read;
  $("confirm").open = true;
  // A form starts in its first field; a question on Cancel.
  // (Not one standing in for a kept value, which entering it would clear.)
  if (fields.length && !fields[0].dataset.kept) setTimeout(() => fields[0].focus(), 100);
  return new Promise((resolve) => (answer = resolve));
}
$("confirm-fields").addEventListener("keydown", (event) => {
  if (event.key === "Enter") $("confirm-action").click();
});
function answered(value) {
  const resolve = answer;
  answer = null;
  if (resolve) resolve(value);
}
$("confirm-action").addEventListener("click", async () => {
  const value = reader ? await reader() : true;
  if (value === null || value === undefined) return;
  $("confirm").open = false;
  answered(value);
});
$("confirm").addEventListener("cds-modal-closed", () => answered(false));

export { invoke, $, t, tn, ref, joined, nodesOf, editHref, quoted, names, applyStrings, el, described, weekText, durationText, untilText, localDate, lines, newId, notify, overview, reachable, refresh, whenRefreshed, runningOf, conditionOf, listName, usersOf, wordingOf, holdersOf, lockedUsersOf, invalid, ask };
