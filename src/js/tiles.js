// A page of tiles, one per session, target or condition: kept and updated in place, so an open menu
// survives a refresh. A session's tile also has its lock and Start / Stop. Deleting is asked here.

import { $, allowanceLabel, ask, confirmStricter, el, invalid, invoke, joined, targetName, newId, editHref, nodesOf, notify, overview, quoted, reachable, ref, refresh, runningOf, conditionOf, t, tn, usersOf, weekText, wordingOf } from "./core.js";
import { LOCKS, conditionLockText } from "./locks.js";
import { ACTIONS, LISTS, SECTIONS } from "./forms.js";
import { KINDS } from "./kinds.js";
import { detour } from "./detour.js";
import { addRows, renderRows, renameSession } from "./fields.js";

// A condition's line on a session's tile, with its name when the session's definition still has it.
// "Active during <name>", for a drive "Active while <name> is unplugged", for a combination with a
// drive in it "Active while <name> is true" (`wordingOf`, as a lock's line is worded), the name a
// link to its condition; without it (a running session keeps its rule) the drive's own name or a plain
// word for the kind.
const ruleLine = (rule, condition) => {
  const icon = "rule-16"; // the Conditions tab's, whatever the kind
  const wording = wordingOf(rule);
  if (condition) {
    const name = ref(condition.name, editHref("conditions", condition.id));
    return [icon, tn(`sessions.tile.${{ drive: "activeDriveNamed", true: "activeCombinedNamed", during: "activeNamed" }[wording]}`, { name })];
  }
  if (rule.type === "flash_drive") return [icon, t("sessions.tile.activeDrive", { drive: quoted(rule.drive) })];
  return [icon, t(rule.type === "combined" ? "sessions.tile.activeCombined" : rule.type === "webhook" ? "sessions.tile.activeWebhook" : "sessions.tile.active")];
};

// A running session's condition line: the rule as `ruleLine` words it, and whether it enforces now, as a
// lock's line says its state: "Active during <name> for the next 3 hours" / "Inactive outside <name> …"
// (the time when the condition has a next change), a drive "Active since <name> is unplugged" /
// "Inactive since <name> is plugged in", a combination with a drive "… since <name> is true / false".
// The tag says the same in a word, so there is no line of its own for it.
const runningRuleLine = (running, rule, condition) => {
  const on = running.enforcing;
  const change = running.condition_change_words;
  const name = condition
    ? ref(condition.name, editHref("conditions", condition.id))
    : rule.type === "flash_drive"
      ? quoted(rule.drive)
      : t("sessions.tile.conditionGone");
  const wording = wordingOf(rule);
  if (wording === "drive") return ["rule-16", tn(`sessions.tile.${on ? "activeSinceDrive" : "inactiveSinceDrive"}`, { name })];
  if (wording === "true") return ["rule-16", tn(`sessions.tile.${on ? "activeSinceTrue" : "inactiveSinceFalse"}`, { name })];
  return ["rule-16", tn(`sessions.tile.${on ? "activeDuring" : "inactiveOutside"}${change ? "For" : ""}`, { name, change })];
};

// A session's allowance line, short like its other lines. Not running: the terms (`Allowed 30 minutes a
// day`, from the backend). Running: where it stands now, from the backend's words (`Running::allowance`):
// a cycle is allowed for the next so long or allowed again in so long, a budget has so much left (and
// when it is given back), a budget for each item says what the items that have been used have left.
// Without one, it says so, as a session without a lock does.
const allowanceLine = (running, allowance) => {
  const icon = "pause-16";
  const state = running && running.allowance;
  if (!state) return [icon, allowance ? allowanceLabel(allowance) : t("sessions.tile.noAllowance")];
  if (state.terms.type === "delayed") return [icon, state.open && state.change_words ? t("sessions.tile.delayedWaiting", { change: state.change_words }) : t("sessions.tile.delayedBegun")];
  if (state.terms.type === "pomodoro") {
    const key = state.open ? "allowanceOpen" : "allowanceBlocked";
    return [icon, state.change_words ? t(`sessions.tile.${key}For`, { change: state.change_words }) : t(`sessions.tile.${key}`)];
  }
  if (state.left !== null) {
    const key = `${state.left === 0 ? "allowanceUsedUp" : "allowanceLeft"}${state.reset_words ? "Reset" : ""}`;
    return [icon, t(`sessions.tile.${key}`, { left: state.left_words, reset: state.reset_words })];
  }
  const items = state.items.map((item) => (item.left === 0 ? t("sessions.tile.allowanceItemUsedUp", { item: item.item }) : t("sessions.tile.allowanceItem", { item: item.item, left: item.left_words })));
  if (!items.length) return [icon, allowanceLabel(state.terms)];
  return [icon, t("sessions.tile.allowanceItems", { items: items.join(t("common.separator")) })];
};

// The sessions that use an item, as links.
const usedBy = (users) => ({ text: tn("common.usedBy", { n: users.length, names: joined(users.map((b) => ref(b.name, editHref("sessions", b.id)))) }) });

// What a tile says, per kind: lines of text.
function summaryOf(kind, item) {
  if (kind === "targets") {
    const parts = [];
    for (const [key, list] of Object.entries(LISTS)) if (item[key] && item[key].length) parts.push(list.count(item[key].length));
    const on = ACTIONS.filter((e) => item.actions && item.actions[e.key]);
    for (const e of on.filter((e) => !SECTIONS.find((s) => s.actions.includes(e)).lists.length)) parts.push(e.short || e.label);
    if (!parts.length) parts.push(t("targets.nothing"));
    const users = usersOf(item.id);
    parts.push(users.length ? usedBy(users) : t("targets.notUsed"));
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
    } else if (item.type === "webhook") what = t("conditions.activeWebhook", { url: item.url });
    else what = item.type === "flash_drive" ? t("conditions.activeDrive", { drive: quoted(item.drive) }) : t("conditions.active", { times: weekText(item.days) });
    return [what, users.length ? usedBy(users) : t("conditions.notUsed")];
  }
  return [];
}

// A text with items in it (`ref`), as plain text: a row's text is no place for links, as the row
// opens a menu.
const plain = (text) => el("span", {}, ...nodesOf(typeof text === "object" && !Array.isArray(text) && text.text ? text.text : text)).textContent;

// What a session's rows say (`fields.js`): per row its value, as defined, and for a running session
// how it stands now (the lines its tile used to have, `runningRuleLine`, `allowanceLine`,
// `lockLines`), or nothing.
function sessionLines(item) {
  const running = runningOf(item.id);
  // The targets are links to their editors, as the condition's name is in its line.
  const names = item.targets.map((id) => (overview.targets.some((l) => l.id === id) ? ref(targetName(id), editHref("targets", id)) : targetName(id)));
  const condition = conditionOf(item);
  const rule = running ? running.condition : condition;
  const lock = LOCKS[item.lock && item.lock.method];
  return {
    block: [names.length ? joined(names) : t("sessions.tile.noTargets"), null],
    // Running: whether it enforces now, and until when if the condition can say.
    when: [
      // The condition, a link to its editor, and what it is in words (its tile's line: the times, the drive, the web address).
      // Its name, a link to its editor (the Targets row's names are); under it what it is in words,
      // and while the session runs what is happening instead.
      condition ? ref(condition.name, editHref("conditions", condition.id)) : rule ? plain(ruleLine(rule, null)[1]) : t("sessions.fields.always"),
      running && rule ? t(`sessions.fields.${running.enforcing === false ? "inactive" : "active"}${running.condition_change_words ? "For" : ""}`, { change: running.condition_change_words }) : running ? t("sessions.fields.active") : null,
      condition ? plain(summaryOf("conditions", condition)[0]) : t("sessions.fields.explain_when"),
    ],
    unless: [item.allowance ? allowanceLabel(item.allowance) : t("sessions.fields.noAllowance"), running ? (item.allowance ? plain(allowanceLine(running, item.allowance)[1]) : t("sessions.fields.enforcing")) : null],
    lock: [
      item.lock.method === "none" ? LOCKS.none.label : lock ? plain(lock.describe(item.lock)) : t("sessions.tile.unknownLock"),
      // Running: how its lock stands, unless it has none.
      running && !(item.lock.method === "none" && !running.locked) ? plain(lockLines(item, running)[0][1]) : null,
    ],
  };
}

// A session's lock line: the lock it will start with, or how its running lock stands.
function lockLines(item, running) {
  const parts = [];
  if (!running) {
    const lock = LOCKS[item.lock && item.lock.method];
    parts.push([item.lock && item.lock.method !== "none" ? "locked" : "unlocked", lock ? lock.describe(item.lock) : t("sessions.tile.unknownLock")]);
    return parts;
  }
  const change = running.lock_change_words;
  const lockRule = running.lock_condition;
  if (running.password) parts.push(["locked", t("sessions.tile.lockedPassword")]);
  else if (running.lock_text) parts.push(["locked", t("sessions.tile.lockedText")]);
  else if (running.stop_delay) {
    const { stop_remaining: left, stop_remaining_words: words } = running;
    parts.push(
      left === null
        ? ["locked", t("sessions.tile.lockedDelay", { delay: running.stop_delay_words })]
        : left === 0
          ? ["unlocked", t("sessions.tile.delayReady")]
          : ["locked", t("sessions.tile.delayWait", { left: words })],
    );
  }
  else if (running.until_restart) parts.push(["locked", t("sessions.tile.lockedRestart")]);
  else if (running.remaining_words) parts.push(["locked", t("sessions.tile.lockedFor", { left: running.remaining_words })]);
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
  } else parts.push(["unlocked", t("sessions.tile.notLocked")]);
  return parts;
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
  const summary = el("div", { class: "summary cds--type-body-compact-01" });
  const actions = el("div", { class: "actions" });
  const columns = "session-tile cds--css-grid-column cds--sm:col-span-4 cds--md:col-span-8 cds--lg:col-span-16";
  // The state tag beside the title, which saves a row.
  const top = el("div", { class: "top" }, el("div", { class: "heading" }, title, tag), menu, lock);
  // A session's lines are its rows (`fields.js`), between its head and Start / Stop.
  const body = el("div", { class: "body" }, el("div", { class: "info" }, kind === "sessions" ? null : summary), actions);
  const tile = el("cds-tile", { class: columns }, top, body);
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
  const parts = { tile, top, name, tag, summary, actions, menu, lock, editIcon, deleteItem: menu.querySelector('[data-action="delete"]') };
  if (kind === "sessions") {
    addSessionControls(parts, id);
    addRows(parts, id);
  }
  tiles.set(`${kind}:${id}`, parts);
  return parts;
}

// Tiles go in two columns from Carbon's lg breakpoint on, each tile in the column of its place (the
// first, third … on the left, the second, fourth … on the right), so the columns are staggered: a tile
// that grows moves only the ones under it. Below it they are all in the first column, which is laid out
// as if it were not there (`app.css`).
const wide = matchMedia("(min-width: 66rem)");
let listed = null;
wide.addEventListener("change", () => listed && renderTiles(listed));

function renderTiles(kind) {
  listed = kind;
  const container = $("tiles");
  if (container.children.length !== 2) container.replaceChildren(el("div", { class: "tile-column" }), el("div", { class: "tile-column" }));
  const [left, right] = container.children;
  const last = new Map();
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
    const column = wide.matches && index % 2 ? right : left;
    const before = last.get(column) || null;
    if (parts.tile.parentElement !== column || parts.tile.previousElementSibling !== before) {
      column.insertBefore(parts.tile, before ? before.nextElementSibling : column.firstElementChild);
    }
    last.set(column, parts.tile);
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
    // A session keeps its menu even while locked: it can still be renamed and copied.
    const held = kindOf.held(item);
    parts.menu.hidden = !!held;
    parts.lock.hidden = !parts.menu.hidden;
    // Nothing to edit while it is held, so no room kept for the title's edit icon: the tag moves up to the name.
    parts.editIcon.classList.toggle("unavailable", !!held);
    if (kind === "sessions") {
      renderSessionControls(parts, item);
      renderRows(parts, item, sessionLines(item));
    }
    else {
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
