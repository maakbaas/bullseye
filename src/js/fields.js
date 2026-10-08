// A session is set up on its own tile, never in an editor page: its fields, Block (its targets),
// When (its condition), Unless (its allowance) and Lock, below the fold of the tile (shown when
// it is expanded). Each choice is saved as it is made, as the lock always was; the targets when
// their menu closes, so ticking several is one change to a running session. An allowance's
// settings, a new session and a new name are asked in the modal. New target and New condition
// (the + beside their fields) go to that kind's editor and come back with it chosen (`detour`).

import { allowanceLabel, ask, el, invalid, invoke, newId, notify, overview, quoted, refresh, runningOf, t } from "./core.js";
import { allowanceSection } from "./forms.js";
import { KINDS, freeName } from "./kinds.js";
import { detour } from "./detour.js";

// The condition dropdown's choice for no condition (an empty value reads as nothing chosen).
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

// A + icon button beside a field, named by its tooltip: making a new item to choose. Carbon's
// menus hold only choices, so the action is a button of its own.
function plus(label) {
  const button = el("cds-icon-button", { kind: "tertiary", size: "md", align: "top-end" }, carbonIcon("add", "icon"), el("span", { slot: "tooltip-content" }, label));
  button.setAttribute("aria-label", label);
  return button;
}

// The fields of the session `id`, added to its tile's parts (`tiles.js`); the lock's dropdown,
// made with Start (`addSessionControls`), is the fourth.
function addFields(parts, id) {
  const session = () => overview.sessions.find((s) => s.id === id);
  // Kept in the order of the Targets page, ticked or not (Carbon's default moves ticked ones up).
  const targets = el("cds-multi-select", { "title-text": t("sessions.fields.block"), size: "md", autoalign: true, label: "", "selection-feedback": "fixed" });
  const newTarget = plus(t("sessions.fields.newTarget"));
  const condition = el("cds-dropdown", { "title-text": t("sessions.fields.when"), size: "md", autoalign: true });
  const newCondition = plus(t("sessions.fields.newCondition"));
  const allowance = el("cds-dropdown", { "title-text": t("sessions.fields.unless"), size: "md", autoalign: true });
  const fields = el(
    "div",
    { class: "session-fields" },
    el("div", { class: "with-action" }, targets, newTarget),
    el("div", { class: "with-action" }, condition, newCondition),
    allowance,
    parts.controls,
  );
  parts.below.append(fields);
  Object.assign(parts, { targets, newTarget, condition, newCondition, allowance });

  // Made on the way: that kind's editor, then back here with it chosen.
  const make = (kind, onSaved) => {
    detour.current = {
      back: KINDS.sessions.path,
      crumbs: [{ label: KINDS.sessions.title, href: KINDS.sessions.path }, { label: session().name, href: KINDS.sessions.path }],
      onSaved,
    };
    location.hash = `#/session/${encodeURIComponent(id)}/${kind}/new`;
  };
  newTarget.addEventListener("click", () => make("targets", (made) => save(session(), { targets: [...session().targets, made.id] })));
  newCondition.addEventListener("click", () => make("conditions", (made) => save(session(), { condition: made.id })));

  // Ticking and unticking are kept on the tile while the menu is open, and saved when it closes.
  const chosen = () => (targets.value ? targets.value.split(",") : []);
  targets.addEventListener("cds-multi-select-selected", () => {
    parts.choosing = true;
    invalid(targets, chosen().length ? null : t("sessions.noTargets"));
  });
  targets.addEventListener("cds-multi-select-toggled", () => {
    if (targets.open || !parts.choosing) return;
    parts.choosing = false;
    const now = session();
    const ticked = chosen();
    invalid(targets, null);
    if (ticked.length && ticked.join(",") !== now.targets.join(",")) save(now, { targets: ticked });
    else refresh();
  });
  condition.addEventListener("cds-dropdown-selected", (event) => {
    const value = event.detail.item.value;
    save(session(), { condition: value === ALWAYS ? null : value });
  });
  allowance.addEventListener("cds-dropdown-selected", (event) => setAllowance(parts, id, event.detail.item.value));
  // The one it has already, chosen again (the dropdown says nothing then): its settings.
  allowance.addEventListener("click", (event) => {
    const item = event.target.closest && event.target.closest("cds-dropdown-item");
    const now = session() && session().allowance;
    if (item && now && item.value === now.type) setAllowance(parts, id, item.value);
  });
}

// What the fields show of `session`; nothing changes under a choice being made.
function renderFields(parts, session) {
  if (parts.choosing) return;
  const running = runningOf(session.id);
  // Locked: its targets and condition stay as they are; its allowance may still get stricter,
  // which is for the service to judge (`allowance::tightened`).
  const locked = !!(running && running.locked);
  // The items are made again only when what there is to choose changes.
  const targetIds = overview.targets.map((target) => `${target.id}:${target.name}`).join("|");
  if (parts.targets.dataset.ids !== targetIds) {
    parts.targets.dataset.ids = targetIds;
    parts.targets.replaceChildren(...overview.targets.map((target) => el("cds-multi-select-item", { value: target.id }, target.name)));
  }
  const names = session.targets.map((id) => (overview.targets.find((target) => target.id === id) || { name: t("common.gone") }).name);
  parts.targets.label = names.length ? names.join(t("common.separator")) : t("sessions.fields.chooseTargets");
  parts.targets.value = session.targets.join(",");
  const conditionIds = overview.conditions.map((c) => `${c.id}:${c.name}`).join("|");
  if (parts.condition.dataset.ids !== conditionIds) {
    parts.condition.dataset.ids = conditionIds;
    parts.condition.replaceChildren(
      el("cds-dropdown-item", { value: ALWAYS }, t("sessions.fields.always")),
      ...overview.conditions.map((c) => el("cds-dropdown-item", { value: c.id }, c.name)),
    );
  }
  parts.condition.value = session.condition || ALWAYS;
  // The allowance it has, in words, in place of the kind's own item.
  const kind = session.allowance && session.allowance.type;
  const words = session.allowance ? allowanceLabel(session.allowance) : "";
  const key = `${JSON.stringify(session.allowance)}|${words}`;
  if (parts.allowance.dataset.key !== key) {
    parts.allowance.dataset.key = key;
    parts.allowance.replaceChildren(
      el("cds-dropdown-item", { value: "none" }, t("sessions.fields.noAllowance")),
      el("cds-dropdown-item", { value: "budget" }, kind === "budget" ? words : t("sessions.fields.budget")),
      el("cds-dropdown-item", { value: "pomodoro" }, kind === "pomodoro" ? words : t("sessions.fields.pomodoro")),
    );
    // Set anew, so the dropdown shows the new words of the item that was chosen already.
    parts.allowance.value = "";
  }
  parts.allowance.value = kind || "none";
  for (const control of [parts.targets, parts.condition]) control.readOnly = locked;
  for (const control of [parts.newTarget, parts.newCondition]) control.disabled = locked;
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

export { addFields, renderFields, newSession, renameSession };
