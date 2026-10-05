// A page of tiles, one per block, block list or schedule: kept and updated in place, so an open menu
// survives a refresh. A block's tile also has its lock and Start / Stop. Deleting is asked here.

import { $, ask, el, invalid, invoke, listName, names, newId, notify, overview, quoted, reachable, refresh, runningOf, scheduleOf, t, usersOf, weekText } from "./core.js";
import { LOCKS } from "./locks.js";
import { ENFORCERS, LISTS, SECTIONS } from "./forms.js";
import { KINDS } from "./kinds.js";
import { detour } from "./detour.js";

// What a tile says, per kind: lines of text.
function summaryOf(kind, item) {
  if (kind === "lists") {
    const parts = [];
    for (const [key, list] of Object.entries(LISTS)) if (item[key] && item[key].length) parts.push(list.count(item[key].length));
    const on = ENFORCERS.filter((e) => item.enforcers && item.enforcers[e.key]);
    for (const e of on.filter((e) => !SECTIONS.find((s) => s.enforcers.includes(e)).lists.length)) parts.push(e.short || e.label);
    if (!parts.length) parts.push(t("lists.nothing"));
    const users = usersOf(item.id);
    parts.push(users.length ? t("common.usedBy", { names: names(users) }) : t("lists.notUsed"));
    return parts;
  }
  if (kind === "schedules") {
    const users = usersOf(item.id);
    return [t("schedules.active", { times: weekText(item.days) }), users.length ? t("common.usedBy", { names: names(users) }) : t("schedules.notUsed")];
  }
  // A block: what it runs with while it runs, else what it is defined as. Each line has an
  // icon in front: its block lists, its schedule, its lock (open when it does not lock), and
  // for a running block when its schedule turns it on or off.
  const running = runningOf(item.id);
  const lists = running ? running.lists.map((l) => l.name) : item.lists.map(listName);
  const parts = [["list-checked-16", lists.length ? lists.join(t("common.separator")) : t("blocks.tile.noLists")]];
  const schedule = scheduleOf(item);
  const days = running ? running.schedule && running.schedule.days : schedule && schedule.days;
  if (!days) parts.push(["calendar-16", t("blocks.tile.always")]);
  else if (schedule) parts.push(["calendar-16", t("blocks.tile.activeNamed", { times: weekText(days), schedule: schedule.name })]);
  else parts.push(["calendar-16", t("blocks.tile.active", { times: weekText(days) })]);
  if (!running) {
    const lock = LOCKS[item.lock && item.lock.method];
    parts.push([item.lock && item.lock.method !== "none" ? "locked" : "unlocked", lock ? lock.describe(item.lock) : t("blocks.tile.unknownLock")]);
    return parts;
  }
  const change = running.lock_change_words;
  const lockIcon = running.locked ? "locked" : "unlocked";
  if (running.password) parts.push([lockIcon, t("blocks.tile.lockedPassword")]);
  else if (running.until_restart) parts.push([lockIcon, t("blocks.tile.lockedRestart")]);
  else if (running.remaining_words) parts.push([lockIcon, t("blocks.tile.lockedTimer", { left: running.remaining_words })]);
  else if (running.lock_schedule && running.locked) parts.push([lockIcon, change ? t("blocks.tile.lockedSchedule", { left: change }) : t("blocks.tile.lockedScheduleOpen")]);
  else if (running.lock_schedule) parts.push([lockIcon, change ? t("blocks.tile.freeSchedule", { change }) : t("blocks.tile.freeScheduleOpen")]);
  else parts.push([lockIcon, t("blocks.tile.notLocked")]);
  const next = running.schedule_change_words;
  if (running.enforcing && next) parts.push(["time", t("blocks.tile.activeNow", { change: next })]);
  if (!running.enforcing) parts.push(["time", next ? t("blocks.tile.inactiveUntil", { change: next }) : t("blocks.tile.inactive")]);
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
  // In the menu's place while it cannot be changed: a running block, or a list or schedule one uses.
  const lock = el("span", { class: "lock", role: "img", "aria-label": t("common.locked"), title: t("common.locked") }, carbonIcon("locked"));
  const name = el("h2", { class: "cds--type-productive-heading-03" });
  const tag = el("cds-tag", { size: "md" });
  const summary = el("div", { class: "summary cds--type-body-compact-01" });
  const actions = el("div", { class: "actions" });
  const tile = el(
    "cds-tile",
    { class: "block-tile cds--css-grid-column cds--sm:col-span-4 cds--md:col-span-8 cds--lg:col-span-16" },
    el("div", { class: "top" }, el("div", {}, name), menu, lock),
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
  const parts = { tile, name, tag, summary, actions, menu, lock, lockItem: menu.querySelector('[data-action="lock"]') };
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
        ...summary.map((line) => (Array.isArray(line) ? el("span", { class: "line" }, carbonIcon(line[0]), el("span", {}, line[1])) : el("span", {}, line))),
      );
    }
    const held = kindOf.held(item);
    parts.menu.hidden = !!held;
    parts.lock.hidden = !parts.menu.hidden;
    if (kind === "blocks") renderBlockControls(parts, item);
    else {
      parts.tag.hidden = !held;
      parts.tag.setAttribute("type", "blue");
      parts.tag.textContent = t("common.inUse");
    }
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
  method.addEventListener("cds-select-selected", () => setLock(id, method.value));
  toggle.addEventListener("click", () => startOrStop(id));
}

// Sets a block's lock to `method`: what the method needs is asked for in the modal, and the
// lock is saved with the block once it is given. Cancel leaves the lock as it was.
async function setLock(id, method) {
  const parts = tiles.get(`blocks:${id}`);
  const block = overview.blocks.find((b) => b.id === id);
  if (!block) return;
  // A lock schedule needs a schedule: without one, it offers to make one, and the lock is then
  // set to it; otherwise the lock stays as it was.
  if (method === "scheduled" && !overview.schedules.length) {
    parts.choosing = true;
    const make = await ask({
      heading: t("locks.scheduled.noneHeading"),
      body: t("locks.scheduled.noneBody"),
      action: t("locks.scheduled.noneAction"),
      kind: "primary",
    });
    parts.choosing = false;
    parts.method.value = block.lock.method;
    if (make) {
      detour.current = {
        back: KINDS.blocks.path,
        crumbs: [{ label: KINDS.blocks.title, href: KINDS.blocks.path }, { label: block.name, href: KINDS.blocks.path }],
        onSaved: async (schedule) => {
          const now = overview.blocks.find((b) => b.id === id) || block;
          await invoke("save_block", { block: { ...now, lock: { method: "scheduled", schedule: schedule.id } } }).catch((e) =>
            notify("list-notices", "error", t("blocks.lockFailed", { name: quoted(block.name) }), String(e)),
          );
        },
      };
      location.hash = `#/lock/${encodeURIComponent(id)}/schedules/new`;
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
  parts.lockItem.hidden = !LOCKS[parts.method.value] || !LOCKS[parts.method.value].form;
  const busy = parts.toggle.dataset.busy === "1";
  const action = running ? "stop" : "start";
  if (parts.toggle.dataset.action !== action) {
    parts.toggle.dataset.action = action;
    parts.toggle.setAttribute("kind", running ? "secondary" : "primary");
    parts.toggle.replaceChildren(t(running ? "blocks.stop" : "blocks.start"), carbonIcon(running ? "stop" : "play", "icon"));
  }
  // A block locked with a password stops with it (asked for on Stop).
  parts.toggle.disabled = busy || (running && running.locked && !running.password) || !reachable;
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
  parts.toggle.dataset.busy = "1";
  parts.toggle.disabled = true;
  try {
    if (running) await invoke("stop_block", { id, password });
    else await invoke("start_block", { id });
    notify("list-notices");
  } catch (e) {
    notify("list-notices", "error", t(running ? "blocks.stopFailed" : "blocks.startFailed", { name: quoted(block.name) }), String(e));
  }
  parts.toggle.dataset.busy = "";
  await refresh();
}

// What deleting a block list or schedule does to the blocks that use it, in sentences: the
// service's rule (`deletion_fallout`, `Library::fallout`), worded by strings.js.
async function fallout(id) {
  const effects = await invoke("deletion_fallout", { blocks: overview.blocks, id });
  const sentences = { deleted: "deletesToo", without_it: "keepsOthers", always: "becomesAlways", unlocked: "losesLock" };
  return Object.entries(sentences)
    .filter(([field]) => effects[field].length)
    .map(([field, key]) => t(`delete.${key}`, { names: effects[field].map(quoted).join(t("common.separator")) }));
}

async function confirmDelete(kind, id) {
  const kindOf = KINDS[kind];
  const item = kindOf.items().find((i) => i.id === id);
  if (!item) return;
  const sure = await ask({
    heading: t("delete.heading", { name: quoted(item.name), noun: kindOf.noun }),
    body: [t("delete.body", { name: quoted(item.name), noun: kindOf.noun }), ...(kind === "blocks" ? [] : await fallout(id))].join(" "),
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
