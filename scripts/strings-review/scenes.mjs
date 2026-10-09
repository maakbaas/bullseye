// The states the window is put in, in order of preference: a text is pictured in the first scene that
// shows it best (`build.mjs`). Each scene starts the window afresh at `hash`, with the stand-in's
// library (`mock.js`) as `setup` changes it (source run in the page before it starts, with `S` the
// state, `t` and `quoted` as the page has them), then takes its `steps`:
//
//   { click: "selector" }  { hover: "selector" }  { fill: ["selector", "text"] }  { type: [...] }
//   { press: "Key" }  { pick: ["dropdown", "value"] }  { open: "selector" }  { eval: "page code" }
//   { scroll: "selector" }  { wait: ms }
//
// Selectors are Playwright's, which go into Carbon's shadow DOM. `keep: true` leaves the pointer and
// focus where the steps left them (an open menu, a tooltip). An `eval` that imports the page's own
// modules (`/js/core.js`, …) gets the instances the page runs, for the few texts no click reaches.

const tile = (name) => `#tiles cds-tile:has(h2:has-text("${name}"))`;
const row = (name, n) => `${tile(name)} .tile-rows > cds-contained-list-item:nth-of-type(${n})`;
const ROW = { targets: 1, condition: 2, allowance: 3, lock: 4 };
const rowButton = (name, key) => `${row(name, ROW[key])} cds-icon-button`;
const rowDropdown = (name, key) => `${row(name, ROW[key])} cds-dropdown`;
const toggle = (name) => `${tile(name)} .actions cds-button[data-action]`;
const pause = (name) => `${tile(name)} .actions cds-button:not([data-action]):not([hidden]) >> nth=-1`;
const menu = (name) => `${tile(name)} cds-overflow-menu`;
const menuItem = (action) => `cds-overflow-menu-item[data-action="${action}"]:visible`;
const field = (n, inner = "input") => `#confirm-fields > :nth-child(${n}) ${inner}`;
const SAVE = { click: "#confirm-action" };
const editorSave = { click: "#save" };

// Session tiles that each show other rows: what runs (`S.running`) and what its rows say (`S.status`).
const statuses = (spec) => `
  const spec = ${JSON.stringify(spec)};
  S.running = {};
  S.status = {};
  for (const [id, { run = {}, ...rows }] of Object.entries(spec)) {
    S.running[id] = run;
    S.status[id] = rows;
  }
`;

// A stopped session for every way its rows read, by its definition (`mock.js`, `stopped`).
const stoppedSessions = (list) => `
  S.running = {};
  S.status = {};
  S.sessions = ${JSON.stringify(list)}.map(([id, name, condition, allowance, lock], n) => ({
    id, name, targets: [["social"], ["social", "games"], ["news"], ["games"], ["spare"]][n % 5], condition, allowance, lock, kept_locks: [],
  }));
`;

// The question a locked session's lock gets before it is made stricter, for a method the tile never
// offers then (`setLock`): what the page would ask, built by its own code.
const describeLock = (lock) => ({
  eval: `
    const core = await import("/js/core.js");
    const { LOCKS } = await import("/js/locks.js");
    const parts = [].concat(LOCKS[${JSON.stringify(lock.method)}].describe(${JSON.stringify(lock)}));
    const text = parts.map((p) => (typeof p === "string" ? p : p.link)).join("");
    core.confirmStricter(core.quoted("Deep work"), core.t("common.sentence", { text }));
  `,
  after: 700,
});

// A stopped session with this lock, to start: the question before it starts.
const startWith = (lock, extra = "") => ({
  setup: `S.sessions.find((s) => s.id === "weekend").lock = ${JSON.stringify(lock)}; S.sessions.find((s) => s.id === "weekend").targets = ["spare"]; ${extra}`,
  steps: [{ click: toggle("Weekend"), after: 700 }],
});

// Settings with unlocking as `unlock` says.
const settings = (unlock, extra = "") => `S.unlock = { ...S.unlock, ...${JSON.stringify(unlock)} }; ${extra}`;

const IMPORT = (contents, left_out = [], apps_checked = true) =>
  `S.answers.open_settings = { path: "C:\\\\Users\\\\you\\\\bullseye-settings.json", contents: ${JSON.stringify(contents)}, left_out: ${JSON.stringify(left_out)}, apps_checked: ${apps_checked} };`;

export const scenes = [
  // ---- The pages as they are.
  { title: "Sessions", hash: "#/" },
  { title: "Targets", hash: "#/targets" },
  { title: "Conditions", hash: "#/conditions" },
  { title: "Settings", hash: "#/settings" },

  // ---- What a session's rows say, running and stopped.
  {
    title: "Sessions, running in several ways",
    setup: statuses({
      deep: { run: { locked: true, remaining: 4980 }, state: "locked", c: ["activeFor", { change: "4 hours" }], a: ["allowedFor", { change: "12 minutes" }], l: ["timeLeft", { left: "83 minutes" }] },
      reading: { run: { enforcing: false, stop_delay: 900, stop_remaining: 720, allowance: { terms: { type: "breaks" }, open: true } }, state: "inactive", c: ["inactiveFor", { change: "13 hours" }], a: ["allowedIn", { change: "20 minutes" }], l: ["in", { left: "12 minutes" }] },
      evening: { run: { locked: true }, state: "locked", c: ["active"], a: ["blocked"], l: ["lockedFor", { change: "3 hours" }] },
      weekend: { run: { enforcing: false }, state: "inactive", c: ["inactive"], a: ["startsIn", { change: "45 minutes" }], l: ["unlockedFor", { change: "2 hours" }] },
    }),
  },
  {
    title: "Sessions, running in other ways",
    setup: statuses({
      deep: { run: { locked: true }, state: "locked", c: ["activeByDevice"], a: ["started"], l: ["locked"] },
      reading: { run: { stop_delay: 900, stop_remaining: 0 }, state: "running", c: ["inactiveByDevice"], a: ["left", { left: "18 minutes" }], l: ["unlocked"] },
      evening: { run: { locked: true }, state: "locked", c: ["inactiveByWebhook"], a: ["back", { reset: "3 hours" }], l: ["lockedByDevice"] },
      weekend: { run: {}, state: "running", c: ["activeByCombination"], a: ["usedUp"], l: ["unlockedByDevice"] },
    }),
  },
  {
    title: "Sessions, running in yet other ways",
    setup: statuses({
      deep: { run: { locked: true }, state: "locked", c: ["inactiveByCombination"], a: ["noneUsedUp"], l: ["lockedByWebhook"] },
      reading: { run: {}, state: "running", c: ["active"], a: ["itemsUsedUp", { n: 1 }], l: ["unlockedByWebhook"] },
      evening: { run: { locked: true }, state: "locked", c: ["active"], a: ["itemsUsedUp", { n: 3 }], l: ["lockedByCombination"] },
      weekend: { run: {}, state: "running", c: ["active"], a: ["breaksLeft", { n: 1 }], l: ["unlockedByCombination"] },
    }),
  },
  {
    title: "Sessions, running with breaks and locks",
    setup: statuses({
      deep: { run: { locked: true, until_restart: true }, state: "locked", c: ["active"], a: ["noneLeft"], l: ["restart"] },
      reading: { run: { locked: true, lock_text: "I will not scroll tonight" }, state: "locked", c: ["active"], a: ["breaksLeft", { n: 2 }], l: ["text"] },
      evening: { run: { locked: true, password: true }, state: "locked", c: ["active"], a: ["neverPaused"], l: ["password"] },
      weekend: { run: {}, state: "running", c: ["active"], a: ["neverPaused"], l: ["notLocked"] },
    }),
  },
  {
    title: "Sessions, stopped, set up in several ways",
    setup: stoppedSessions([
      ["s1", "Mornings", "hours", { type: "delayed", kind: "after", value: "1h" }, { method: "timed", kind: "for", value: "2h" }],
      ["s2", "Until Friday", "key", { type: "delayed", kind: "from", value: "2026-10-16 18:00" }, { method: "timed", kind: "until", value: "2026-10-16 18:00" }],
      ["s3", "Each site", "home", { type: "budget", allowed: "15m", scope: "each", reset: "never" }, { method: "until_restart" }],
      ["s4", "Small budget", "out", { type: "budget", allowed: "30m", scope: "total", reset: "never" }, { method: "text", text: "I will not" }],
      ["s5", "One break", null, { type: "breaks", count: 1, length: "10m", reset: "never" }, { method: "password" }],
    ]),
  },
  {
    title: "Sessions, stopped, set up in other ways",
    setup: stoppedSessions([
      ["s1", "Breaks", null, { type: "breaks", count: 3, length: "10m", reset: "never" }, { method: "delay", delay: "15m" }],
      ["s2", "During work", null, null, { method: "condition", condition: "hours" }],
      ["s3", "Key", null, null, { method: "condition", condition: "key" }],
      ["s4", "Random", null, null, { method: "random_text", length: 30 }],
      ["s5", "Plain", null, null, { method: "none" }],
    ]),
  },
  {
    title: "Sessions running whose condition was deleted",
    setup: `
      S.conditions = S.conditions.filter((c) => !["hours", "home", "out", "key"].includes(c.id));
      S.running = {
        deep: { locked: true, condition: { type: "schedule", days: window.__week(510, 1050) } },
        reading: { condition: { type: "webhook", url: "https://example.com" } },
        evening: { condition: { type: "flash_drive", serial: "1", drive: "SanDisk Cruzer" } },
        weekend: { condition: { type: "combined", mode: "any", members: [], rules: [] } },
      };
      S.sessions.find((s) => s.id === "evening").condition = "gone";
      S.sessions.find((s) => s.id === "weekend").condition = "gone2";
      S.sessions.find((s) => s.id === "deep").targets.push("deleted-target");
      S.status = {};
    `,
  },
  {
    title: "Conditions, a combination with a deleted condition, schedules not described yet",
    hash: "#/conditions",
    setup: `S.conditions.find((c) => c.id === "both").members = ["hours", "deleted"]; S.pending.push("describe_schedule");`,
  },
  { title: "Conditions, a drive used by a locked session", hash: "#/conditions", setup: `S.running.evening = { locked: true };` },
  { title: "One session shown on its own", hash: "#/edit/evening" },
  { title: "No sessions yet", setup: `S.sessions = []; S.running = {};` },
  { title: "No targets yet", hash: "#/targets", setup: `S.targets = []; S.sessions = []; S.running = {};` },
  { title: "No conditions yet", hash: "#/conditions", setup: `S.conditions = [];` },
  { title: "The service does not answer", setup: `S.fail.status = "The Bullseye service did not answer: the pipe \\\\\\\\.\\\\pipe\\\\bullseye is not there";` },

  // ---- Tooltips and menus.
  { title: "A session's ⋮ menu", steps: [{ click: menu("Evening") }], keep: true },
  { title: "A target's ⋮ menu", hash: "#/targets", steps: [{ click: menu("News") }], keep: true },
  { title: "Tooltip of the ⋮ button", steps: [{ hover: menu("Evening"), after: 900 }], keep: true },
  { title: "Tooltip of the Targets row's button", steps: [{ hover: rowButton("Evening", "targets"), after: 900 }], keep: true },
  { title: "Tooltip of the Condition row's button", steps: [{ hover: rowButton("Evening", "condition"), after: 900 }], keep: true },
  { title: "Tooltip of the Allowance row's button", steps: [{ hover: rowButton("Evening", "allowance"), after: 900 }], keep: true },
  { title: "Tooltip of the Lock row's button", steps: [{ hover: rowButton("Evening", "lock"), after: 900 }], keep: true },
  { title: "A session's allowance dropdown, open", steps: [{ open: rowDropdown("Evening", "allowance") }], keep: true },
  { title: "A session's lock dropdown, open", steps: [{ open: rowDropdown("Evening", "lock") }], keep: true },

  // ---- New, rename, duplicate, delete.
  { title: "New session", steps: [{ click: "#new-item" }] },
  { title: "New session, no name", steps: [{ click: "#new-item" }, { fill: [field(1), ""] }, SAVE] },
  { title: "New session, refused", setup: `S.fail.save_session = "A session cannot be saved while the service is busy";`, steps: [{ click: "#new-item" }, SAVE] },
  { title: "Renaming a session", steps: [{ click: menu("Evening") }, { click: menuItem("rename") }] },
  { title: "Duplicated", steps: [{ click: menu("Evening") }, { click: menuItem("duplicate"), after: 900 }] },
  { title: "Not duplicated", setup: `S.fail.duplicate = "There is no session with this id";`, steps: [{ click: menu("Evening") }, { click: menuItem("duplicate"), after: 900 }] },
  { title: "Deleting a session", steps: [{ click: menu("Evening") }, { click: menuItem("delete"), after: 700 }] },
  { title: "Deleting a running session", steps: [{ click: menu("Reading") }, { click: menuItem("delete"), after: 700 }] },
  {
    title: "Deleting a target that sessions use",
    hash: "#/targets",
    setup: `S.answers.deletion_fallout = { without_it: ["Reading"], always: ["Evening"], unlocked: ["Weekend"], shrinks: ["Out and about"], removed: ["Weekdays with key"] };`,
    steps: [{ click: menu("News") }, { click: menuItem("delete"), after: 700 }],
  },
  { title: "Deleting a condition", hash: "#/conditions", steps: [{ click: menu("Weekdays with key") }, { click: menuItem("delete"), after: 700 }] },
  { title: "A target not deleted", hash: "#/targets", setup: `S.fail.delete_target = "The service could not write its file";`, steps: [{ click: menu("Spare") }, { click: menuItem("delete"), after: 700 }, SAVE, { wait: 600 }] },

  // ---- A session's rows: targets and condition.
  { title: "A session's targets", steps: [{ click: rowButton("Evening", "targets"), after: 700 }] },
  { title: "A session's condition", steps: [{ click: rowButton("Evening", "condition"), after: 700 }] },
  {
    title: "A session's targets, refused in the dialog",
    setup: `S.fail.save_session = "A running session needs something to block";`,
    steps: [{ click: rowButton("Reading", "targets"), after: 700 }, { click: "#confirm-fields cds-contained-list-item:has-text('News')" }, SAVE, { wait: 500 }],
  },
  { title: "A locked session given another target", steps: [{ click: rowButton("Deep work", "targets"), after: 700 }, { click: "#confirm-fields cds-contained-list-item:has-text('News')" }, SAVE, { wait: 900 }] },
  { title: "A locked session made active at all times", steps: [{ click: rowButton("Deep work", "condition"), after: 700 }, { click: "#confirm-fields cds-contained-list-item:has-text('Always')" }, SAVE, { wait: 900 }] },
  { title: "A locked session's allowance taken away", steps: [{ pick: [rowDropdown("Deep work", "allowance"), "none"], after: 900 }] },
  { title: "A new target, made from a session's tile", steps: [{ click: rowButton("Evening", "targets"), after: 700 }, { click: "#confirm-fields cds-button", after: 900 }] },
  { title: "A new condition, made from a session's tile", steps: [{ click: rowButton("Evening", "condition"), after: 700 }, { click: "#confirm-fields cds-button", after: 900 }] },
  { title: "A session's allowance not saved", setup: `S.fail.save_session = "The service could not write its file";`, steps: [{ pick: [rowDropdown("Evening", "allowance"), "none"], after: 900 }] },

  // ---- Allowances.
  {
    title: "A time budget's settings",
    setup: `S.sessions.find((s) => s.id === "evening").allowance = { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" };`,
    steps: [{ click: rowButton("Evening", "allowance"), after: 900 }],
  },
  {
    title: "A time budget's scope, open",
    setup: `S.sessions.find((s) => s.id === "evening").allowance = { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" };`,
    steps: [{ click: rowButton("Evening", "allowance"), after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=0" }],
    keep: true,
  },
  {
    title: "A time budget's reset, open",
    setup: `S.sessions.find((s) => s.id === "evening").allowance = { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" };`,
    steps: [{ click: rowButton("Evening", "allowance"), after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=1" }],
    keep: true,
  },
  {
    title: "A time budget's anchor, open",
    setup: `S.sessions.find((s) => s.id === "evening").allowance = { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" };`,
    steps: [{ click: rowButton("Evening", "allowance"), after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=2" }],
    keep: true,
  },
  { title: "A Pomodoro cycle's settings", steps: [{ click: rowButton("Evening", "allowance"), after: 900 }] },
  { title: "Breaks' settings", steps: [{ click: rowButton("Reading", "allowance"), after: 900 }] },
  { title: "A delayed start's settings", steps: [{ pick: [rowDropdown("Weekend", "allowance"), "delayed"], after: 900 }] },
  { title: "A delayed start's kind, open", steps: [{ pick: [rowDropdown("Weekend", "allowance"), "delayed"], after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=0" }], keep: true },
  {
    title: "A delayed start from a date and time",
    steps: [{ pick: [rowDropdown("Weekend", "allowance"), "delayed"], after: 900 }, { pick: ["#confirm-fields cds-dropdown:visible >> nth=0", "from"] }],
  },
  { title: "An allowance being checked", setup: `S.pending.push("describe_allowance");`, steps: [{ click: rowButton("Evening", "allowance"), after: 900 }] },
  {
    title: "An allowance that cannot be saved",
    setup: `S.answers.describe_allowance = { reject: { why: "A cycle on the clock has to fit a day evenly", field: null } };`,
    steps: [{ click: rowButton("Evening", "allowance"), after: 900 }, SAVE, { wait: 700 }],
  },

  // ---- Locks: their settings.
  { title: "A timed lock's settings", steps: [{ pick: [rowDropdown("Weekend", "lock"), "timed"], after: 900 }] },
  { title: "A timed lock's kind, open", steps: [{ pick: [rowDropdown("Weekend", "lock"), "timed"], after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=0" }], keep: true },
  { title: "A timed lock until a time, no date", steps: [{ pick: [rowDropdown("Weekend", "lock"), "timed"], after: 900 }, { pick: ["#confirm-fields cds-dropdown:visible >> nth=0", "until"] }, SAVE, { wait: 500 }] },
  {
    title: "A timed lock until a time, no time",
    steps: [
      { pick: [rowDropdown("Weekend", "lock"), "timed"], after: 900 },
      { pick: ["#confirm-fields cds-dropdown:visible >> nth=0", "until"] },
      { eval: `document.querySelector("#confirm-fields cds-date-picker").value = "2026-10-16";` },
      SAVE,
      { wait: 500 },
    ],
  },
  { title: "A delay lock's settings, units open", steps: [{ pick: [rowDropdown("Weekend", "lock"), "delay"], after: 900 }, { open: "#confirm-fields cds-dropdown:visible >> nth=0" }], keep: true },
  { title: "A custom text lock's settings, empty", steps: [{ pick: [rowDropdown("Weekend", "lock"), "text"], after: 900 }, SAVE, { wait: 500 }] },
  { title: "A random text lock's settings", steps: [{ pick: [rowDropdown("Weekend", "lock"), "random_text"], after: 900 }] },
  { title: "A password lock's settings, empty", steps: [{ pick: [rowDropdown("Weekend", "lock"), "password"], after: 900 }, SAVE, { wait: 500 }] },
  {
    title: "A password lock's passwords differ",
    steps: [{ pick: [rowDropdown("Weekend", "lock"), "password"], after: 900 }, { fill: [field(1), "secret"] }, { fill: [field(2), "secrets"] }, SAVE, { wait: 500 }],
  },
  { title: "A password lock that has a password", setup: `S.sessions.find((s) => s.id === "weekend").lock = { method: "password" };`, steps: [{ click: rowButton("Weekend", "lock"), after: 900 }] },
  { title: "A condition lock's settings", steps: [{ click: rowButton("Evening", "lock"), after: 900 }] },
  {
    title: "A condition lock with none chosen",
    steps: [{ click: rowButton("Evening", "lock"), after: 900 }, { eval: `document.querySelector("#confirm-fields cds-dropdown").value = "";` }, SAVE, { wait: 500 }],
  },
  {
    title: "A condition lock with no conditions",
    setup: `S.conditions = []; for (const s of S.sessions) { s.condition = null; if (s.lock.method === "condition") s.lock = { method: "none" }; }`,
    steps: [{ pick: [rowDropdown("Weekend", "lock"), "condition"], after: 900 }],
  },
  { title: "A lock not set", setup: `S.fail.save_session = "The service could not write its file";`, steps: [{ pick: [rowDropdown("Weekend", "lock"), "until_restart"], after: 900 }] },

  // ---- Locks: the question before a session starts locked.
  { title: "Starting a session locked for a while", ...startWith({ method: "timed", kind: "for", value: "2h" }) },
  { title: "Starting a session locked until a time", ...startWith({ method: "timed", kind: "until", value: "2026-10-13 18:00" }) },
  { title: "Starting a session locked until restart", ...startWith({ method: "until_restart" }) },
  { title: "Starting a session locked by a drive", ...startWith({ method: "condition", condition: "key" }) },
  { title: "Starting a session locked with a delay", ...startWith({ method: "delay", delay: "15m" }) },
  { title: "Starting a session locked with a text", ...startWith({ method: "text", text: "I will not scroll tonight" }) },
  { title: "Starting a session locked with a random text", ...startWith({ method: "random_text", length: 30 }) },
  { title: "Starting a session locked with one random character", ...startWith({ method: "random_text", length: 1 }) },
  { title: "Starting a session locked with a password", ...startWith({ method: "password" }) },
  { title: "A session not started", setup: `S.fail.start_session = "Nothing to block: add a site or program to its targets";`, steps: [{ click: toggle("Weekend"), after: 900 }] },

  // ---- Locks: a running session.
  {
    title: "Locking a running session now",
    steps: [{ pick: [rowDropdown("Reading", "lock"), "text"], after: 900 }, { fill: [field(1, "textarea"), "I will not scroll tonight"] }, SAVE, { wait: 900 }],
  },
  { title: "A locked session's timed lock made longer", steps: [{ click: rowButton("Deep work", "lock"), after: 900 }, SAVE, { wait: 900 }] },
  {
    title: "A locked session's lock until a time made later",
    setup: `S.sessions.find((s) => s.id === "deep").lock = { method: "timed", kind: "until", value: "2026-10-13 18:00" };`,
    steps: [{ click: rowButton("Deep work", "lock"), after: 900 }, SAVE, { wait: 900 }],
  },
  {
    title: "A locked session's delay made longer",
    setup: `S.running.reading = { locked: true, stop_delay: 900 };`,
    steps: [{ click: rowButton("Reading", "lock"), after: 900 }, SAVE, { wait: 900 }],
  },
  { title: "A locked session's lock, described: none", steps: [describeLock({ method: "none" })] },
  { title: "A locked session's lock, described: until restart", steps: [describeLock({ method: "until_restart" })] },
  { title: "A locked session's lock, described: a schedule", steps: [describeLock({ method: "condition", condition: "hours" })] },
  { title: "A locked session's lock, described: a drive", steps: [describeLock({ method: "condition", condition: "key" })] },
  { title: "A locked session's lock, described: a combination", steps: [describeLock({ method: "condition", condition: "out" })] },
  { title: "A locked session's lock, described: a custom text", steps: [describeLock({ method: "text", text: "x" })] },
  { title: "A locked session's lock, described: a random text", steps: [describeLock({ method: "random_text", length: 30 })] },
  { title: "A locked session's lock, described: one random character", steps: [describeLock({ method: "random_text", length: 1 })] },
  { title: "A locked session's lock, described: a password", steps: [describeLock({ method: "password" })] },

  // ---- Stopping.
  {
    title: "Stopping a session locked with a password",
    setup: `S.running.reading = { locked: true, password: true }; S.sessions.find((s) => s.id === "reading").lock = { method: "password" };`,
    steps: [{ click: toggle("Reading"), after: 700 }, SAVE, { wait: 500 }],
  },
  {
    title: "Stopping a session locked with a text, mistyped",
    setup: `S.running.reading = { locked: true, lock_text: "I will not scroll tonight" }; S.sessions.find((s) => s.id === "reading").lock = { method: "text", text: "I will not scroll tonight" };`,
    steps: [{ click: toggle("Reading"), after: 700 }, { type: [field(2, "textarea"), "I will"] }, SAVE, { wait: 500 }],
  },
  { title: "Stopping not done", setup: `S.fail.request_stop = "The service could not write its file";`, steps: [{ click: toggle("Reading"), after: 900 }] },
  { title: "A break not taken", setup: `S.fail.take_break = "No breaks left until the reset";`, steps: [{ click: pause("Reading"), after: 900 }] },

  // ---- The target editor.
  { title: "Editing a target a locked session uses", hash: "#/targets/edit/social" },
  { title: "Editing a target with programs, apps and titles", hash: "#/targets/edit/games" },
  { title: "Editing a target with programs and apps", hash: "#/targets/edit/news" },
  { title: "A new target", hash: "#/targets/new", steps: [{ eval: `document.querySelectorAll("cds-accordion-item").forEach((item) => (item.open = true));` }] },
  { title: "A new target, its apps loading", hash: "#/targets/new", setup: `S.pending.push("installed_apps");`, steps: [{ eval: `document.querySelectorAll("cds-accordion-item").forEach((item) => (item.open = true));` }, { scroll: ".app-picker" }] },
  { title: "A new target, its apps not listed", hash: "#/targets/new", setup: `S.fail.installed_apps = "PowerShell could not list the installed apps";`, steps: [{ eval: `document.querySelectorAll("cds-accordion-item").forEach((item) => (item.open = true));` }, { scroll: ".app-picker" }] },
  { title: "A target without a name", hash: "#/targets/edit/news", steps: [{ fill: ["#form cds-text-input input", ""] }, editorSave, { wait: 600 }] },
  { title: "A target not saved", hash: "#/targets/edit/news", setup: `S.fail.save_target = "The service could not write its file";`, steps: [editorSave, { wait: 700 }] },
  {
    title: "A target a locked session uses, made stricter",
    hash: "#/targets/edit/social",
    steps: [{ eval: `const area = document.querySelector("#form cds-textarea"); area.value = area.value + "\\ntiktok.com";` }, editorSave, { wait: 900 }],
  },

  // ---- The condition editor.
  { title: "Editing a schedule a locked session uses", hash: "#/conditions/edit/hours" },
  { title: "Editing a drive a locked session uses", hash: "#/conditions/edit/key", setup: `S.running.evening = { locked: true }; S.conditions.find((c) => c.id === "key").serial = "GONE01";` },
  { title: "A drive condition, its drives open", hash: "#/conditions/edit/key", setup: `S.conditions.find((c) => c.id === "key").serial = "GONE01";`, steps: [{ wait: 400 }, { open: "#form cds-dropdown:visible >> nth=1" }], keep: true },
  { title: "Looking for drives", hash: "#/conditions/edit/key", setup: `S.pending.push("connected_drives");` },
  { title: "No drives", hash: "#/conditions/new", setup: `S.answers.connected_drives = [];`, steps: [{ pick: ["#form cds-dropdown >> nth=0", "flash_drive"] }] },
  { title: "No drive chosen", hash: "#/conditions/new", steps: [{ pick: ["#form cds-dropdown >> nth=0", "flash_drive"] }, editorSave, { wait: 600 }] },
  { title: "Editing a webhook", hash: "#/conditions/edit/home" },
  { title: "A webhook's method, open", hash: "#/conditions/edit/home", steps: [{ open: "#form cds-dropdown:visible >> nth=1" }], keep: true },
  { title: "A webhook's default, open", hash: "#/conditions/edit/home", steps: [{ open: "#form cds-dropdown:visible >> nth=3" }], keep: true },
  { title: "A webhook's match, open", hash: "#/conditions/edit/home", steps: [{ open: "#form cds-dropdown:visible >> nth=4" }], keep: true },
  {
    title: "A webhook not active by default, matching a body",
    hash: "#/conditions/edit/home",
    setup: `Object.assign(S.conditions.find((c) => c.id === "home"), { fallback: false, matcher: { on: "body", status: null, body: "" } });`,
    steps: [editorSave, { wait: 600 }],
  },
  { title: "A webhook tested: matches, active", hash: "#/conditions/edit/home", setup: `S.answers.test_webhook = { status: 200, body: "home", outcome: "matches", active: true };`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook tested: matches, not active", hash: "#/conditions/edit/home", setup: `S.answers.test_webhook = { status: 200, body: "away", outcome: "matches", active: false };`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook tested: no match, active", hash: "#/conditions/edit/home", setup: `S.answers.test_webhook = { status: 404, body: "", outcome: "default", active: true };`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook tested: no match, not active", hash: "#/conditions/edit/home", setup: `S.answers.test_webhook = { status: 404, body: "", outcome: "default", active: false };`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook being tested", hash: "#/conditions/edit/home", setup: `S.pending.push("test_webhook");`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook test without an answer", hash: "#/conditions/edit/home", setup: `S.fail.test_webhook = "No answer within 10 seconds";`, steps: [{ click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook with a wrong address, tested", hash: "#/conditions/edit/home", steps: [{ fill: ["#form cds-text-input:visible input >> nth=1", "example"] }, { click: "#form cds-button:has-text('Test')", after: 700 }] },
  { title: "A webhook asked too often", hash: "#/conditions/edit/home", steps: [{ fill: ["#form .duration cds-text-input input", "0"] }, editorSave, { wait: 600 }] },
  { title: "A webhook with a wrong HTTP code", hash: "#/conditions/edit/home", steps: [{ fill: ["#form cds-text-input:visible input >> nth=3", "abc"] }, editorSave, { wait: 600 }] },
  { title: "A new webhook", hash: "#/conditions/new", steps: [{ pick: ["#form cds-dropdown >> nth=0", "webhook"] }] },
  { title: "A new webhook, further down", hash: "#/conditions/new", steps: [{ pick: ["#form cds-dropdown >> nth=0", "webhook"] }, { scroll: "#form .form section:nth-of-type(3)" }] },
  { title: "Editing a combination", hash: "#/conditions/edit/out" },
  { title: "A combination's mode, open", hash: "#/conditions/edit/out", steps: [{ open: "#form cds-dropdown:visible >> nth=1" }], keep: true },
  { title: "A combination with too few conditions", hash: "#/conditions/new", setup: `S.conditions = S.conditions.filter((c) => c.id === "hours");`, steps: [{ pick: ["#form cds-dropdown >> nth=0", "combined"] }] },
  { title: "A combination with none chosen", hash: "#/conditions/new", steps: [{ pick: ["#form cds-dropdown >> nth=0", "combined"] }, editorSave, { wait: 600 }] },
  { title: "A new condition, its type open", hash: "#/conditions/new", steps: [{ open: "#form cds-dropdown >> nth=0" }], keep: true },
  { title: "A new schedule saved with nothing marked", hash: "#/conditions/new", steps: [editorSave, { wait: 600 }] },
  { title: "A condition without a name", hash: "#/conditions/edit/hours", steps: [{ fill: ["#form cds-text-input input", ""] }, editorSave, { wait: 600 }] },

  // ---- Settings.
  { title: "Settings, a delay, nothing locked", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour" }, "S.running = {};") },
  { title: "Settings, a delay, locked", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour" }) },
  { title: "Settings, unlock counting down", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", remaining: 1800, remaining_words: "30 minutes" }) },
  { title: "Settings, unlock ready", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", remaining: 0, remaining_words: "0 seconds" }) },
  { title: "Settings, the other way open", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", remaining: 1800, remaining_words: "30 minutes", way: { method: "condition", condition: "key" }, way_open: true }) },
  { title: "Settings, a text to unlock with", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", way: { method: "text", text: "I give up" } }) },
  { title: "Settings, a password to unlock with", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", way: { method: "password" } }) },
  {
    title: "Settings, unlocking with a text",
    hash: "#/settings",
    setup: settings({ delay: 3600, delay_words: "1 hour", way: { method: "text", text: "I give up" } }),
    steps: [{ click: "#unlock-answer", after: 700 }],
  },
  { title: "Settings, unlocking with a password", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", way: { method: "password" } }), steps: [{ click: "#unlock-answer", after: 700 }] },
  { title: "Settings, the ways to unlock, open", hash: "#/settings", steps: [{ open: ".way-method" }], keep: true },
  { title: "Settings, setting a text to unlock with", hash: "#/settings", steps: [{ pick: [".way-method", "text"], after: 700 }] },
  { title: "Settings, a text to unlock with, saved", hash: "#/settings", steps: [{ pick: [".way-method", "text"], after: 700 }, { fill: [field(1, "textarea"), "I give up"] }, SAVE, { wait: 700 }] },
  { title: "Settings, the other way removed", hash: "#/settings", setup: settings({ way: { method: "text", text: "I give up" } }), steps: [{ pick: [".way-method", "none"], after: 700 }] },
  { title: "Settings, the other way not set", hash: "#/settings", setup: `S.fail.set_unlock_way = "Unlocking has to be ready first";`, steps: [{ pick: [".way-method", "text"], after: 700 }, { fill: [field(1, "textarea"), "I give up"] }, SAVE, { wait: 700 }] },
  { title: "Settings, a condition to unlock with but none made", hash: "#/settings", setup: `S.conditions = [];`, steps: [{ pick: [".way-method", "condition"], after: 700 }] },
  { title: "Settings, unlocking", hash: "#/settings", steps: [{ click: "#unlock", after: 700 }] },
  { title: "Settings, unlocked", hash: "#/settings", steps: [{ click: "#unlock", after: 700 }, SAVE, { wait: 700 }] },
  { title: "Settings, not unlocked", hash: "#/settings", setup: `S.fail.unlock_all = "The service could not end the sessions";`, steps: [{ click: "#unlock", after: 700 }, SAVE, { wait: 700 }] },
  { title: "Settings, unlocking aborted", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour", remaining: 1800, remaining_words: "30 minutes" }), steps: [{ click: "#abort-unlock", after: 700 }] },
  { title: "Settings, a delay set", hash: "#/settings", steps: [{ fill: ["#delay-field cds-text-input input", "90"] }, { click: "#set-delay", after: 700 }] },
  { title: "Settings, no delay set", hash: "#/settings", setup: settings({ delay: 3600, delay_words: "1 hour" }), steps: [{ fill: ["#delay-field cds-text-input input", "0"] }, { click: "#set-delay", after: 700 }] },
  { title: "Settings, the delay not changed", hash: "#/settings", setup: `S.fail.set_unlock_delay = "A shorter delay needs unlocking to be ready";`, steps: [{ click: "#set-delay", after: 700 }] },
  { title: "Settings, exported", hash: "#/settings", steps: [{ click: "#export", after: 700 }] },
  { title: "Settings, not exported", hash: "#/settings", setup: `S.fail.export_settings = "Cannot write C:\\\\Users\\\\you\\\\bullseye-settings.json";`, steps: [{ click: "#export", after: 700 }] },
  {
    title: "Settings, importing",
    hash: "#/settings",
    setup: IMPORT({ targets: 4, conditions: 5, sessions: 3, unlocked: ["Evening"] }, [{ name: "Minecraft", target: "Games" }]),
    steps: [{ click: "#import", after: 700 }],
  },
  {
    title: "Settings, importing one of each",
    hash: "#/settings",
    setup: IMPORT({ targets: 1, conditions: 1, sessions: 1, unlocked: ["Evening", "Deep work"] }, [], false),
    steps: [{ click: "#import", after: 700 }],
  },
  {
    title: "Settings, imported",
    hash: "#/settings",
    setup: IMPORT({ targets: 4, conditions: 5, sessions: 3, unlocked: [] }, [{ name: "Minecraft", target: "Games" }]),
    steps: [{ click: "#import", after: 700 }, SAVE, { wait: 700 }],
  },
  { title: "Settings, not imported", hash: "#/settings", setup: `S.fail.open_settings = "Not a Bullseye settings file";`, steps: [{ click: "#import", after: 700 }] },
];
