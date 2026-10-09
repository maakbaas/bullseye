// The three kinds the user makes, each with a page of tiles: sessions, targets and conditions;
// targets and conditions also with an editor. Pages, tiles and the editor are the same code for
// all, by this table. (A session is set up on its tile, `fields.js`: no editor of its own.)

import { invoke, lockedUsersOf, names, newId, overview, runningOf, t } from "./core.js";
import { LISTS, SECTIONS, conditionForm, targetForm } from "./forms.js";

// The first free automatic name of a kind: "Session 1", "Session 2", … (`<kind>.autoName`), ignoring case.
function freeName(kind, items) {
  const taken = new Set(items.map((item) => item.name.trim().toLowerCase()));
  for (let n = 1; ; n++) {
    const name = t(`${kind}.autoName`, { n });
    if (!taken.has(name.toLowerCase())) return name;
  }
}

// ---- The three kinds. Each has a page of tiles and an editor.

// A kind's texts in strings.js: its page, its editor and its empty state.
const kindTexts = (kind, editor = true) => ({
  title: t(`${kind}.title`),
  noun: t(`${kind}.noun`),
  description: t(`${kind}.description`),
  newLabel: t(`${kind}.newLabel`),
  empty: [t(`${kind}.emptyTitle`), t(`${kind}.emptyText`)],
  ...(editor ? { editTitle: t(`${kind}.editTitle`), editorDescription: t("common.editorDescription") } : {}),
});

const KINDS = {
  sessions: {
    path: "#/",
    // A link to a session opens its tile (`main.js`); a new one is asked in the modal (`newSession`).
    editPath: "#/edit/",
    ...kindTexts("sessions", false),
    items: () => overview.sessions,
    remove: (id) => invoke("delete_session", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "session", id, newId: copyId }),
    // Set up on its tile, which keeps what a locked session may not change read-only (`fields.js`).
    held: () => null,
    limited: () => null,
    // Whether something locked stands in the way of deleting it (a running session that is not locked
    // ends with it).
    inUse: (session) => !!(runningOf(session.id) && runningOf(session.id).locked),
  },
  targets: {
    path: "#/targets",
    editPath: "#/targets/edit/",
    newPath: "#/targets/new",
    ...kindTexts("targets"),
    items: () => overview.targets,
    create: () => ({
      id: newId(),
      name: freeName("targets", overview.targets),
      ...Object.fromEntries(Object.keys(LISTS).map((key) => [key, []])),
      // Every checkbox ticked; a choice (windows by title) at its first option, closing.
      actions: Object.fromEntries(SECTIONS.flatMap((s) => s.actions.map((e) => [e.key, !s.choice]))),
    }),
    save: (list) => invoke("save_target", { target: list }),
    remove: (id) => invoke("delete_target", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "target", id, newId: copyId }),
    // What a locked session uses can only get stricter (the service holds to it); what a session uses
    // that is running but not locked can be changed or deleted, and follows.
    held: () => null,
    limited: (list) => {
      const holders = lockedUsersOf(list.id);
      return holders.length ? t("targets.limited", { names: names(holders) }) : null;
    },
    inUse: (list) => lockedUsersOf(list.id).length > 0,
    form: () => targetForm(),
  },
  conditions: {
    path: "#/conditions",
    editPath: "#/conditions/edit/",
    newPath: "#/conditions/new",
    ...kindTexts("conditions"),
    items: () => overview.conditions,
    create: () => ({ id: newId(), name: freeName("conditions", overview.conditions), type: "schedule", days: [0, 1, 2, 3, 4, 5, 6].map(() => []) }),
    save: (condition) => invoke("save_condition", { condition }),
    remove: (id) => invoke("delete_condition", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "condition", id, newId: copyId }),
    // A flash drive can only stay the same drive, so while a locked session uses it there is nothing to change.
    held: (condition) => {
      if (condition.type !== "flash_drive" && condition.type !== "webhook") return null;
      const holders = lockedUsersOf(condition.id);
      return holders.length ? t("conditions.held", { names: names(holders) }) : null;
    },
    limited: (condition) => {
      const holders = lockedUsersOf(condition.id);
      return holders.length ? t("conditions.limited", { names: names(holders) }) : null;
    },
    inUse: (condition) => lockedUsersOf(condition.id).length > 0,
    form: () => conditionForm(),
  },
};

export { KINDS, freeName };
