// The page: which view the address shows (`route`), the tabs, Settings, and starting up.

import { $, applyStrings, ask, el, invalid, invoke, joined, notify, overview, quoted, refresh, t, whenRefreshed } from "./core.js";
import { LOCKS } from "./locks.js";
import { KINDS } from "./kinds.js";
import { askText, renderTiles, tiles } from "./tiles.js";
import { closeEditor, openEditor, renderEditorLock } from "./editor.js";
import { detour } from "./detour.js";

applyStrings();
whenRefreshed(render);

let page = "blocks"; // the kind whose page or editor is shown

function render() {
  if (!$("view-settings").hidden) renderSettings();
  if (!$("view-list").hidden) renderTiles(page);
  renderEditorLock();
}

// ---- Settings: unlocking.

// What Settings shows follows where unlocking stands (`overview.unlock`): with no delay (a clean
// install) it is one click; with one it is a countdown that is asked for, that can be aborted,
// and that ends in Unlock or Abort.
function renderSettings() {
  const { delay, delay_words: delayWords, remaining, remaining_words: left } = overview.unlock;
  const free = delay === 0;
  const requested = remaining !== null;
  // Ready when the countdown is over or the other way to unlock lets go: either is enough.
  const ready = remaining === 0 || overview.unlock.way_open;
  // Nothing locked right now: nothing to unlock, so no button to ask for it (a countdown that is
  // still there can be aborted).
  const locked = overview.running.some((running) => running.locked);
  $("unlock-text").textContent = t(free ? "settings.unlockFree" : "settings.unlockText");
  $("unlock-state").hidden = locked && free;
  $("unlock-state").textContent = !locked
    ? t("settings.unlockNothing")
    : !requested
      ? t("settings.unlockAsk", { delay: delayWords })
      : ready
        ? t(remaining === 0 ? "settings.unlockReady" : "settings.unlockReadyWay")
        : t("settings.unlockWait", { left });
  $("unlock").hidden = !locked;
  $("unlock").replaceChildren(t(free || ready ? "settings.unlock" : "settings.request"), carbonIcon(free || ready ? "unlocked" : "time", "icon"));
  // Counting down: nothing to press but Abort.
  $("unlock").disabled = requested && !ready;
  $("abort-unlock").hidden = free || !requested;
  // A text or password that unlocks at once, when the countdown has not.
  const way = overview.unlock.way;
  const answer = way && (way.method === "text" || way.method === "password");
  $("unlock-answer").hidden = !locked || ready || !answer;
  const answerLabel = answer ? t("settings.way.answer" + (way.method === "text" ? "Text" : "Password")) : "";
  if (answer && $("unlock-answer").textContent.trim() !== answerLabel) $("unlock-answer").replaceChildren(answerLabel, carbonIcon("unlocked", "icon"));
  renderWay();
  const abortLabel = t(remaining === 0 ? "settings.relock" : "settings.abort");
  if ($("abort-unlock").textContent.trim() !== abortLabel) $("abort-unlock").replaceChildren(abortLabel, carbonIcon("locked", "icon"));
  // Only when it differs: this draws every second, and a label swapped under a click can lose it.
  const delayLabel = t(free ? "settings.setDelay" : "settings.changeDelay");
  if ($("set-delay").textContent.trim() !== delayLabel) $("set-delay").replaceChildren(delayLabel, carbonIcon("time", "icon"));
}

// A delay in seconds as it is typed (`1d`, `2h 30m`), to start the field from: the words the
// backend writes (`2 hours and 30 minutes`) are not what it reads.
function typedDelay(seconds) {
  const units = [["d", 86400], ["h", 3600], ["m", 60], ["s", 1]];
  const parts = [];
  for (const [unit, size] of units) {
    const n = Math.floor(seconds / size);
    seconds -= n * size;
    if (n) parts.push(`${n}${unit}`);
  }
  return parts.join(" ");
}

async function unlock() {
  const { delay, remaining, way_open: wayOpen } = overview.unlock;
  try {
    if (delay === 0 || remaining === 0 || wayOpen) {
      // The last step, which cannot be taken back: asked once more.
      const sure = await ask({ heading: t("settings.confirmHeading"), body: t("settings.confirmBody"), action: t("settings.unlock"), kind: "danger" });
      if (!sure) return;
      // Every block ends; the app stays, with nothing to enforce.
      await invoke("unlock_all");
      notify("settings-notices", "success", t("settings.unlocked"));
    } else {
      await invoke("request_unlock");
      notify("settings-notices");
    }
  } catch (e) {
    notify("settings-notices", "error", t("settings.failed"), String(e));
  }
  await refresh();
  renderSettings();
}
$("unlock").addEventListener("click", unlock);

// Unlocking with the answer of the other way to unlock: its text typed in (no pasting), or its password.
$("unlock-answer").addEventListener("click", async () => {
  const way = overview.unlock.way;
  if (!way) return;
  let password = null;
  let text = null;
  if (way.method === "text") {
    text = await askText(t("settings.way.answerHeading"), way.text, t("settings.unlock"));
    if (text === null) return;
  } else {
    const field = el("cds-password-input", { label: t("blocks.stopPassword"), autocomplete: "off" });
    password = await ask({
      heading: t("settings.way.answerHeading"),
      body: t("settings.way.answerBody"),
      action: t("settings.unlock"),
      kind: "danger",
      fields: [field],
      read: async () => (invalid(field, field.value ? null : t("blocks.stopPasswordMissing")) ? null : field.value),
    });
    if (!password) return;
  }
  try {
    await invoke("unlock_with", { password, text });
    notify("settings-notices", "success", t("settings.unlocked"));
  } catch (e) {
    notify("settings-notices", "error", t("settings.failed"), String(e));
  }
  await refresh();
  renderSettings();
});

// ---- The other way to unlock (`unlock::Way`): a lock method besides the delay, which is always there.

// Only these of the lock methods (the delay is always there), with the same labels as on a block.
const WAYS = ["none", "condition", "text", "password"];
const wayMethod = el("cds-dropdown", { autoalign: true, "title-text": t("settings.way.title"), class: "way-method" });
for (const key of WAYS) wayMethod.append(el("cds-dropdown-item", { value: key }, LOCKS[key].label));
$("way-controls").append(wayMethod);
let choosingWay = false;

// What it is now, in a line.
function renderWay() {
  const { way, way_open: open } = overview.unlock;
  const method = way ? way.method : "none";
  if (!choosingWay && wayMethod.value !== method) wayMethod.value = method;
  const key = { none: "stateNone", text: "stateText", password: "statePassword", condition: "stateCondition" }[method];
  $("way-state").textContent = open ? t("settings.way.stateOpen") : t("settings.way." + key);
}

// Setting it: the lock method's own form (worded for unlocking), then the service checks it. Cancel
// keeps what it was.
async function setWay(method) {
  notify("way-notices");
  try {
    let lock = null;
    if (method !== "none") {
      if (method === "condition" && !overview.conditions.length) {
        notify("way-notices", "error", t("settings.way.failed"), t("settings.way.noConditions"));
        return;
      }
      const current = overview.unlock.way;
      const form = LOCKS[method].form({ name: "", lock: current || { method: "none" }, kept_locks: [], condition: null });
      choosingWay = true;
      lock = await ask({
        heading: t("settings.way.heading", { method: LOCKS[method].label }),
        action: t("common.save"),
        kind: "primary",
        fields: form.fields,
        read: form.read,
      });
      if (!lock) return;
    }
    await invoke("set_unlock_way", { way: lock });
    notify("way-notices", "success", t(lock ? "settings.way.saved" : "settings.way.removed"));
  } catch (e) {
    notify("way-notices", "error", t("settings.way.failed"), String(e));
  } finally {
    choosingWay = false;
    await refresh();
    renderSettings();
  }
}
wayMethod.addEventListener("cds-dropdown-selected", (event) => setWay(event.detail.item.value));
// The dropdown says nothing when the way that is set is chosen again: its items are listened to, to open its settings again.
for (const item of wayMethod.querySelectorAll("cds-dropdown-item")) {
  item.addEventListener("click", () => {
    const set = overview.unlock.way ? overview.unlock.way.method : "none";
    if (item.value === set && LOCKS[item.value] && LOCKS[item.value].form) setWay(item.value);
  });
}

// Aborting: back to locked, as if it had not been asked for.
$("abort-unlock").addEventListener("click", async () => {
  try {
    await invoke("abort_unlock");
    notify("settings-notices", "success", t("settings.aborted"));
  } catch (e) {
    notify("settings-notices", "error", t("settings.failed"), String(e));
  }
  await refresh();
  renderSettings();
});

// How long unlocking takes, typed like a duration (0 for none, at most 24 hours). A longer delay is
// free; the service refuses a shorter one until unlocking is ready.
$("set-delay").addEventListener("click", async () => {
  notify("delay-notices");
  try {
    await invoke("set_unlock_delay", { delay: $("delay").value });
    await refresh();
    const { delay, delay_words: words } = overview.unlock;
    notify("delay-notices", "success", t(delay === 0 ? "settings.delayFree" : "settings.delaySet", { delay: words }));
  } catch (e) {
    notify("delay-notices", "error", t("settings.delayFailed"), String(e));
  }
  renderSettings();
});
$("delay").addEventListener("keydown", (event) => { if (event.key === "Enter") $("set-delay").click(); });

// Export and Import: the dialogs, the file and its reading are the app's (`export_settings`,
// `open_settings`); the service replaces everything on Import.
$("export").replaceChildren(t("settings.export"), carbonIcon("document-export", "icon"));
$("import").replaceChildren(t("settings.import"), carbonIcon("document-import", "icon"));
$("export").addEventListener("click", async () => {
  try {
    const file = await invoke("export_settings");
    if (file) notify("transfer-notices", "success", t("settings.exported", { file }));
  } catch (e) {
    notify("transfer-notices", "error", t("settings.exportFailed"), String(e));
  }
});
$("import").addEventListener("click", async () => {
  notify("transfer-notices");
  try {
    const opened = await invoke("open_settings");
    if (!opened) return;
    const { lists, conditions, blocks, unlocked } = opened.contents;
    const body = [
      t("settings.importBody", {
        lists: t("settings.importLists", { n: lists }),
        conditions: t("settings.importConditions", { n: conditions }),
        blocks: t("settings.importBlocks", { n: blocks }),
      }),
    ];
    if (overview.running.length) body.push(t("settings.importStops"));
    if (unlocked.length) body.push(t("settings.importUnlocked", { n: unlocked.length, names: joined(unlocked.map(quoted)).join("") }));
    // Store apps not installed here are left out (the backend tells which: `Library::without_missing_apps`).
    const apps = opened.left_out.map((app) => t("settings.importAppIn", { app: quoted(app.name), list: quoted(app.list) }));
    const leftOut = !opened.apps_checked ? t("settings.importAppsUnknown") : apps.length ? t("settings.importAppsLeftOut", { n: apps.length, apps: joined(apps).join("") }) : "";
    if (leftOut) body.push(leftOut);
    const sure = await ask({ heading: t("settings.importHeading"), body: body.join(" "), action: t("settings.importAction"), kind: "danger" });
    if (!sure) return;
    await invoke("import_settings", { path: opened.path });
    notify("transfer-notices", "success", t("settings.imported"), leftOut);
    await refresh();
  } catch (e) {
    notify("transfer-notices", "error", t("settings.importFailed"), String(e));
  }
});
// ---- Pages, by the address after #: #/ (blocks), #/new, #/edit/<id>, the same under
// #/lists and #/conditions, and #/settings.

// The tab of the page shown; an editor is its kind's.
function markPage(page) {
  $("pages").value = page;
}

// One view of the panel, with its heading in the fixed section above the tabs.
function showView(view) {
  for (const name of ["list", "editor", "settings"]) {
    $(`view-${name}`).hidden = name !== view;
    $(`head-${name}`).hidden = name !== view;
  }
  // The breadcrumb is always there, as in Carbon's own example of one with a single item: a link
  // with a trailing slash (`openEditor` sets the editor's trail, which has no trailing slash).
  $("trail").toggleAttribute("no-trailing-slash", view === "editor");
  if (view !== "editor") {
    const here = view === "settings" ? { label: t("settings.title"), href: "#/settings" } : { label: KINDS[page].title, href: KINDS[page].path };
    $("trail").replaceChildren(el("cds-breadcrumb-item", {}, el("cds-breadcrumb-link", { href: here.href }, here.label)));
  }
}

function route() {
  const hash = location.hash || "#/";
  let view = "list";
  let kind = "blocks";
  let edit = null;
  // A block list or condition made on a detour (`detour`): from the block editor, or for a
  // lock. Without its detour (the page was reloaded), back to where it came from.
  const nested = hash.match(/^#\/((?:new|edit\/[^/]+|lock\/[^/]+))\/(lists|conditions)\/(?:new|edit\/([^/]+))$/);
  if (nested) {
    const back = nested[1].startsWith("lock/") ? KINDS.blocks.path : `#/${nested[1]}`;
    if (!detour.current || detour.current.back !== back) {
      location.hash = back;
      return;
    }
    page = nested[2];
    showView("editor");
    // Still within the block it is made for.
    markPage("blocks");
    openEditor(nested[2], nested[3] ? decodeURIComponent(nested[3]) : null, { via: detour.current });
    $("scroller").scrollTo(0, 0);
    return;
  }
  // Back from a detour: the block as it was being edited, with what was made on it.
  const resumed = detour.current && detour.current.back === hash ? detour.current.draft : null;
  detour.current = null;
  const match = hash.match(/^#\/(?:(lists|conditions)(?:\/|$))?(new|edit\/(.+))?$/);
  if (hash === "#/settings") view = "settings";
  else if (match) {
    kind = match[1] || "blocks";
    if (match[2]) {
      view = "editor";
      edit = match[3] ? decodeURIComponent(match[3]) : "";
    }
  }
  page = kind;
  showView(view);
  // The editor belongs to its kind's page.
  markPage(view === "settings" ? "settings" : kind);
  if (view === "list") {
    const kindOf = KINDS[kind];
    $("list-title").textContent = kindOf.title;
    $("list-description").textContent = kindOf.description;
    // Always just "New", with the page's own name for screen readers.
    $("new-item").replaceChildren(t("common.new"), carbonIcon("add", "icon"));
    $("new-item").setAttribute("aria-label", kindOf.newLabel);
    $("empty-title").textContent = kindOf.empty[0];
    $("empty-text").textContent = kindOf.empty[1];
    notify("list-notices");
    $("tiles").replaceChildren();
    tiles.clear();
    renderTiles(kind);
  }
  if (view === "editor") openEditor(kind, edit || null, { draft: kind === "blocks" ? resumed : null });
  else closeEditor();
  if (view === "settings") {
    $("delay").value = typedDelay(overview.unlock.delay);
    notify("delay-notices");
    renderSettings();
    notify("settings-notices");
  }
  $("scroller").scrollTo(0, 0);
}
window.addEventListener("hashchange", route);
$("new-item").addEventListener("click", () => (location.hash = KINDS[page].newPath));
const PAGES = { blocks: "#/", lists: "#/lists", conditions: "#/conditions", settings: "#/settings" };
// Below Carbon's md breakpoint (672 px) the tabs are icons only, each with its label as tooltip.
const narrow = matchMedia("(max-width: 41.99rem)");
const tabs = document.querySelectorAll("#pages cds-tab");
for (const tab of tabs) {
  tab.prepend(carbonIcon(tab.dataset.icon));
  tab.setAttribute("aria-label", tab.textContent.trim());
}
// Carbon's icon tabs: `icon-size` on the tab bar lets their tooltips out of it.
const iconsOnly = () => {
  if (narrow.matches) $("pages").setAttribute("icon-size", "default");
  else $("pages").removeAttribute("icon-size");
  tabs.forEach((tab) => (tab.iconOnly = narrow.matches));
};
iconsOnly();
narrow.addEventListener("change", iconsOnly);
// The scrollbar's room on each side of the content (kept whether or not it is shown, on both sides
// so the content is centred), which the fixed section keeps too, so both line up (`.lane`);
// measured again when zooming changes it.
const gutter = () => {
  const scroller = $("scroller");
  document.documentElement.style.setProperty("--gutter", `${(scroller.offsetWidth - scroller.clientWidth) / 2}px`);
};
gutter();
window.addEventListener("resize", gutter);
const openPage = (page) => {
  if (location.hash !== PAGES[page]) location.hash = PAGES[page];
};
$("pages").addEventListener("cds-tabs-selected", (event) => openPage(event.detail.item.value));
// An editor counts as its page's tab, which is already selected there, so choosing it again selects
// nothing: a click still goes to the page.
for (const tab of tabs) tab.addEventListener("click", () => openPage(tab.getAttribute("value")));
$("delete").append(carbonIcon("trash-can", "icon"));
// Every button on a page has an icon: Carbon keeps room for one at the right of a button's text.
$("save").append(carbonIcon("checkmark", "icon"));
$("cancel").append(carbonIcon("close", "icon"));
$("abort-unlock").append(carbonIcon("locked", "icon"));
$("set-delay").append(carbonIcon("time", "icon"));

// No browser menu on right-click (Back, Refresh, …: WebView2's own, which make no sense in an
// app), except in a text field, where it cuts, copies and pastes.
document.addEventListener("contextmenu", (event) => {
  const typing = event.composedPath().some((node) => node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement);
  if (!typing) event.preventDefault();
});

// The window is made when it is asked for and shown once this page has drawn (`ui::window_ready`);
// its X only hides it. While it is hidden nothing is looked at, so nothing is asked of the service.
let visible = true;
window.__TAURI__.event.listen("visibility", (event) => {
  visible = event.payload;
  if (visible) refresh();
});
setInterval(() => { if (visible && !document.hidden) refresh(); }, 1000);
refresh()
  .then(route)
  .finally(() => {
    document.body.classList.add("ready");
    invoke("window_ready");
  });
