// A page of tiles, one per session, target or condition: kept and updated in place, so an open menu
// survives a refresh. A session's tile also has its lock and Start / Stop. Deleting is asked here.

import { $, ask, confirmStricter, el, invalid, invoke, joined, targetName, newId, editHref, nodesOf, notify, overview, quoted, reachable, ref, refresh, runningOf, conditionOf, t, tn, usersOf, weekText } from "./core.js";
import { LOCKS } from "./locks.js";
import { LISTS, SECTIONS } from "./forms.js";
import { KINDS } from "./kinds.js";
import { detour } from "./detour.js";
import { addRows, renderRows, renameSession } from "./fields.js";

// A running session's condition when its definition no longer has it (it keeps the rule it started
// with): the drive's own name, or a word for the kind.
function ruleName(rule) {
  if (rule.type === "flash_drive") return t("sessions.tile.activeDrive", { drive: quoted(rule.drive) });
  return t(rule.type === "combined" ? "sessions.tile.activeCombined" : rule.type === "webhook" ? "sessions.tile.activeWebhook" : "sessions.tile.active");
}

// The sessions that use an item, as links; or a word that none does.
const usersText = (item) => {
  const users = usersOf(item.id);
  return users.length ? joined(users.map((b) => ref(b.name, editHref("sessions", b.id)))) : t("tiles.none");
};

// A target or condition tile's rows, as a session's are (`fields.js`) but only to read: [icon, label,
// value] each, the value text or parts with links in it.
function rowsOf(kind, item) {
  if (kind === "targets") {
    const rows = [];
    // A row per part of the editor that has something in it: what its lists hold, or its actions.
    for (const section of SECTIONS) {
      const counts = section.lists.filter((key) => item[key] && item[key].length).map((key) => LISTS[key].count(item[key].length));
      const actions = section.lists.length ? [] : section.actions.filter((e) => item.actions && item.actions[e.key]).map((e) => e.short || e.label);
      const value = [...counts, ...actions];
      if (value.length) rows.push([section.icon, t(`tiles.${section.key}`), value.join(t("common.separator"))]);
    }
    if (!rows.length) rows.push(["target-16", t("tiles.blocks"), t("targets.nothing")]);
    return [...rows, ["locked", t("tiles.usedBy"), usersText(item)]];
  }
  let when;
  if (item.type === "combined") {
    const members = item.members.map((id) => {
      const member = overview.conditions.find((c) => c.id === id);
      return member ? ref(member.name, editHref("conditions", id)) : t("common.gone");
    });
    when = tn(item.mode === "any" ? "conditions.activeAny" : "conditions.activeAll", { names: joined(members) });
  } else if (item.type === "webhook") when = t("conditions.activeWebhook", { url: item.url });
  else if (item.type === "flash_drive") when = t("conditions.activeDrive", { drive: quoted(item.drive) });
  else when = t("conditions.active", { times: weekText(item.days) });
  const icon = { flash_drive: "plug-16", webhook: "webhook", combined: "rule-16" }[item.type] || "calendar-16";
  return [[icon, t("tiles.when"), when], ["locked", t("tiles.usedBy"), usersText(item)]];
}

// A row to read, as a session's rows are laid out (`app.css`, `.tile-rows`).
const infoRow = ([icon, label, value]) =>
  el(
    "cds-contained-list-item",
    {},
    carbonIcon(icon, "icon"),
    el("span", { class: "row" }, el("span", { class: "row-label" }, label), el("span", { class: "row-text" }, el("span", { class: "row-value" }, ...nodesOf(value)))),
  );

// A text with items in it (`ref`), as plain text.
const plain = (text) => el("span", {}, ...nodesOf(text)).textContent;

// What a session's rows say (`fields.js`): the targets' names and the condition's, each a link to
// its editor. The allowance and lock rows are their dropdowns.
function sessionLines(item) {
  const running = runningOf(item.id);
  const names = item.targets.map((id) => (overview.targets.some((l) => l.id === id) ? ref(targetName(id), editHref("targets", id)) : targetName(id)));
  const condition = conditionOf(item);
  const rule = running ? running.condition : condition;
  return {
    block: names.length ? joined(names) : t("sessions.tile.noTargets"),
    when: condition ? ref(condition.name, editHref("conditions", condition.id)) : rule ? plain(ruleName(rule)) : t("sessions.fields.always"),
  };
}

// ---- A page of tiles. Tiles are kept and updated in place, so an open menu or a lock being
// typed survives a refresh.

const tiles = new Map(); // `${kind}:${id}` -> its parts

// Brings the tile of the session `id` into view: where a link to a session goes, as it has no
// page of its own.
function showSession(id) {
  const parts = tiles.get(`sessions:${id}`);
  if (parts) parts.tile.scrollIntoView({ block: "start" });
}

// A tile's ⋮ opens below its button, or above it where there is no room below: Carbon's overflow
// menu does not flip by itself, so its direction (its own property) is set before it opens. Its
// items are Carbon's md size, 2.5rem.
function openTowardsRoom(trigger, body) {
  const rem = parseFloat(getComputedStyle(document.documentElement).fontSize);
  const height = body.querySelectorAll("cds-overflow-menu-item:not([hidden])").length * 2.5 * rem;
  const box = trigger.getBoundingClientRect();
  const up = innerHeight - box.bottom < height && box.top > innerHeight - box.bottom;
  body.setAttribute("direction", up ? "top" : "bottom");
}

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
      // A session is set up on its tile, so it is renamed here instead of edited.
      kind === "sessions"
        ? el("cds-overflow-menu-item", { "data-action": "rename" }, t("sessions.fields.rename"))
        : el("cds-overflow-menu-item", { "data-action": "edit" }, t("common.edit")),
      el("cds-overflow-menu-item", { "data-action": "duplicate" }, t("common.duplicate")),
      el("cds-overflow-menu-item", { "data-action": "delete", danger: true }, t("common.delete")),
    ),
  );
  // In the menu's place while it cannot be changed: a session that is locked.
  const lock = el("span", { class: "lock", role: "img", "aria-label": t("common.locked"), title: t("common.locked") }, carbonIcon("locked"));
  // The title is a link to the editor, as the names inside the tiles' lines are.
  // An edit icon after it shows on hover and focus (`app.css`).
  const name = el("span");
  const editIcon = carbonIcon("edit", "icon");
  editIcon.classList.add("title-edit");
  const title = el("h2", { class: "cds--type-productive-heading-03" }, kind === "sessions" ? name : el("cds-link", { href: editHref(kind, id) }, name, editIcon));
  const tag = el("cds-tag", { size: "md" });
  const actions = el("div", { class: "actions" });
  const columns = "session-tile cds--css-grid-column cds--sm:col-span-4 cds--md:col-span-8 cds--lg:col-span-16";
  // The state tag beside the title, which saves a row.
  const top = el("div", { class: "top" }, el("div", { class: "heading" }, title, tag), menu, lock);
  // Its rows under its head (a session's: `fields.js`), then a session's Start / Stop on the right.
  const rows = el("cds-contained-list", { class: "tile-rows" });
  const tile = el("cds-tile", { class: columns }, top, rows, kind === "sessions" ? actions : null);
  // Below its button, or above it where there is no room.
  const menuBody = menu.querySelector("cds-overflow-menu-body");
  menu.addEventListener("click", () => !menu.open && openTowardsRoom(menu, menuBody), true);
  // On the items themselves: Carbon moves the open menu out of the tile.
  for (const item of menu.querySelectorAll("[data-action]")) {
    item.addEventListener("click", () => {
      if (item.dataset.action === "rename") renameSession(id);
      if (item.dataset.action === "edit") location.hash = KINDS[kind].editPath + encodeURIComponent(id);
      if (item.dataset.action === "duplicate") duplicate(kind, id);
      if (item.dataset.action === "delete") confirmDelete(kind, id);
    });
  }
  const parts = { tile, top, rows, name, tag, actions, menu, lock, editIcon, deleteItem: menu.querySelector('[data-action="delete"]') };
  if (kind === "sessions") {
    addSessionControls(parts, id);
    addRows(parts, id);
  }
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
    // A session keeps its menu even while locked: it can still be renamed and copied.
    const held = kindOf.held(item);
    parts.menu.hidden = !!held;
    parts.lock.hidden = !parts.menu.hidden;
    // Nothing to edit while it is held, so no room kept for the title's edit icon: the tag moves up to the name.
    parts.editIcon.classList.toggle("unavailable", !!held);
    if (kind === "sessions") {
      renderSessionControls(parts, item);
      renderRows(parts, item, sessionLines(item));
    } else {
      // Its rows, only redrawn when they changed (an open link keeps its focus).
      const rows = rowsOf(kind, item);
      const shown = JSON.stringify(rows);
      if (parts.rows.dataset.shown !== shown) {
        parts.rows.dataset.shown = shown;
        parts.rows.replaceChildren(...rows.map(infoRow));
      }
      // Tagged while a locked session uses it (it can then only get stricter); a running session
      // that is not locked leaves it free to change, but not to delete.
      parts.tag.hidden = !kindOf.limited(item);
      parts.tag.setAttribute("type", "blue");
      parts.tag.textContent = t("common.inUse");
    }
    parts.deleteItem.disabled = kindOf.inUse(item);
  });
  $("empty").hidden = shown.length > 0;
}

// ---- A session's tile: its state, its lock (the last of its fields, `fields.js`) and Start / Stop.

function addSessionControls(parts, id) {
  const toggle = el("cds-button", { kind: "primary", size: "md" });
  // A delay lock's wait can be ended while it counts down or waits to be used.
  const abort = el("cds-button", { kind: "tertiary", size: "md", hidden: true }, t("sessions.abortStop"), carbonIcon("locked", "icon"));
  parts.actions.append(abort, toggle);
  // The Lock row's menu chooses the method, or the settings of the one it has (`fields.js`).
  Object.assign(parts, { toggle, abort, setLock: (method) => setLock(id, method) });
  abort.addEventListener("click", () => abortStop(id));
  toggle.addEventListener("click", () => startOrStop(id));
}

// Sets a session's lock to `method`: what the method needs is asked for in the modal, and the
// lock is saved with the session once it is given. Cancel leaves the lock as it was.
async function setLock(id, method) {
  const parts = tiles.get(`sessions:${id}`);
  const session = overview.sessions.find((b) => b.id === id);
  if (!session) return;
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
    if (make) {
      detour.current = {
        back: KINDS.sessions.path,
        crumbs: [{ label: KINDS.sessions.title, href: KINDS.sessions.path }, { label: session.name, href: KINDS.sessions.path }],
        onSaved: async (condition) => {
          const now = overview.sessions.find((b) => b.id === id) || session;
          await invoke("save_session", { session: { ...now, lock: { method: "condition", condition: condition.id } } }).catch((e) =>
            notify("list-notices", "error", t("sessions.lockFailed", { name: quoted(session.name) }), String(e)),
          );
        },
      };
      location.hash = `#/session/${encodeURIComponent(id)}/conditions/new`;
    }
    return;
  }
  const form = LOCKS[method].form && LOCKS[method].form(session);
  let lock = { method };
  if (form) {
    // Kept on the tile while the dialog is open, though the session has the old lock still.
    parts.choosing = true;
    const heading = t("sessions.lockHeading", { method: LOCKS[method].label, name: quoted(session.name) });
    const held = runningOf(id)?.locked;
    lock = await ask({
      heading,
      action: t("common.save"),
      kind: "primary",
      fields: form.fields,
      read: async () => {
        const read = await form.read();
        // A locked session takes only a stricter lock: the service says if it would not, in the dialog.
        if (read && held) await invoke("check_stricter", { check: { what: "lock", id, lock: read } });
        return read;
      },
    });
  }
  // A running session that is not locked takes the lock at once, as if it started with it now
  // (`lock_session`), after a yes for a lock that holds; it is also its lock for the next start.
  const running = runningOf(id);
  if (lock && running) {
    parts.choosing = true;
    const question = await LOCKS[method].confirm({ ...session, lock });
    // A lock that holds is asked first, and so is a held one being made stricter; no lock (nothing to
    // hold) is not.
    const now = running.locked
      ? await confirmStricter(quoted(session.name), `${plain(LOCKS[method].describe(lock))}.`)
      : !question ||
        (await ask({ heading: t("sessions.lockNow.heading", { name: quoted(session.name) }), body: question.body, action: t("sessions.lockNow.action"), kind: "primary" }));
    if (now) {
      try {
        await invoke("lock_session", { id, lock });
        notify("list-notices");
      } catch (e) {
        notify("list-notices", "error", t("sessions.lockFailed", { name: quoted(session.name) }), String(e));
      }
    }
    parts.choosing = false;
    return refresh();
  }
  if (lock) {
    try {
      await invoke("save_session", { session: { ...session, lock } });
      notify("list-notices");
    } catch (e) {
      notify("list-notices", "error", t("sessions.lockFailed", { name: quoted(session.name) }), String(e));
    }
  }
  // Cancelled or refused: the tile shows the lock the session has.
  parts.choosing = false;
  await refresh();
}

function renderSessionControls(parts, session) {
  const running = runningOf(session.id);
  const quiet = running && running.enforcing === false;
  parts.tag.hidden = false;
  parts.tag.setAttribute("type", running ? (quiet ? "cool-gray" : running.locked ? "green" : "blue") : "gray");
  parts.tag.textContent = t(`sessions.state.${running ? (quiet ? "inactive" : running.locked ? "runningLocked" : "running") : "notRunning"}`);
  const busy = parts.toggle.dataset.busy === "1";
  // A session locked with a delay is asked to stop first, then waited out.
  const asking = running && running.stop_delay && running.stop_remaining === null;
  parts.abort.hidden = !(running && running.stop_delay && running.stop_remaining !== null);
  // Waiting: keep it locked; once the wait is over, lock it again.
  const abortLabel = t(running && running.stop_remaining === 0 ? "sessions.relock" : "sessions.abortStop");
  if (parts.abort.textContent.trim() !== abortLabel) parts.abort.replaceChildren(abortLabel, carbonIcon("locked", "icon"));
  const action = running ? (asking ? "ask" : "stop") : "start";
  if (parts.toggle.dataset.action !== action) {
    parts.toggle.dataset.action = action;
    parts.toggle.setAttribute("kind", running ? "secondary" : "primary");
    parts.toggle.replaceChildren(t(asking ? "sessions.requestStop" : running ? "sessions.stop" : "sessions.start"), carbonIcon(running ? "stop" : "play", "icon"));
  }
  // A session locked with a password stops with it, one locked with a text by typing it (asked for on Stop).
  parts.toggle.disabled = busy || (running && running.locked && !asking && !running.password && !running.lock_text) || !reachable;
}

// The dialog of a text lock: the text, and an input to type it into. Pasting and dropping text
// into the input are refused, so the text has to be typed; the dialog only goes on when what is
// typed is exactly the text (the service checks it again). Resolves the typed text, or null.
async function askText(heading, expected, action = t("sessions.stop")) {
  const shown = el("p", { class: "typed-text cds--type-body-01" }, expected);
  const field = el("cds-textarea", { label: t("sessions.stopTextLabel"), rows: 4, autocomplete: "off", spellcheck: "false" });
  const refuse = (event) => event.preventDefault();
  for (const name of ["paste", "drop"]) field.addEventListener(name, refuse, true);
  field.addEventListener("beforeinput", (event) => (/^insertFrom(Paste|Drop|Yank)/.test(event.inputType) ? refuse(event) : null), true);
  field.addEventListener("input", () => invalid(field, null));
  setTimeout(() => field.focus(), 150);
  const same = (a, b) => a.replace(/\r\n/g, "\n") === b.replace(/\r\n/g, "\n");
  const given = await ask({
    heading,
    body: t("sessions.stopTextBody"),
    action,
    kind: "danger",
    fields: [shown, field],
    read: async () => (invalid(field, same(field.value, expected) ? null : t("sessions.stopTextDiffers")) ? null : field.value),
  });
  return given || null;
}

// Ends the wait of a delay lock: locked again.
async function abortStop(id) {
  const session = overview.sessions.find((b) => b.id === id);
  try {
    await invoke("abort_stop", { id });
    notify("list-notices");
  } catch (e) {
    notify("list-notices", "error", t("sessions.stopFailed", { name: quoted(session.name) }), String(e));
  }
  await refresh();
}

async function startOrStop(id) {
  const parts = tiles.get(`sessions:${id}`);
  const running = runningOf(id);
  const session = overview.sessions.find((b) => b.id === id);
  // A session that cannot be stopped is only started after a second, deliberate click.
  const lock = !running && LOCKS[session.lock.method];
  const question = lock && (await lock.confirm(session));
  if (question && !(await ask({ ...question, kind: "primary" }))) return;
  // A session locked with a password stops with it.
  let password = null;
  if (running && running.password) {
    const field = el("cds-password-input", { label: t("sessions.stopPassword"), autocomplete: "off" });
    const given = await ask({
      heading: t("sessions.stopHeading", { name: quoted(session.name) }),
      body: t("sessions.stopBody"),
      action: t("sessions.stop"),
      kind: "danger",
      fields: [field],
      read: async () => (invalid(field, field.value ? null : t("sessions.stopPasswordMissing")) ? null : field.value),
    });
    if (!given) return;
    password = given;
  }
  // A session locked with a text stops by typing it in.
  let text = null;
  if (running && running.lock_text) {
    text = await askText(t("sessions.stopHeading", { name: quoted(session.name) }), running.lock_text);
    if (text === null) return;
  }
  parts.toggle.dataset.busy = "1";
  parts.toggle.disabled = true;
  try {
    if (running && running.stop_delay && running.stop_remaining === null) await invoke("request_stop", { id });
    else if (running) await invoke("stop_session", { id, password, text });
    else await invoke("start_session", { id });
    notify("list-notices");
  } catch (e) {
    notify("list-notices", "error", t(running ? "sessions.stopFailed" : "sessions.startFailed", { name: quoted(session.name) }), String(e));
  }
  parts.toggle.dataset.busy = "";
  await refresh();
}

// What deleting a target or condition does to the sessions that use it, in sentences: the
// service's rule (`deletion_fallout`, `Library::fallout`), worded by strings.js.
async function fallout(id) {
  const effects = await invoke("deletion_fallout", { sessions: overview.sessions, conditions: overview.conditions, id });
  const sentences = { without_it: "losesTarget", always: "becomesAlways", unlocked: "losesLock", shrinks: "shrinksCombination", removed: "removesCombination" };
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
      ...(kind === "sessions" ? (runningOf(id) ? [t("delete.stopsIt")] : []) : await fallout(id)),
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

export { tiles, renderTiles, showSession, confirmDelete, askText };
