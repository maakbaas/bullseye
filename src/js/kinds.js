// The four kinds the user makes, each with a page of tiles and an editor: blocks, block lists,
// conditions and allowances. Pages, tiles and the editor are the same code for all, by this table.

import { invoke, lockedUsersOf, names, newId, overview, runningOf, t } from "./core.js";
import { ENFORCERS, LISTS, allowanceForm, blockForm, conditionForm, listForm } from "./forms.js";

// The first free automatic name of a kind: "Block 1", "Block 2", … (`<kind>.autoName`), ignoring case.
function freeName(kind, items) {
  const taken = new Set(items.map((item) => item.name.trim().toLowerCase()));
  for (let n = 1; ; n++) {
    const name = t(`${kind}.autoName`, { n });
    if (!taken.has(name.toLowerCase())) return name;
  }
}

// ---- The four kinds. Each has a page of tiles and an editor.

// A kind's texts in strings.js: its page, its editor and its empty state.
const kindTexts = (kind) => ({
  title: t(`${kind}.title`),
  noun: t(`${kind}.noun`),
  description: t(`${kind}.description`),
  newLabel: t(`${kind}.newLabel`),
  editTitle: t(`${kind}.editTitle`),
  empty: [t(`${kind}.emptyTitle`), t(`${kind}.emptyText`)],
  editorDescription: t(`${kind}.editorDescription`),
});

const KINDS = {
  blocks: {
    path: "#/",
    editPath: "#/edit/",
    newPath: "#/new",
    ...kindTexts("blocks"),
    items: () => overview.blocks,
    create: () => ({ id: newId(), name: freeName("blocks", overview.blocks), lists: [], condition: null, allowance: null, lock: { method: "none" } }),
    save: (block) => invoke("save_block", { block }),
    remove: (id) => invoke("delete_block", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "block", id, newId: copyId }),
    // Why it cannot be changed now, or null.
    // A running block can be changed, its lock aside, unless it is locked; it cannot be deleted.
    held: (block) => (runningOf(block.id) && runningOf(block.id).locked ? t("blocks.held") : null),
    limited: () => null,
    // Whether something locked stands in the way of deleting it (a running block that is not locked
    // ends with it).
    inUse: (block) => !!(runningOf(block.id) && runningOf(block.id).locked),
    form: () => blockForm(),
  },
  lists: {
    path: "#/lists",
    editPath: "#/lists/edit/",
    newPath: "#/lists/new",
    ...kindTexts("lists"),
    items: () => overview.lists,
    create: () => ({
      id: newId(),
      name: freeName("lists", overview.lists),
      ...Object.fromEntries(Object.keys(LISTS).map((key) => [key, []])),
      enforcers: Object.fromEntries(ENFORCERS.map((e) => [e.key, true])),
    }),
    save: (list) => invoke("save_list", { list }),
    remove: (id) => invoke("delete_list", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "list", id, newId: copyId }),
    // What a locked block uses can only get stricter (the service holds to it); what a block uses
    // that is running but not locked can be changed or deleted, and follows.
    held: () => null,
    limited: (list) => {
      const holders = lockedUsersOf(list.id);
      return holders.length ? t("lists.limited", { names: names(holders) }) : null;
    },
    inUse: (list) => lockedUsersOf(list.id).length > 0,
    form: () => listForm(),
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
    // A flash drive can only stay the same drive, so while a locked block uses it there is nothing to change.
    held: (condition) => {
      if (condition.type !== "flash_drive") return null;
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
  allowances: {
    path: "#/allowances",
    editPath: "#/allowances/edit/",
    newPath: "#/allowances/new",
    ...kindTexts("allowances"),
    items: () => overview.allowances,
    create: () => ({ id: newId(), name: freeName("allowances", overview.allowances), type: "budget", allowed: "30m", scope: "total", reset: "day" }),
    save: (allowance) => invoke("save_allowance", { allowance }),
    remove: (id) => invoke("delete_allowance", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "allowance", id, newId: copyId }),
    // Nothing is read-only: while a locked block has it, the service only takes stricter terms.
    held: () => null,
    limited: (allowance) => {
      const holders = lockedUsersOf(allowance.id);
      return holders.length ? t("allowances.limited", { names: names(holders) }) : null;
    },
    inUse: (allowance) => lockedUsersOf(allowance.id).length > 0,
    form: () => allowanceForm(),
  },
};

export { KINDS };
