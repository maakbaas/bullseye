// A session is set up on its own tile, never in an editor page: a row each for its Targets,
// Condition, Allowance and Lock. Each choice is saved as it is made. An allowance's settings, a
// lock's settings, a new session and a new name are asked in the modal. New target and New
// condition go to that kind's editor and come back with it chosen (`detour`).

import { ask, confirmStricter, dismiss, editHref, el, invalid, invoke, newId, nodesOf, notify, overview, quoted, refresh, runningOf, t } from "./core.js";
import { LOCKS } from "./locks.js";
import { allowanceSection } from "./forms.js";
import { KINDS, freeName } from "./kinds.js";
import { detour } from "./detour.js";

// The condition dropdown's choice for no condition.
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

// ---- A session's rows: Targets, Condition, Allowance and Lock (keys `block`, `when`, `unless`,
// `lock`), a `cds-contained-list` on its tile. Condition, Allowance and Lock are a dropdown in the
// row, each choice saved as it is made; beside it a button for what the choice has to set (the
// condition's page, an allowance's or a lock's settings, in the modal). Targets are many, so their
// button opens a list to tick in the modal. A locked session's targets, condition and lock method
// stay as they are; its allowance may still get stricter, and a timed or delay lock longer, which is
// for the service to judge.

const ROWS = [
  { key: "block", icon: "target-16" },
  { key: "when", icon: "rule-16" },
  { key: "unless", icon: "pause-16" },
  { key: "lock", icon: "locked" },
];

// What a row's dropdown offers, and which of it the session has.
function optionsOf(key) {
  if (key === "when") {
    return [[ALWAYS, t("sessions.fields.always")], ...overview.conditions.map((c) => [c.id, c.name])];
  }
  if (key === "unless") {
    return ["none", "budget", "pomodoro", "delayed"].map((type) => [type, t(`allowance.type.${type}`)]);
  }
  return Object.entries(LOCKS).map(([value, lock]) => [value, lock.label]);
}
function chosenOf(key, session) {
  if (key === "when") return overview.conditions.some((c) => c.id === session.condition) ? session.condition : ALWAYS;
  if (key === "unless") return session.allowance ? session.allowance.type : "none";
  return session.lock.method;
}

// Whether there is more room above a dropdown than below it, in the scrolling part of the window
// (a menu cannot show outside it).
function roomAbove(select) {
  const [box, view] = [select.getBoundingClientRect(), document.getElementById("scroller").getBoundingClientRect()];
  return box.top - view.top > view.bottom - box.bottom;
}

// Whether the service would accept a change to what a locked session holds (`check_stricter`); if
// not, why, where the other errors of the tile are. Asked before anything is confirmed.
async function allowed(check, session) {
  if (!runningOf(session.id)?.locked) return true;
  try {
    await invoke("check_stricter", { check });
    return true;
  } catch (e) {
    notify("list-notices", "error", t("sessions.fields.saveFailed", { name: quoted(session.name) }), String(e));
    return false;
  }
}

// The rows of the session `id`, added to its tile's parts (`tiles.js`).
function addRows(parts, id) {
  const list = parts.rows;
  parts.fields = {};
  for (const { key, icon } of ROWS) {
    const label = el("span", { class: "row-label" }, t(`sessions.fields.${key}`));
    const value = el("span", { class: "row-value" });
    // Under it, in a few words, what is happening now (`tiles.js`).
    const now = el("span", { class: "row-now cds--type-body-compact-01" });
    // The row's inline action: a ghost icon button, as Carbon's contained lists have them.
    const button = (symbol, tip, act) => {
      const node = el("cds-icon-button", { kind: "ghost", size: "md", align: "left" }, carbonIcon(symbol, "icon"), el("span", { slot: "tooltip-content" }, tip));
      node.addEventListener("click", act);
      return node;
    };
    let select = null;
    let edit;
    if (key === "block" || key === "when") {
      // A list to tick or choose from in the modal (`fillPanel`), filled each time it opens.
      edit = button("edit", t(`sessions.fields.${key === "block" ? "changeBlock" : "changeWhen"}`), async () => {
        const { node, chosen } = choices(id, key);
        const heading = t("sessions.fields.panelHeading", { what: t(`sessions.fields.${key}`), name: quoted(current(id).name) });
        if (!(await ask({ heading, fields: [node], action: t("common.save"), kind: "primary", read: () => true }))) return;
        const next = chosen();
        const session = current(id);
        if (Object.entries(next).some(([field, value]) => JSON.stringify(value) !== JSON.stringify(session[field] ?? null))) save(session, next);
      });
    } else {
      select = el("cds-dropdown", { type: "inline", size: "sm", "aria-label": t(`sessions.fields.${key}`) });
      // Its menu opens under it, left edges together, or above it where there is more room there.
      for (const gesture of ["pointerdown", "keydown"]) select.addEventListener(gesture, () => (select.direction = roomAbove(select) ? "top" : "bottom"), true);
      select.addEventListener("cds-dropdown-selected", async (event) => {
        const chosen = event.detail.item.value;
        const session = current(id);
        if (chosen === chosenOf(key, session)) return;
        if (key === "unless") {
          // A locked session may only keep the kind it has or lose it: the service says so before anything is asked.
          if (!(await allowed({ what: "allowance_kind", id, kind: chosen }, session))) return refresh();
          return setAllowance(parts, id, chosen);
        }
        parts.setLock(chosen);
      });
      edit = {
        unless: () => button("settings", t("sessions.fields.allowanceSettings"), () => setAllowance(parts, id, chosenOf("unless", current(id)))),
        lock: () => button("settings", t("sessions.lockSettings"), () => parts.setLock(chosenOf("lock", current(id)))),
      }[key]();
    }
    const buttons = el("span", { slot: "action", class: "row-actions" }, edit);
    const box = el("span", { class: "row" }, label, el("span", { class: "row-text" }, select, value, now));
    const row = el("cds-contained-list-item", {}, carbonIcon(icon, "icon"), box, buttons);
    list.append(row);
    parts.fields[key] = { row, box, label, value, now, edit, select };
  }
}

// Whether a running lock can be made stricter: one that ends at a time (a duration or until) can
// end later, a delay can be longer (the service judges which).
const lengthens = (running) => running.remaining !== null && !running.lock_condition && !running.until_restart && !running.password && !running.lock_text ? true : !!running.stop_delay;

// What the rows say: `lines` per row, its value and what is happening now (`tiles.js`), what the
// dropdowns have chosen, and which can be changed.
function renderRows(parts, session, lines) {
  const running = runningOf(session.id);
  const locked = !!(running && running.locked);
  for (const { key } of ROWS) {
    const { label, value, now, edit, select } = parts.fields[key];
    // "Target" or "Targets", as many as it has.
    if (key === "block") {
      const named = t("sessions.fields.blockCount", { n: session.targets.length });
      if (label.textContent !== named) label.textContent = named;
    }
    // The targets' and the condition's names; a dropdown says what the other rows have.
    const line = select ? "" : lines[key];
    const shown = JSON.stringify(line);
    if (value.dataset.shown !== shown) {
      value.dataset.shown = shown;
      value.replaceChildren(...(line ? nodesOf(line) : []));
    }
    const status = lines.now[key] || "";
    const said = JSON.stringify(status);
    if (now.dataset.shown !== said) {
      now.dataset.shown = said;
      now.replaceChildren(...(status ? nodesOf(status) : []));
    }
    if (select) {
      // The choices are rebuilt only when they changed, so an open dropdown is left alone.
      const options = optionsOf(key);
      const signature = JSON.stringify(options);
      if (select.dataset.options !== signature) {
        select.dataset.options = signature;
        select.replaceChildren(...options.map(([choice, name]) => el("cds-dropdown-item", { value: choice }, name)));
        select.value = "";
      }
      const chosen = chosenOf(key, session);
      if (select.value !== chosen) select.value = chosen;
      // Fixed while locked; the allowance only stricter, so none stays none (`allowance::kind_allowed`).
      select.disabled = locked && (key !== "unless" || !session.allowance);
    }
    // The button: targets and the condition's page, the settings of the allowance or lock it has. A
    // locked session's targets cannot change; its lock settings only to end later or wait longer.
    let open = true;
    if (key === "block" || key === "when") open = !locked;
    if (key === "unless") open = chosenOf(key, session) !== "none";
    if (key === "lock") open = !!(LOCKS[session.lock.method] && LOCKS[session.lock.method].form) && (!locked || lengthens(running));
    edit.hidden = !open;
  }
}

// A row's list in the dialog: the targets to mark, or the conditions to choose one of, under a New
// button on the right (made on a detour, then chosen); nothing is saved before Save. `chosen()`:
// what to save.
function choices(id, key) {
  const session = current(id);
  const many = key === "block";
  const kind = many ? "targets" : "conditions";
  const options = many ? overview.targets.map((target) => [target.id, target.name]) : optionsOf("when");
  const picked = new Set(many ? session.targets : [chosenOf("when", session)]);
  const mark = (on) => (on ? carbonIcon("checkmark", "icon") : el("span", { slot: "icon", class: "no-mark" }));
  const marks = new Map();
  const items = options.map(([value, text]) => {
    marks.set(value, mark(picked.has(value)));
    const item = el("cds-contained-list-item", { clickable: true }, marks.get(value), text);
    item.addEventListener("cds-contained-list-item-click", () => {
      // Targets are marked and unmarked, all but the last; a condition is one of them.
      if (many && picked.has(value)) {
        if (picked.size > 1) picked.delete(value);
      } else {
        if (!many) picked.clear();
        picked.add(value);
      }
      for (const [other, old] of marks) {
        const next = mark(picked.has(other));
        old.replaceWith(next);
        marks.set(other, next);
      }
    });
    return item;
  });
  const chosen = () => {
    if (many) return { targets: options.map(([value]) => value).filter((value) => picked.has(value)) };
    const [value] = picked;
    return { condition: value === ALWAYS ? null : value };
  };
  const more = el("cds-button", { kind: "primary", size: "sm" }, t(`${kind}.newLabel`), carbonIcon("add", "icon"));
  more.addEventListener("click", () => {
    dismiss();
    make(id, kind, (made) => save(current(id), many ? { targets: [...chosen().targets, made.id] } : { condition: made.id }));
  });
  const list = el("cds-contained-list", { kind: "disclosed", size: "md", "aria-label": t(`${kind}.title`) }, ...items);
  return { node: el("div", { class: "choices" }, more, list), chosen };
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
  const running = runningOf(id);
  const locked = !!(running && running.locked);
  if (type === "none") {
    // Taking it away from a locked session is stricter: asked first.
    if (locked && !((await allowed({ what: "allowance", id, allowance: null }, session)) && (await confirmStricter(quoted(session.name), t("stricter.noAllowance"))))) return refresh();
    return save(session, { allowance: null });
  }
  const form = allowanceSection();
  form.fill(session.allowance && session.allowance.type === type ? session.allowance : { type });
  // Its type is what was chosen in the panel.
  form.nodes[0].hidden = true;
  parts.choosing = true;
  const terms = await ask({
    heading: t(`sessions.fields.${type}Heading`, { name: quoted(session.name) }),
    action: t("common.save"),
    kind: "primary",
    fields: form.nodes,
    read: async () => {
      await form.settle();
      if (form.error()) return null;
      const read = form.read();
      // A locked session takes only a stricter one: the service says if it would not, in the dialog.
      if (locked) await invoke("check_stricter", { check: { what: "allowance", id, allowance: read } });
      return read;
    },
  });
  parts.choosing = false;
  if (!terms) return refresh();
  // A locked session takes only a stricter one, which cannot be taken back: asked first.
  if (locked) {
    const words = await invoke("describe_allowance", { terms }).then((described) => t("common.sentence", { text: described.words }), () => "");
    parts.choosing = true;
    const sure = await confirmStricter(quoted(session.name), words);
    parts.choosing = false;
    if (!sure) return refresh();
  }
  await save(session, { allowance: terms });
}

// A new session: its name, in the modal; the rest, targets too, is set on its tile, which is then
// shown on its own, as a link to it shows it. Without a target it does nothing yet.
async function newSession() {
  const name = el("cds-text-input", { label: t("common.name"), value: freeName("sessions", overview.sessions) });
  const made = await ask({
    heading: t("sessions.newLabel"),
    action: t("common.save"),
    kind: "primary",
    fields: [name],
    read: async () => {
      if (!name.value.trim()) return invalid(name, t("sessions.nameMissing")) && null;
      return { id: newId(), name: name.value.trim(), targets: [], condition: null, allowance: null, lock: { method: "none" } };
    },
  });
  if (!made) return;
  await save(made, {});
  location.hash = editHref("sessions", made.id);
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
