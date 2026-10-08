// A session is set up on its own tile, never in an editor page: a row each for Block (its
// targets), When (its condition), Unless (its allowance) and Lock, each opening a menu. Each
// choice is saved as it is made. An allowance's settings, a new session and a new name are asked
// in the modal. New target and New condition (in their menus) go to that kind's editor and come
// back with it chosen (`detour`).

import { ask, el, invalid, invoke, newId, notify, overview, quoted, refresh, runningOf, t } from "./core.js";
import { LOCKS } from "./locks.js";
import { allowanceSection } from "./forms.js";
import { KINDS, freeName } from "./kinds.js";
import { detour } from "./detour.js";

// The condition menu's choice for no condition.
const ALWAYS = "always";

// Saves a session with `change` applied; a refusal (a locked session, no target) is shown on the page.
async function save(session, change) {
  try {
    await invoke("save_session", { session: { ...session, ...change } });
    notify("list-notices");
  } catch (e) {
    notify("list-notices", "error", t("sessions.fields.saveFailed", { name: quoted(session.name) }), String(e));
  }
  await refresh();
}

// ---- A session's rows: Block, When, Unless and Lock, a `cds-contained-list` on its tile. A row
// that can be changed opens Carbon's menu of what it can be: the targets to tick, one condition,
// allowance or lock method to choose, and making a new target or condition, or the settings of
// the allowance or lock it has. A locked session's targets, condition and lock stay as they are
// (no menu); its allowance may still get stricter, which is for the service to judge.

const ROWS = [
  { key: "block", icon: "list-checked-16" },
  { key: "when", icon: "rule-16" },
  { key: "unless", icon: "pause-16" },
  { key: "lock", icon: "locked" },
];


// The rows of the session `id`, added to its tile's parts (`tiles.js`).
function addRows(parts, id) {
  const list = el("cds-contained-list", { class: "session-rows" });
  parts.rows = {};
  for (const { key, icon } of ROWS) {
    const label = el("span", { class: "row-label" }, t(`sessions.fields.${key}`));
    const value = el("span", { class: "row-value" });
    const now = el("span", { class: "row-now" });
    // The row's inline action: Carbon's tab-tip popover, its edit button and the open panel one
    // shape; the panel is filled each time it opens (`fillPanel`). Carbon's popover closes itself on
    // Escape and on a click elsewhere, and flips above its button where there is no room below.
    const what = t("sessions.fields.change", { what: t(`sessions.fields.${key}`).toLowerCase() });
    const button = el("button", { type: "button", class: "cds--popover--tab-tip__button", "aria-label": what, title: what }, carbonIcon("edit"));
    const panel = el("cds-popover-content", {});
    const edit = el("cds-popover", { slot: "action", tabtip: true, autoalign: true, align: "bottom-end", class: "row-action" }, button, panel);
    const close = () => (edit.open = false);
    button.addEventListener("click", async () => {
      if (!edit.open) fillPanel(parts, id, key, panel, close);
      edit.open = !edit.open;
      if (!edit.open) return;
      // The focus to its first control, as a popover's content should have it (and Escape closes it
      // from there).
      await edit.updateComplete;
      // Carbon joins a tab-tip's button to its panel below it only; where the panel went above
      // (no room below), the button joins it at its top instead (`.up`, app.css).
      requestAnimationFrame(() => button.classList.toggle("up", panel.firstElementChild.getBoundingClientRect().bottom <= button.getBoundingClientRect().top + 1));
      // A clickable list item is a button inside it; it does not hand on the focus itself.
      const first = panel.querySelector("cds-checkbox, cds-contained-list-item[clickable]");
      const inner = first && first.shadowRoot && first.shadowRoot.querySelector("button");
      (inner || first)?.focus();
    });
    const row = el("cds-contained-list-item", {}, carbonIcon(icon, "icon"), el("span", { class: "row" }, label, el("span", { class: "row-text" }, value, now)), edit);
    list.append(row);
    parts.rows[key] = { row, label, value, now, edit };
  }
  parts.top.after(list);
}

// What the rows say: `lines` per row, its value and what is happening now (`tiles.js`), and which
// can be changed.
function renderRows(parts, session, lines) {
  const running = runningOf(session.id);
  const locked = !!(running && running.locked);
  for (const { key } of ROWS) {
    const { label, value, now, edit } = parts.rows[key];
    // "Target" or "Targets", as many as it has.
    if (key === "block") {
      const named = t("sessions.fields.blockCount", { n: session.targets.length });
      if (label.textContent !== named) label.textContent = named;
    }
    const [text, current] = lines[key];
    if (value.textContent !== text) value.textContent = text;
    if (now.textContent !== (current || "")) now.textContent = current || "";
    now.hidden = !current;
    // Fixed while locked; the allowance only stricter.
    const open = !(locked && key !== "unless");
    edit.hidden = !open;
  }
}

// What a row's panel holds, under the row's name: the targets as Carbon checkboxes, each tick saved
// at once and the panel staying open for the next, and a button to make a new one; or one condition,
// allowance or lock method in a plain list to click (`choices`).
function fillPanel(parts, id, key, panel, close) {
  const session = current(id);
  const button = (text, icon, act) => {
    const node = el("cds-button", { kind: "tertiary", size: "md" }, text, carbonIcon(icon, "icon"));
    node.addEventListener("click", () => {
      close();
      act();
    });
    return node;
  };
  // One of several: a plain list to click (Carbon's contained list, its kind for one in a popover),
  // the one it has checked, the others keeping its room; a choice closes the panel. Making a new one
  // or the settings of the one it has are the list's last rows, with their icons.
  const choices = (options, chosen, choose, more = []) =>
    el(
      "cds-contained-list",
      { kind: "disclosed", label: t(`sessions.fields.${key}`) },
      ...options.map(([value, text]) =>
        listItem(text, value === chosen ? carbonIcon("checkmark", "icon") : el("span", { slot: "icon", class: "no-mark" }), () => value !== chosen && choose(value)),
      ),
      ...more.map(([text, icon, act]) => listItem(text, carbonIcon(icon, "icon"), act)),
    );
  const listItem = (text, icon, act) => {
    const item = el("cds-contained-list-item", { clickable: true }, icon, text);
    item.addEventListener("cds-contained-list-item-click", () => {
      close();
      act();
    });
    return item;
  };
  let nodes;
  if (key === "block") {
    // What is ticked while it is open: each tick builds on the last, also before the last one's
    // save has come back.
    const chosen = new Set(session.targets);
    const group = el("cds-checkbox-group", { "legend-text": t("sessions.fields.block") });
    for (const target of overview.targets) {
      const box = el("cds-checkbox", { "label-text": target.name });
      box.checked = chosen.has(target.id);
      box.addEventListener("cds-checkbox-changed", () => {
        const now = current(id);
        // A session needs a target (the service refuses it too).
        if (!box.checked && chosen.size === 1) {
          box.checked = true;
          return notify("list-notices", "error", t("sessions.fields.saveFailed", { name: quoted(now.name) }), t("sessions.noTargets"));
        }
        if (box.checked) chosen.add(target.id);
        else chosen.delete(target.id);
        save(now, { targets: overview.targets.map((t) => t.id).filter((t) => chosen.has(t)) });
      });
      group.append(box);
    }
    nodes = [group, button(t("sessions.fields.newTarget"), "add", () => make(id, "targets", (made) => save(current(id), { targets: [...current(id).targets, made.id] })))];
  } else if (key === "when") {
    nodes = [
      choices(
        [[ALWAYS, t("sessions.fields.always")], ...overview.conditions.map((c) => [c.id, c.name])],
        session.condition || ALWAYS,
        (value) => save(current(id), { condition: value === ALWAYS ? null : value }),
        [[t("sessions.fields.newCondition"), "add", () => make(id, "conditions", (made) => save(current(id), { condition: made.id }))]],
      ),
    ];
  } else if (key === "unless") {
    const kind = session.allowance ? session.allowance.type : "none";
    nodes = [
      choices(
        [["none", t("sessions.fields.noAllowance")], ["budget", t("sessions.fields.budget")], ["pomodoro", t("sessions.fields.pomodoro")]],
        kind,
        (value) => setAllowance(parts, id, value),
        kind !== "none" ? [[t("sessions.fields.allowanceSettings"), "settings", () => setAllowance(parts, id, kind)]] : [],
      ),
    ];
  } else {
    const method = session.lock.method;
    nodes = [
      choices(
        Object.entries(LOCKS).map(([value, lock]) => [value, lock.label]),
        method,
        (value) => parts.setLock(value),
        LOCKS[method] && LOCKS[method].form ? [[t("sessions.lockSettings"), "settings", () => parts.setLock(method)]] : [],
      ),
    ];
  }
  panel.replaceChildren(el("div", { class: "row-panel" }, ...nodes));
}

const current = (id) => overview.sessions.find((s) => s.id === id);

// Made on the way: that kind's editor, then back here with it chosen.
function make(id, kind, onSaved) {
  detour.current = {
    back: KINDS.sessions.path,
    crumbs: [{ label: KINDS.sessions.title, href: KINDS.sessions.path }, { label: current(id).name, href: KINDS.sessions.path }],
    onSaved,
  };
  location.hash = `#/session/${encodeURIComponent(id)}/${kind}/new`;
}

// Sets a session's allowance: none at once, a budget or a cycle by its settings in the modal.
async function setAllowance(parts, id, type) {
  const session = overview.sessions.find((s) => s.id === id);
  if (type === "none") return save(session, { allowance: null });
  const form = allowanceSection();
  form.fill(session.allowance && session.allowance.type === type ? session.allowance : { type });
  // Its type is what was chosen in the dropdown.
  form.nodes[0].hidden = true;
  parts.choosing = true;
  const terms = await ask({
    heading: t(`sessions.fields.${type}Heading`, { name: quoted(session.name) }),
    action: t("common.save"),
    kind: "primary",
    fields: form.nodes,
    read: async () => {
      await form.settle();
      return form.error() ? null : form.read();
    },
  });
  parts.choosing = false;
  if (terms) await save(session, { allowance: terms });
  else await refresh();
}

// A new session: its name and its targets, in the modal; the rest is set on its tile, which
// opens expanded.
async function newSession(expand) {
  const name = el("cds-text-input", { label: t("common.name"), value: freeName("sessions", overview.sessions) });
  const targets = el("cds-multi-select", {
    "title-text": t("sessions.fields.targets"),
    label: t("sessions.fields.chooseTargets"),
    size: "md",
    autoalign: true,
    "selection-feedback": "fixed",
  });
  targets.append(...overview.targets.map((target) => el("cds-multi-select-item", { value: target.id }, target.name)));
  const made = await ask({
    heading: t("sessions.newLabel"),
    action: t("common.save"),
    kind: "primary",
    fields: [name, targets],
    read: async () => {
      const chosen = targets.value ? targets.value.split(",") : [];
      if (!name.value.trim()) return invalid(name, t("sessions.nameMissing")) && null;
      if (!chosen.length) return invalid(targets, t("sessions.noTargets")) && null;
      return { id: newId(), name: name.value.trim(), targets: chosen, condition: null, allowance: null, lock: { method: "none" } };
    },
  });
  if (!made) return;
  // Open from the start, and in view once its tile is there.
  expand(made.id);
  await save(made, {});
  expand(made.id);
}

// A new name, in the modal.
async function renameSession(id) {
  const session = overview.sessions.find((s) => s.id === id);
  const name = el("cds-text-input", { label: t("common.name"), value: session.name });
  const given = await ask({
    heading: t("sessions.fields.renameHeading", { name: quoted(session.name) }),
    action: t("common.save"),
    kind: "primary",
    fields: [name],
    read: async () => (name.value.trim() ? name.value.trim() : invalid(name, t("sessions.nameMissing")) && null),
  });
  if (given) await save(session, { name: given });
}

export { addRows, renderRows, newSession, renameSession };
