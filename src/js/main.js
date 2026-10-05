// The page: which view the address shows (`route`), the tabs, Settings, and starting up.

import { $, applyStrings, ask, invoke, notify, overview, quoted, refresh, t, whenRefreshed } from "./core.js";
import { KINDS } from "./kinds.js";
import { renderTiles, tiles } from "./tiles.js";
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
  const ready = remaining === 0;
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
        ? t("settings.unlockReady")
        : t("settings.unlockWait", { left });
  $("unlock").hidden = !locked;
  $("unlock").replaceChildren(t(free || ready ? "settings.unlock" : "settings.request"));
  // Counting down: nothing to press but Abort.
  $("unlock").disabled = requested && !ready;
  $("abort-unlock").hidden = free || !requested;
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
  const { delay, remaining } = overview.unlock;
  try {
    if (delay === 0 || remaining === 0) {
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
    if (unlocked.length) body.push(t("settings.importUnlocked", { n: unlocked.length, names: unlocked.map(quoted).join(t("common.separator")) }));
    const sure = await ask({ heading: t("settings.importHeading"), body: body.join(" "), action: t("settings.importAction"), kind: "danger" });
    if (!sure) return;
    await invoke("import_settings", { path: opened.path });
    notify("transfer-notices", "success", t("settings.imported"));
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
    for (const name of ["list", "editor", "settings"]) $(`view-${name}`).hidden = name !== "editor";
    // Still within the block it is made for.
    markPage("blocks");
    openEditor(nested[2], nested[3] ? decodeURIComponent(nested[3]) : null, { via: detour.current });
    window.scrollTo(0, 0);
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
  for (const name of ["list", "editor", "settings"]) $(`view-${name}`).hidden = name !== view;
  // The editor belongs to its kind's page.
  markPage(view === "settings" ? "settings" : kind);
  if (view === "list") {
    const kindOf = KINDS[kind];
    $("list-title").textContent = kindOf.title;
    $("list-description").textContent = kindOf.description;
    $("new-item").replaceChildren(kindOf.newLabel, carbonIcon("add", "icon"));
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
  window.scrollTo(0, 0);
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
$("pages").addEventListener("cds-tabs-selected", (event) => {
  const page = event.detail.item.value;
  if (location.hash !== PAGES[page]) location.hash = PAGES[page];
});
$("delete").append(carbonIcon("trash-can", "icon"));

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
