// The editor of a target or condition: opening it (also on a detour from a session's tile,
// `detour.js`), its breadcrumb, saving and deleting, and keeping what a locked session uses to
// stricter changes.

import { $, confirmStricter, el, invoke, lockedUsersOf, names, notify, refresh, t } from "./core.js";
import { KINDS } from "./kinds.js";
import { confirmDelete } from "./tiles.js";

// The form of the kind being edited, built anew each time the editor opens. A reused one would
// put Carbon's text areas back into the page after they had been taken out of it, and Carbon
// then no longer measures an empty one, whose helper text stays squeezed to zero width.
const forms = {};
let editing = null; // { kind, item (as opened), isNew }

// The breadcrumb: links up to the last, which is this page.
function setCrumbs(crumbs) {
  $("trail").replaceChildren(
    ...crumbs.map((crumb, i) =>
      el("cds-breadcrumb-item", {}, el("cds-breadcrumb-link", i === crumbs.length - 1 ? { "is-currentpage": true } : { href: crumb.href }, crumb.label)),
    ),
  );
}

// `via`: the detour this item is made on.
function openEditor(kind, id, { via = null } = {}) {
  const kindOf = KINDS[kind];
  const item = id ? kindOf.items().find((i) => i.id === id) : kindOf.create();
  if (!item) {
    location.hash = kindOf.path;
    return;
  }
  editing = { kind, item: structuredClone(item), isNew: !id, via };
  $("view-editor").dataset.kind = kind;
  const form = (forms[kind] = kindOf.form());
  $("form").replaceChildren(...form.nodes);
  form.fill(editing.item, editing.isNew);
  const here = id ? (kindOf.items().find((i) => i.id === id) || item).name : kindOf.newLabel;
  setCrumbs(via ? [...via.crumbs, { label: here }] : [{ label: kindOf.title, href: kindOf.path }, { label: here }]);
  $("editor-title").textContent = id ? kindOf.editTitle : kindOf.newLabel;
  $("editor-description").textContent = kindOf.editorDescription;
  $("cancel").setAttribute("href", via ? via.back : kindOf.path);
  $("delete").hidden = !id || !!via;
  notify("editor-notices");
  $("editor-notices").dataset.lock = "";
  renderEditorLock();
}

// What runs keeps what it started with, so what it uses is read-only until it ends.
function renderEditorLock() {
  if ($("view-editor").hidden || !editing) return;
  const kind = KINDS[editing.kind];
  const held = !editing.isNew && kind.held(editing.item);
  // Locked: what is used may only get stricter, which is for the service to judge.
  const limited = !held && !editing.isNew && kind.limited(editing.item);
  forms[editing.kind].disable(!!held);
  // No example entries where entries are fixed or limited, or they read as entries.
  if (forms[editing.kind].examples) forms[editing.kind].examples(!held && !limited);
  // Only the name can be changed on something a locked session holds; it never loosens anything.
  $("delete").disabled = !!held || (!editing.isNew && kind.inUse(editing.item));
  const box = $("editor-notices");
  const shown = box.dataset.lock === "1";
  const note = held || limited;
  // An error stays until the next save or leaving; the lock note must not replace it.
  if (box.dataset.lock === "error") return;
  if (note && !shown) {
    notify("editor-notices", "info", t(held ? "editor.lockedTitle" : "editor.limitedTitle"), note, { closable: false });
    box.dataset.lock = "1";
  } else if (!note && shown) {
    notify("editor-notices");
    box.dataset.lock = "";
  }
}

$("save").addEventListener("click", async () => {
  const { kind } = editing;
  const form = forms[kind];
  // What is still being typed counts, and what cannot be read is said here.
  await form.settle();
  const problem = form.error();
  // What is wrong is said at its field (`error()` marks it): the first of them is brought into view.
  if (problem) {
    const wrong = $("form").querySelector("[invalid], .form-note[kind='error']:not([hidden])");
    if (wrong) wrong.scrollIntoView({ block: "center", behavior: "smooth" });
    return;
  }
  $("save").disabled = true;
  try {
    const saved = form.read(editing.item);
    // What a locked session uses can only get stricter, and that cannot be taken back: asked first
    // (nothing to ask when nothing is changed).
    const holders = editing.isNew ? [] : lockedUsersOf(editing.item.id);
    if (holders.length && JSON.stringify(saved) !== JSON.stringify(editing.item)) {
      // The service says first if it would be refused, as it is when it is looser.
      await invoke("check_stricter", { check: kind === "targets" ? { what: "target", target: saved } : { what: "condition", condition: saved } });
      if (!(await confirmStricter(names(holders), t("stricter.shared")))) {
        $("save").disabled = false;
        return;
      }
    }
    await KINDS[kind].save(saved);
    const { via } = editing;
    if (via) {
      // Back where it was made from, with it: the service's view first, so it is there.
      await via.onSaved(saved);
      await refresh();
      location.hash = via.back;
    } else location.hash = KINDS[kind].path;
  } catch (e) {
    notify("editor-notices", "error", t("editor.saveFailed", { noun: KINDS[kind].noun }), String(e));
    $("editor-notices").dataset.lock = "error";
  }
  $("save").disabled = false;
  await refresh();
});
$("delete").addEventListener("click", () => editing && confirmDelete(editing.kind, editing.item.id));

// Leaving the editor (`main.js`).
function closeEditor() {
  editing = null;
}

export { openEditor, renderEditorLock, closeEditor };
