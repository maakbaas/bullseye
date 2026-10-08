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

// An overflow menu opens below its button, or above it where there is no room below (Carbon's
// overflow menu does not flip by itself): its direction is set before it opens. Its items are
// Carbon's md size, 2.5rem.
function openTowardsRoom(trigger, body) {
  const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
  const height = body.querySelectorAll("cds-overflow-menu-item").length * 2.5 * rem;
  const box = trigger.getBoundingClientRect();
  const up = innerHeight - box.bottom < height && box.top > innerHeight - box.bottom;
  body.setAttribute("direction", up ? "top" : "bottom");
}

// The rows of the session `id`, added to its tile's parts (`tiles.js`).
function addRows(parts, id) {
  const list = el("cds-contained-list", { class: "session-rows" });
  parts.rows = {};
  for (const { key, icon } of ROWS) {
    const label = el("span", { class: "row-label" }, t(`sessions.fields.${key}`));
    const value = el("span", { class: "row-value" });
    const now = el("span", { class: "row-now" });
    // The row's inline action: Carbon's overflow menu with an edit icon (ghost, as on the target
    // and condition tiles), as the tile's ⋮ is; its items are made each time it opens (`fillMenu`).
    const what = t("sessions.fields.change", { what: t(`sessions.fields.${key}`).toLowerCase() });
    const icon2 = carbonIcon("edit", "icon");
    icon2.classList.add("cds--overflow-menu__icon");
    const body = el("cds-overflow-menu-body", { flipped: true, class: "row-menu" });
    const edit = el("cds-overflow-menu", { slot: "action", "aria-label": what, "menu-alignment": "bottom-end", class: "row-action" }, icon2, el("span", { slot: "tooltip-content" }, what), body);
    // Filled before it opens: the click reaches the host before its own button inside it.
    edit.addEventListener(
      "click",
      () => {
        if (edit.open) return;
        fillMenu(parts, id, key, body);
        openTowardsRoom(edit, body);
      },
      true,
    );
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

// What a row's menu holds: the targets, each ticked or not (a choice ticks or unticks it, saved at
// once, the menu staying open for the next); or one condition, allowance or lock method to choose, the one it has ticked; and at the
// end, making a new target or condition, or the settings of the allowance or lock it has. Carbon's
// overflow menu items are actions, so the tick is a checkmark in front of the name; the others keep
// its room, so the names line up.
function fillMenu(parts, id, key, body) {
  const session = current(id);
  const item = (text, act, mark = false, icon = null) => {
    const node = el(
      "cds-overflow-menu-item",
      { title: text },
      el("span", { class: "menu-choice" }, mark ? carbonIcon("checkmark") : icon ? carbonIcon(icon) : el("span", { class: "no-mark" }), el("span", {}, text)),
    );
    node.addEventListener("click", (event) => act(event, node));
    return node;
  };
  // A target's checkmark, or the room for one, as its item is open.
  const mark = (node, on) => node.querySelector(".menu-choice").firstChild.replaceWith(on ? carbonIcon("checkmark") : el("span", { class: "no-mark" }));
  const choices = (options, chosen, choose) => options.map(([value, text]) => item(text, () => value !== chosen && choose(value), value === chosen));
  const end = (text, icon, act) => {
    const node = item(text, act, false, icon);
    node.divider = true;
    return node;
  };
  let items;
  if (key === "block") {
    // What is ticked while it is open: each tick builds on the last, also before the last one's
    // save has come back.
    const chosen = new Set(session.targets);
    items = [
      ...overview.targets.map((target) => {
        return item(
          target.name,
          (event, node) => {
            // The menu stays open to tick another: the click goes no further than its item (the
            // overflow menu closes on any click that reaches its list).
            event.stopPropagation();
            const now = current(id);
            const on = chosen.has(target.id);
            // A session needs a target (the service refuses it too).
            if (on && chosen.size === 1) return notify("list-notices", "error", t("sessions.fields.saveFailed", { name: quoted(now.name) }), t("sessions.noTargets"));
            if (on) chosen.delete(target.id);
            else chosen.add(target.id);
            mark(node, !on);
            save(now, { targets: overview.targets.map((t) => t.id).filter((t) => chosen.has(t)) });
          },
          session.targets.includes(target.id),
        );
      }),
      end(t("sessions.fields.newTarget"), "add", () => make(id, "targets", (made) => save(current(id), { targets: [...current(id).targets, made.id] }))),
    ];
  } else if (key === "when") {
    items = [
      ...choices([[ALWAYS, t("sessions.fields.always")], ...overview.conditions.map((c) => [c.id, c.name])], session.condition || ALWAYS, (value) =>
        save(session, { condition: value === ALWAYS ? null : value }),
      ),
      end(t("sessions.fields.newCondition"), "add", () => make(id, "conditions", (made) => save(current(id), { condition: made.id }))),
    ];
  } else if (key === "unless") {
    const kind = session.allowance ? session.allowance.type : "none";
    items = [
      ...choices([["none", t("sessions.fields.noAllowance")], ["budget", t("sessions.fields.budget")], ["pomodoro", t("sessions.fields.pomodoro")]], kind, (value) => setAllowance(parts, id, value)),
      ...(kind !== "none" ? [end(t("sessions.fields.allowanceSettings"), "settings", () => setAllowance(parts, id, kind))] : []),
    ];
  } else {
    const method = session.lock.method;
    items = [
      ...choices(Object.entries(LOCKS).map(([value, lock]) => [value, lock.label]), method, (value) => parts.setLock(value)),
      ...(LOCKS[method] && LOCKS[method].form ? [end(t("sessions.lockSettings"), "settings", () => parts.setLock(method))] : []),
    ];
  }
  body.replaceChildren(...items);
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

export { addRows, renderRows, newSession, renameSession, openTowardsRoom };
