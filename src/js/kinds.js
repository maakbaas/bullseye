// The three kinds the user makes, each with a page of tiles and an editor: blocks, block lists and
// conditions. Pages, tiles and the editor are the same code for all three, by this table.

import { holdersOf, invoke, names, newId, overview, runningOf, t } from "./core.js";
import { ENFORCERS, LISTS, blockForm, conditionForm, listForm } from "./forms.js";

// ---- The three kinds. Each has a page of tiles and an editor.

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
    create: () => ({ id: newId(), name: "", lists: [], condition: null, lock: { method: "none" } }),
    save: (block) => invoke("save_block", { block }),
    remove: (id) => invoke("delete_block", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "block", id, newId: copyId }),
    // Why it cannot be changed now, or null.
    held: (block) => (runningOf(block.id) ? t("blocks.held") : null),
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
      name: "",
      ...Object.fromEntries(Object.keys(LISTS).map((key) => [key, []])),
      enforcers: Object.fromEntries(ENFORCERS.map((e) => [e.key, true])),
    }),
    save: (list) => invoke("save_list", { list }),
    remove: (id) => invoke("delete_list", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "list", id, newId: copyId }),
    held: (list) => {
      const holders = holdersOf(list.id);
      return holders.length ? t("lists.held", { names: names(holders) }) : null;
    },
    form: () => listForm(),
  },
  conditions: {
    path: "#/conditions",
    editPath: "#/conditions/edit/",
    newPath: "#/conditions/new",
    ...kindTexts("conditions"),
    items: () => overview.conditions,
    create: () => ({ id: newId(), name: "", type: "schedule", days: [0, 1, 2, 3, 4, 5, 6].map(() => []) }),
    save: (condition) => invoke("save_condition", { condition }),
    remove: (id) => invoke("delete_condition", { id }),
    duplicate: (id, copyId) => invoke("duplicate", { kind: "condition", id, newId: copyId }),
    held: (condition) => {
      const holders = holdersOf(condition.id);
      return holders.length ? t("conditions.held", { names: names(holders) }) : null;
    },
    form: () => conditionForm(),
  },
};

export { KINDS };
