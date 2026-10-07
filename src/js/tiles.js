// A page of tiles, one per block, block list or condition: kept and updated in place, so an open menu
// survives a refresh. A block's tile also has its lock and Start / Stop. Deleting is asked here.

import { $, ask, el, invalid, invoke, joined, listName, newId, editHref, nodesOf, notify, overview, quoted, reachable, ref, refresh, runningOf, conditionOf, t, tn, usersOf, weekText, wordingOf } from "./core.js";
import { LOCKS, conditionLockText } from "./locks.js";
import { ENFORCERS, LISTS, SECTIONS } from "./forms.js";
import { KINDS } from "./kinds.js";
import { detour } from "./detour.js";

// A condition's line on a block's tile, with its name when the block's definition still has it.
// "Active during <name>", for a drive "Active while <name> is unplugged", for a combination with a
// drive in it "Active while <name> is true" (`wordingOf`, as a lock's line is worded), the name a
// link to its condition; without it (a running block keeps its rule) the drive's own name or a plain
// word for the kind.
const ruleLine = (rule, condition) => {
  const icon = "rule-16"; // the Conditions tab's, whatever the kind
  const wording = wordingOf(rule);
  if (condition) {
    const name = ref(condition.name, editHref("conditions", condition.id));
    return [icon, tn(`blocks.tile.${{ drive: "activeDriveNamed", true: "activeCombinedNamed", during: "activeNamed" }[wording]}`, { name })];
  }
  if (rule.type === "flash_drive") return [icon, t("blocks.tile.activeDrive", { drive: quoted(rule.drive) })];
  return [icon, t(rule.type === "combined" ? "blocks.tile.activeCombined" : "blocks.tile.active")];
};

// A running block's condition line: the rule as `ruleLine` words it, and whether it enforces now, as a
// lock's line says its state: "Active by <name> for the next 3 hours" (the time when the condition has
// a next change), a drive "Active while <name> is unplugged" / "Inactive while <name> is plugged in".
// The tag says the same in a word, so there is no line of its own for it.
const runningRuleLine = (running, rule, condition) => {
  const on = running.enforcing;
  const change = running.condition_change_words;
  const name = condition
    ? ref(condition.name, editHref("conditions", condition.id))
    : rule.type === "flash_drive"
      ? quoted(rule.drive)
      : t("blocks.tile.conditionGone");
  if (rule.type === "flash_drive") return ["rule-16", tn(`blocks.tile.${on ? "activeDriveNamed" : "inactiveDriveNamed"}`, { name })];
  return ["rule-16", tn(`blocks.tile.${on ? "active" : "inactive"}By${change ? "For" : ""}`, { name, change })];
};

// The blocks that use an item, as links.
const usedBy = (users) => ({ text: tn("common.usedBy", { n: users.length, names: joined(users.map((b) => ref(b.name, editHref("blocks", b.id)))) }) });

// What a tile says, per kind: lines of text.
function summaryOf(kind, item) {
  if (kind === "lists") {
    const parts = [];
    for (const [key, list] of Object.entries(LISTS)) if (item[key] && item[key].length) parts.push(list.count(item[key].length));
    const on = ENFORCERS.filter((e) => item.enforcers && item.enforcers[e.key]);
    for (const e of on.filter((e) => !SECTIONS.find((s) => s.enforcers.includes(e)).lists.length)) parts.push(e.short || e.label);
    if (!parts.length) parts.push(t("lists.nothing"));
    const users = usersOf(item.id);
    parts.push(users.length ? usedBy(users) : t("lists.notUsed"));
    return parts;
  }
  if (kind === "conditions") {
    const users = usersOf(item.id);
    let what;
    if (item.type === "combined") {
      const members = item.members.map((id) => {
        const member = overview.conditions.find((c) => c.id === id);
        return member ? ref(member.name, editHref("conditions", id)) : t("common.gone");
      });
      what = { text: tn(item.mode === "any" ? "conditions.activeAny" : "conditions.activeAll", { names: joined(members) }) };
    } else what = item.type === "flash_drive" ? t("conditions.activeDrive", { drive: quoted(item.drive) }) : t("conditions.active", { times: weekText(item.days) });
    return [what, users.length ? usedBy(users) : t("conditions.notUsed")];
  }
  // A block: what it runs with while it runs, else what it is defined as. Each line has an
  // icon in front: its block lists, its condition, its lock (open when it does not lock), and
  // for a running block when its condition turns it on or off.
  const running = runningOf(item.id);
  // A running block's definition is fixed, so its block lists are the ones it was started with.
  const lists = item.lists.map((id) => (overview.lists.some((l) => l.id === id) ? ref(listName(id), editHref("lists", id)) : listName(id)));
  const parts = [["list-checked-16", lists.length ? joined(lists) : t("blocks.tile.noLists")]];
  const condition = conditionOf(item);
  const rule = running ? running.condition : condition;
  if (!rule) parts.push(["rule-16", t("blocks.tile.always")]);
  else parts.push(running ? runningRuleLine(running, rule, condition) : ruleLine(rule, condition));
  if (!running) {
    const lock = LOCKS[item.lock && item.lock.method];
    parts.push([item.lock && item.lock.method !== "none" ? "locked" : "unlocked", lock ? lock.describe(item.lock) : t("blocks.tile.unknownLock")]);
    return parts;
  }
  const change = running.lock_change_words;
  const lockRule = running.lock_condition;
  if (running.password) parts.push(["locked", t("blocks.tile.lockedPassword")]);
  else if (running.lock_text) parts.push(["locked", t("blocks.tile.lockedText")]);
  else if (running.until_restart) parts.push(["locked", t("blocks.tile.lockedRestart")]);
  else if (running.remaining_words) parts.push(["locked", t("blocks.tile.lockedFor", { left: running.remaining_words })]);
  else if (lockRule) {
    // A condition lock is one line: what it is, as the condition's own line is worded, and the state
    // it is in now (the icon, and its words), with when that changes if it is known.
    const lockCondition = overview.conditions.find((c) => c.id === (item.lock && item.lock.condition));
    const name = lockCondition
      ? ref(lockCondition.name, editHref("conditions", lockCondition.id))
      : lockRule.type === "flash_drive"
        ? quoted(lockRule.drive)
        : t("locks.condition.goneName");
    parts.push([running.locked ? "locked" : "unlocked", conditionLockText(wordingOf(lockRule), name, { locked: running.locked, change })]);
  } else parts.push(["unlocked", t("blocks.tile.notLocked")]);
  return parts;
}

// ---- A page of tiles. Tiles are kept and updated in place, so an open menu or a lock being
// typed survives a refresh.

const tiles = new Map(); // `${kind}:${id}` -> its parts

function makeTile(kind, id) {
  // The class is how Carbon colours an overflow menu's icon (without it: white on a white tile).
  const menuIcon = carbonIcon("overflow-menu-vertical", "icon");
  menuIcon.classList.add("cds--overflow-menu__icon");
  const menu = el(
    "cds-overflow-menu",
    { "aria-label": t("common.options"), "menu-alignment": "bottom-end" },
    menuIcon,
    el("span", { slot: "tooltip-content" }, t("common.options")),
    el(
      "cds-overflow-menu-body",
      { flipped: true },
      el("cds-overflow-menu-item", { "data-action": "edit" }, t("common.edit")),
      el("cds-overflow-menu-item", { "data-action": "duplicate" }, t("common.duplicate")),
      // A block's lock settings (a timer's length, …); the method is the tile's select.
      kind === "blocks" ? el("cds-overflow-menu-item", { "data-action": "lock" }, t("blocks.lockSettings")) : null,
      el("cds-overflow-menu-item", { "data-action": "delete", danger: true }, t("common.delete")),
    ),
  );
  // In the menu's place while it cannot be changed: a block that is locked.
  const lock = el("span", { class: "lock", role: "img", "aria-label": t("common.locked"), title: t("common.locked") }, carbonIcon("locked"));
  // The title is a link to the editor, as the names inside the tiles' lines are.
  // An edit icon after it shows on hover and focus (`app.css`).
  const name = el("span");
  const editIcon = carbonIcon("edit", "icon");
  editIcon.classList.add("title-edit");
  const title = el("h2", { class: "cds--type-productive-heading-03" }, el("cds-link", { href: editHref(kind, id) }, name, editIcon));
  const tag = el("cds-tag", { size: "md" });
  const summary = el("div", { class: "summary cds--type-body-compact-01" });
  const actions = el("div", { class: "actions" });
  const tile = el(
    "cds-tile",
    { class: "block-tile cds--css-grid-column cds--sm:col-span-4 cds--md:col-span-8 cds--lg:col-span-16" },
    el("div", { class: "top" }, el("div", {}, title), menu, lock),
    el("div", { class: "body" }, el("div", { class: "info" }, tag, summary), actions),
  );
  // On the items themselves: Carbon moves the open menu out of the tile.
  for (const item of menu.querySelectorAll("[data-action]")) {
    item.addEventListener("click", () => {
      if (item.dataset.action === "edit") location.hash = KINDS[kind].editPath + encodeURIComponent(id);
      if (item.dataset.action === "duplicate") duplicate(kind, id);
      if (item.dataset.action === "lock") setLock(id, tiles.get(`${kind}:${id}`).method.value);
      if (item.dataset.action === "delete") confirmDelete(kind, id);
    });
  }
  const parts = { tile, name, tag, summary, actions, menu, lock, lockItem: menu.querySelector('[data-action="lock"]'), deleteItem: menu.querySelector('[data-action="delete"]') };
  if (kind === "blocks") addBlockControls(parts, id);
  tiles.set(`${kind}:${id}`, parts);
  return parts;
}

function renderTiles(kind) {
  const container = $("tiles");
  const kindOf = KINDS[kind];
  const shown = kindOf.items();
  const wanted = new Set(shown.map((item) => `${kind}:${item.id}`));
  for (const [key, parts] of tiles) {
    if (!wanted.has(key)) {
      parts.tile.remove();
      tiles.delete(key);
    }
  }
  shown.forEach((item, index) => {
    const parts = tiles.get(`${kind}:${item.id}`) || makeTile(kind, item.id);
    if (container.children[index] !== parts.tile) container.insertBefore(parts.tile, container.children[index] || null);
    parts.name.textContent = item.name;
    // A line is its text, or [icon, text].
    const summary = summaryOf(kind, item);
    const text = JSON.stringify(summary);
    if (parts.summary.dataset.text !== text) {
      parts.summary.dataset.text = text;
      parts.summary.replaceChildren(
        ...summary.map((line) =>
          Array.isArray(line)
            ? el("span", { class: "line" }, carbonIcon(line[0]), el("span", {}, ...nodesOf(line[1])))
            : el("span", {}, ...nodesOf(typeof line === "string" ? line : line.text)),
        ),
      );
    }
    const held = kindOf.held(item);
    parts.menu.hidden = !!held;
    parts.lock.hidden = !parts.menu.hidden;
    if (kind === "blocks") renderBlockControls(parts, item);
    else {
      // Tagged while a locked block uses it (it can then only get stricter); a running block
      // that is not locked leaves it free to change, but not to delete.
      parts.tag.hidden = !kindOf.limited(item);
      parts.tag.setAttribute("type", "blue");
      parts.tag.textContent = t("common.inUse");
    }
    parts.deleteItem.disabled = kindOf.inUse(item);
  });
  $("empty").hidden = shown.length > 0;
}

// ---- A block's tile: its state, its lock (set here, next to Start) and Start / Stop.

function addBlockControls(parts, id) {
  const method = el("cds-select", { "label-text": t("blocks.lockLabel"), size: "sm", class: "lock-method" });
  for (const [key, lock] of Object.entries(LOCKS)) method.append(el("cds-select-item", { value: key, label: lock.label }, lock.label));
  const toggle = el("cds-button", { kind: "primary", size: "sm" });
  const controls = el("div", { class: "lock-controls" }, method);
  parts.actions.append(controls, toggle);
  Object.assign(parts, { method, toggle, controls });
  // Choosing the lock the block already has changes nothing, so the select says nothing and its
  // settings could not be opened that way. While its list is open the select is therefore blank,
  // so any choice is a change; it goes back to the block's lock when the list closes without one.
  let had = null;
  const restore = () => {
    if (had === null) return;
    method.value = had;
    had = null;
    parts.choosing = false;
  };
  method.addEventListener("pointerdown", () => {
    if (had !== null) return setTimeout(restore, 0);
    if (!method.value || parts.choosing) return;
    had = method.value;
    parts.choosing = true;
    method.value = "";
  });
  method.addEventListener("focusout", restore);
  method.addEventListener("keydown", (event) => event.key === "Escape" && restore());
  method.addEventListener("cds-select-selected", () => {
    if (!method.value) return;
    had = null;
    setLock(id, method.value);
  });
  toggle.addEventListener("click", () => startOrStop(id));
}

// Sets a block's lock to `method`: what the method needs is asked for in the modal, and the
// lock is saved with the block once it is given. Cancel leaves the lock as it was.
async function setLock(id, method) {
  const parts = tiles.get(`blocks:${id}`);
  const block = overview.blocks.find((b) => b.id === id);
  if (!block) return;
  // A condition lock needs a condition: without one, it offers to make one, and the lock is then
  // set to it; otherwise the lock stays as it was.
  if (method === "condition" && !overview.conditions.length) {
    parts.choosing = true;
    const make = await ask({
      heading: t("locks.condition.noneHeading"),
      body: t("locks.condition.noneBody"),
      action: t("locks.condition.noneAction"),
      kind: "primary",
    });
    parts.choosing = false;
    parts.method.value = block.lock.method;
    if (make) {
      detour.current = {
        back: KINDS.blocks.path,
        crumbs: [{ label: KINDS.blocks.title, href: KINDS.blocks.path }, { label: block.name, href: KINDS.blocks.path }],
        onSaved: async (condition) => {
          const now = overview.blocks.find((b) => b.id === id) || block;
          await invoke("save_block", { block: { ...now, lock: { method: "condition", condition: condition.id } } }).catch((e) =>
            notify("list-notices", "error", t("blocks.lockFailed", { name: quoted(block.name) }), String(e)),
          );
        },
      };
      location.hash = `#/lock/${encodeURIComponent(id)}/conditions/new`;
    }
    return;
  }
  const form = LOCKS[method].form && LOCKS[method].form(block);
  let lock = { method };
  if (form) {
    // Kept on the tile while the dialog is open, though the block has the old lock still.
    parts.choosing = true;
    const heading = t("blocks.lockHeading", { method: LOCKS[method].label, name: quoted(block.name) });
    lock = await ask({ heading, action: t("common.save"), kind: "primary", fields: form.fields, read: form.read });
  }
  if (lock) {
    try {
      await invoke("save_block", { block: { ...block, lock } });
      notify("list-notices");
    } catch (e) {
      notify("list-notices", "error", t("blocks.lockFailed", { name: quoted(block.name) }), String(e));
    }
  }
  // Cancelled or refused: the tile shows the lock the block has.
  parts.choosing = false;
  parts.method.value = block.lock.method;
  await refresh();
}

function renderBlockControls(parts, block) {
  const running = runningOf(block.id);
  const quiet = running && running.enforcing === false;
  parts.tag.hidden = false;
  parts.tag.setAttribute("type", running ? (quiet ? "cool-gray" : running.locked ? "green" : "blue") : "gray");
  parts.tag.textContent = t(`blocks.state.${running ? (quiet ? "inactive" : running.locked ? "runningLocked" : "running") : "notRunning"}`);
  // The lock is set while the block is stopped; a running block's is in its summary.
  parts.controls.hidden = !!running;
  if (block.lock && !parts.choosing && parts.method.value !== block.lock.method) parts.method.value = block.lock.method;
  // Settings to change (in the menu): all locks but No lock.
  parts.lockItem.hidden = !!running || !LOCKS[parts.method.value] || !LOCKS[parts.method.value].form;
  const busy = parts.toggle.dataset.busy === "1";
  const action = running ? "stop" : "start";
  if (parts.toggle.dataset.action !== action) {
    parts.toggle.dataset.action = action;
    parts.toggle.setAttribute("kind", running ? "secondary" : "primary");
    parts.toggle.replaceChildren(t(running ? "blocks.stop" : "blocks.start"), carbonIcon(running ? "stop" : "play", "icon"));
  }
  // A block locked with a password stops with it, one locked with a text by typing it (asked for on Stop).
  parts.toggle.disabled = busy || (running && running.locked && !running.password && !running.lock_text) || !reachable;
}

// The dialog of a text lock: the text, and an input to type it into. Pasting and dropping text
// into the input are refused, so the text has to be typed; the dialog only goes on when what is
// typed is exactly the text (the service checks it again). Resolves the typed text, or null.
async function askText(block, expected) {
  const shown = el("p", { class: "typed-text cds--type-body-01" }, expected);
  const field = el("cds-textarea", { label: t("blocks.stopTextLabel"), rows: 4, autocomplete: "off", spellcheck: "false" });
  const refuse = (event) => event.preventDefault();
  for (const name of ["paste", "drop"]) field.addEventListener(name, refuse, true);
  field.addEventListener("beforeinput", (event) => (/^insertFrom(Paste|Drop|Yank)/.test(event.inputType) ? refuse(event) : null), true);
  field.addEventListener("input", () => invalid(field, null));
  setTimeout(() => field.focus(), 150);
  const same = (a, b) => a.replace(/\r\n/g, "\n") === b.replace(/\r\n/g, "\n");
  const given = await ask({
    heading: t("blocks.stopHeading", { name: quoted(block.name) }),
    body: t("blocks.stopTextBody"),
    action: t("blocks.stop"),
    kind: "danger",
    fields: [shown, field],
    read: async () => (invalid(field, same(field.value, expected) ? null : t("blocks.stopTextDiffers")) ? null : field.value),
  });
  return given || null;
}

async function startOrStop(id) {
  const parts = tiles.get(`blocks:${id}`);
  const running = runningOf(id);
  const block = overview.blocks.find((b) => b.id === id);
  // A block that cannot be stopped is only started after a second, deliberate click.
  const lock = !running && LOCKS[block.lock.method];
  const question = lock && (await lock.confirm(block));
  if (question && !(await ask({ ...question, kind: "primary" }))) return;
  // A block locked with a password stops with it.
  let password = null;
  if (running && running.password) {
    const field = el("cds-password-input", { label: t("blocks.stopPassword"), autocomplete: "off" });
    const given = await ask({
      heading: t("blocks.stopHeading", { name: quoted(block.name) }),
      body: t("blocks.stopBody"),
      action: t("blocks.stop"),
      kind: "danger",
      fields: [field],
      read: async () => (invalid(field, field.value ? null : t("blocks.stopPasswordMissing")) ? null : field.value),
    });
    if (!given) return;
    password = given;
  }
  // A block locked with a text stops by typing it in.
  let text = null;
  if (running && running.lock_text) {
    text = await askText(block, running.lock_text);
    if (text === null) return;
  }
  parts.toggle.dataset.busy = "1";
  parts.toggle.disabled = true;
  try {
    if (running) await invoke("stop_block", { id, password, text });
    else await invoke("start_block", { id });
    notify("list-notices");
  } catch (e) {
    notify("list-notices", "error", t(running ? "blocks.stopFailed" : "blocks.startFailed", { name: quoted(block.name) }), String(e));
  }
  parts.toggle.dataset.busy = "";
  await refresh();
}

// What deleting a block list or condition does to the blocks that use it, in sentences: the
// service's rule (`deletion_fallout`, `Library::fallout`), worded by strings.js.
async function fallout(id) {
  const effects = await invoke("deletion_fallout", { blocks: overview.blocks, conditions: overview.conditions, id });
  const sentences = { deleted: "deletesToo", without_it: "keepsOthers", always: "becomesAlways", unlocked: "losesLock", shrinks: "shrinksCombination", removed: "removesCombination" };
  return Object.entries(sentences)
    .filter(([field]) => effects[field].length)
    .map(([field, key]) => t(`delete.${key}`, { names: joined(effects[field].map(quoted)).join("") }));
}

async function confirmDelete(kind, id) {
  const kindOf = KINDS[kind];
  const item = kindOf.items().find((i) => i.id === id);
  if (!item) return;
  const sure = await ask({
    heading: t("delete.heading", { name: quoted(item.name), noun: kindOf.noun }),
    body: [
      t("delete.body", { name: quoted(item.name), noun: kindOf.noun }),
      ...(kind === "blocks" ? (runningOf(id) ? [t("delete.stopsIt")] : []) : await fallout(id)),
    ].join(" "),
    action: t("common.delete"),
    kind: "danger",
  });
  if (!sure) return;
  const inEditor = !$("view-editor").hidden;
  try {
    await kindOf.remove(id);
    notify("list-notices");
    if (inEditor) location.hash = kindOf.path;
  } catch (e) {
    notify(inEditor ? "editor-notices" : "list-notices", "error", t("delete.failed", { noun: kindOf.noun }), String(e));
    if (inEditor) $("editor-notices").dataset.lock = "error";
  }
  await refresh();
}

// A copy, made and named by the service; it comes at the end of the page.
async function duplicate(kind, id) {
  const kindOf = KINDS[kind];
  const original = kindOf.items().find((i) => i.id === id);
  if (!original) return;
  const copyId = newId();
  try {
    await kindOf.duplicate(id, copyId);
    await refresh();
    const copy = kindOf.items().find((i) => i.id === copyId);
    notify("list-notices", "success", t("common.duplicated", { name: quoted(copy ? copy.name : ""), original: quoted(original.name) }));
  } catch (e) {
    notify("list-notices", "error", t("common.duplicateFailed", { name: quoted(original.name) }), String(e));
  }
}

export { tiles, renderTiles, confirmDelete };
