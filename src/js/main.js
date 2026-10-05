// The page: which view the address shows (`route`), the tabs, Settings, and starting up.

import { $, applyStrings, ask, invoke, notify, quoted, refresh, t, whenRefreshed } from "./core.js";
import { KINDS } from "./kinds.js";
import { renderTiles, tiles } from "./tiles.js";
import { closeEditor, openEditor, renderEditorLock } from "./editor.js";
import { detour } from "./detour.js";

applyStrings();
whenRefreshed(render);

let page = "blocks"; // the kind whose page or editor is shown

function render() {
  if (!$("view-list").hidden) renderTiles(page);
  renderEditorLock();
}

// ---- Settings: the unlock password.

async function unlock() {
  const password = $("password").value;
  if (!password) return;
  $("unlock").disabled = true;
  try {
    // Every block ends; the app stays, with nothing to enforce.
    await invoke("unlock_all", { password });
    notify("settings-notices", "success", t("settings.unlocked"));
    await refresh();
  } catch (e) {
    notify("settings-notices", "error", t("settings.failed"), String(e));
  }
  $("password").value = "";
  $("password").focus();
}
$("unlock").addEventListener("click", unlock);

// Export and Import: the dialogs, the file and its reading are the app's (`export_settings`,
// `open_settings`); the service replaces everything on Import.
$("export").replaceChildren(t("settings.export"), carbonIcon("download", "icon"));
$("import").replaceChildren(t("settings.import"), carbonIcon("upload", "icon"));
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
    const { lists, schedules, blocks, unlocked } = opened.contents;
    const body = [
      t("settings.importBody", {
        lists: t("settings.importLists", { n: lists }),
        schedules: t("settings.importSchedules", { n: schedules }),
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
$("password").addEventListener("input", () => {
  $("unlock").disabled = !$("password").value;
  notify("settings-notices");
});
$("password").addEventListener("keydown", (event) => { if (event.key === "Enter") unlock(); });

// ---- Pages, by the address after #: #/ (blocks), #/new, #/edit/<id>, the same under
// #/lists and #/schedules, and #/settings.

// The tab of the page shown; an editor is its kind's.
function markPage(page) {
  $("pages").value = page;
}

function route() {
  const hash = location.hash || "#/";
  let view = "list";
  let kind = "blocks";
  let edit = null;
  // A block list or schedule made on a detour (`detour`): from the block editor, or for a
  // lock. Without its detour (the page was reloaded), back to where it came from.
  const nested = hash.match(/^#\/((?:new|edit\/[^/]+|lock\/[^/]+))\/(lists|schedules)\/new$/);
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
    openEditor(nested[2], null, { via: detour.current });
    window.scrollTo(0, 0);
    return;
  }
  // Back from a detour: the block as it was being edited, with what was made on it.
  const resumed = detour.current && detour.current.back === hash ? detour.current.draft : null;
  detour.current = null;
  const match = hash.match(/^#\/(?:(lists|schedules)(?:\/|$))?(new|edit\/(.+))?$/);
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
    $("password").value = "";
    $("unlock").disabled = true;
    notify("settings-notices");
  }
  window.scrollTo(0, 0);
}
window.addEventListener("hashchange", route);
$("new-item").addEventListener("click", () => (location.hash = KINDS[page].newPath));
const PAGES = { blocks: "#/", lists: "#/lists", schedules: "#/schedules", settings: "#/settings" };
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
