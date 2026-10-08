// A session is set up on its own tile, never in an editor page: a row each for Block (its
// targets), When (its condition), Unless (its allowance) and Lock, each opening a menu. Each
// choice is saved as it is made. An allowance's settings, a new session and a new name are asked
// in the modal. New target and New condition (in their menus) go to that kind's editor and come
// back with it chosen (`detour`).

import { $, ask, el, invalid, invoke, newId, notify, overview, quoted, refresh, runningOf, t } from "./core.js";
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

// The one menu, opened at the row it is for (as Carbon's menu button opens its menu).
const menu = el("cds-menu", { size: "md" });
document.body.append(menu);
// Escape, a click elsewhere or a choice: Carbon's menu asks to be closed, and its owner closes it.
menu.addEventListener("cds-menu-closed", () => (menu.open = false));

// The rows of the session `id`, added to its tile's parts (`tiles.js`).
function addRows(parts, id) {
  const list = el("cds-contained-list", { class: "session-rows" });
  parts.rows = {};
  for (const { key, icon } of ROWS) {
    const label = el("span", { class: "row-label" }, t(`sessions.fields.${key}`));
    const value = el("span", { class: "row-value" });
    const now = el("span", { class: "row-now" });
    // The row's inline action: an edit icon (ghost, as on the target and condition tiles) that
    // opens the row's menu, hanging from it as a menu button's does.
    const edit = el(
      "cds-icon-button",
      { slot: "action", kind: "ghost", size: "md", align: "left", "aria-label": t("sessions.fields.change", { what: t(`sessions.fields.${key}`).toLowerCase() }) },
      carbonIcon("edit", "icon"),
      el("span", { slot: "tooltip-content" }, t("sessions.fields.change", { what: t(`sessions.fields.${key}`).toLowerCase() })),
    );
    const row = el("cds-contained-list-item", {}, carbonIcon(icon, "icon"), el("span", { class: "row" }, label, el("span", { class: "row-text" }, value, now)), edit);
    edit.addEventListener("click", () => openMenu(parts, id, key, edit));
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

// The menu of a row, from its edit button.
async function openMenu(parts, id, key, button) {
  const session = overview.sessions.find((s) => s.id === id);
  // A choice closes the menu first: what it does may open the modal or another page.
  const item = (label, action, icon) => {
    const node = el("cds-menu-item", { label }, icon ? carbonIcon(icon, "render-icon") : null);
    node.addEventListener("click", () => {
      menu.open = false;
      action();
    });
    return node;
  };
  const choices = (label, options, chosen, choose) =>
    el(
      "cds-menu-item-radio-group",
      { label },
      ...options.map(([value, text]) => {
        const node = item(text, () => value !== chosen && choose(value));
        node.setAttribute("aria-checked", String(value === chosen));
        return node;
      }),
    );
  const divider = () => el("cds-menu-item-divider");
  let items;
  if (key === "block") {
    // Each tick is saved; the session is never left without a target (the service refuses it too).
    items = [
      ...overview.targets.map((target) => {
        const node = el("cds-menu-item-selectable", { label: target.name, selected: session.targets.includes(target.id) });
        node.addEventListener("click", () => {
          const now = overview.sessions.find((s) => s.id === id);
          const ticked = now.targets.includes(target.id) ? now.targets.filter((t) => t !== target.id) : [...now.targets, target.id];
          // The last one stays ticked (the item has unticked itself already).
          if (!ticked.length) {
            node.selected = true;
            return notify("list-notices", "error", t("sessions.fields.saveFailed", { name: quoted(now.name) }), t("sessions.noTargets"));
          }
          save(now, { targets: overview.targets.map((t) => t.id).filter((t) => ticked.includes(t)) });
        });
        return node;
      }),
      divider(),
      item(t("sessions.fields.newTarget"), () => make(id, "targets", (made) => save(current(id), { targets: [...current(id).targets, made.id] })), "add"),
    ];
  } else if (key === "when") {
    items = [
      choices(t("sessions.fields.when"), [[ALWAYS, t("sessions.fields.always")], ...overview.conditions.map((c) => [c.id, c.name])], session.condition || ALWAYS, (value) =>
        save(session, { condition: value === ALWAYS ? null : value }),
      ),
      divider(),
      item(t("sessions.fields.newCondition"), () => make(id, "conditions", (made) => save(current(id), { condition: made.id })), "add"),
    ];
  } else if (key === "unless") {
    const kind = session.allowance ? session.allowance.type : "none";
    items = [
      choices(t("sessions.fields.unless"), [["none", t("sessions.fields.noAllowance")], ["budget", t("sessions.fields.budget")], ["pomodoro", t("sessions.fields.pomodoro")]], kind, (value) =>
        setAllowance(parts, id, value),
      ),
      ...(kind !== "none" ? [divider(), item(t("sessions.fields.allowanceSettings"), () => setAllowance(parts, id, kind))] : []),
    ];
  } else {
    const method = session.lock.method;
    items = [
      choices(t("sessions.lockLabel"), Object.entries(LOCKS).map(([value, lock]) => [value, lock.label]), method, (value) => parts.setLock(value)),
      ...(LOCKS[method] && LOCKS[method].form ? [divider(), item(t("sessions.lockSettings"), () => parts.setLock(method))] : []),
    ];
  }
  menu.replaceChildren(...items);
  menu.label = t(`sessions.fields.${key}`);
  // Below the button, its end at the button's end (as Carbon's menu button with bottom-end
  // alignment). Its size is known once it is open, so it is opened where Carbon puts it, measured
  // (Carbon's own measure: the element its shadow root draws) and moved there before it is drawn,
  // setting the same inline position Carbon sets. Room is made below the button first, scrolling
  // the page as far as it goes; where there still is none it goes above the button.
  let b = button.getBoundingClientRect();
  menu.x = [b.left, b.right];
  menu.y = [b.top, b.bottom];
  menu.open = true;
  await menu.updateComplete;
  const { width, height } = menu.shadowRoot.querySelector(".cds--menu").getBoundingClientRect();
  const edge = 8; // Carbon's menu keeps this far from the window's edge
  const below = innerHeight - b.bottom - edge;
  if (below < height) $("scroller").scrollBy(0, height - below);
  b = button.getBoundingClientRect();
  const fits = innerHeight - b.bottom - edge >= height;
  menu.style.insetInlineStart = `${Math.max(edge, b.right - width)}px`;
  menu.style.insetBlockStart = `${fits ? b.bottom : Math.max(edge, b.top - height)}px`;
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
