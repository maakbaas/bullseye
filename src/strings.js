// Every text the window shows, in one place, so the copy can be fine-tuned without touching the
// page. `index.html` loads this file and looks texts up by their path (`t("blocks.title")`).
//
// - `{name}`, `{times}`, … are filled in by the page. Keep each placeholder a text uses (you may
//   move it); the comment above a group says what they stand for when it is not obvious.
// - A text with `one` and `other` is chosen by the number `{n}`.
// - Times in words (`2 hours 5 minutes 0 seconds`), weekly schedules (`Mon-Fri 08:00-17:00`) and
//   the service's own messages (why something was refused) come from the Rust code, so the window
//   and the CLI say the same; they are not in this file.
// - Edit, then rebuild (or reopen the window in `npm run dev` after touching src-tauri/src/main.rs).
//   A missing text shows as its path, and the console says which.
window.STRINGS = {
  app: {
    name: "Bullseye",
  },

  // The header's icons, one per page.
  nav: {
    blocks: "Blocks",
    lists: "Block lists",
    schedules: "Schedules",
    settings: "Settings",
  },

  common: {
    cancel: "Cancel",
    save: "Save",
    delete: "Delete",
    edit: "Edit",
    options: "Options",
    locked: "Locked",
    name: "Name",
    // A block list or schedule that a block names but that no longer exists.
    gone: "(gone)",
    // A name inside a sentence; {name}.
    quoted: "“{name}”",
    // Between names in a list of them.
    separator: ", ",
    serviceDown: "The Bullseye service does not answer",
    // A block list's or schedule's tile while a running block uses it.
    inUse: "In use by a running block",
    // {names}: the blocks that use it, quoted.
    usedBy: "Used by {names}",
    // While a schedule's description is being asked of the backend.
    weeklyStandIn: "a weekly schedule",
  },

  blocks: {
    title: "Blocks",
    noun: "block",
    description: "A block is what you start: one or more block lists, a schedule and a lock. Start any of them; they run independently.",
    newLabel: "New block",
    editTitle: "Edit block",
    emptyTitle: "No blocks yet",
    emptyText: "A block combines block lists (what to block) with a schedule (when). Make block lists first, then use New block.",
    editorDescription: "A block enforces its block lists, when its schedule has it active. Its lock is set on its tile, next to Start, and changed from its menu.",
    held: "This block is running. Its settings can be changed once it ends.",

    // The editor.
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Give the block a name",
    whatTitle: "What",
    whatText: "The block lists it enforces, each by its own enforcers and allowed sites.",
    listsLegend: "Block lists",
    noLists: "No block lists yet: a block needs at least one.",
    newList: "New block list",
    whenTitle: "When",
    whenText: "Outside its schedule a running block blocks nothing, but keeps running.",
    scheduleLabel: "Schedule",
    scheduleHelper: "When it is active once started. Schedules are made on the Schedules page.",
    always: "Always",

    // A block's tile. {times}: a weekly schedule in words; {schedule}: its name; {left} and
    // {change}: times in words.
    tile: {
      noLists: "No block lists yet",
      active: "Active {times}",
      activeNamed: "Active {times} ({schedule})",
      always: "Always active",
      unknownLock: "Unknown lock",
      lockedPassword: "Locked with a password: stop it with its password",
      lockedTimer: "Locked: {left} left",
      lockedSchedule: "Locked by its lock schedule, for another {left}",
      lockedScheduleOpen: "Locked by its lock schedule",
      freeSchedule: "Not locked now: can be stopped; locked again in {change}",
      freeScheduleOpen: "Not locked now: can be stopped",
      notLocked: "Not locked: can be stopped at any time",
      activeNow: "Active now; inactive in {change}",
      inactiveUntil: "Inactive by its schedule; active again in {change}",
      inactive: "Inactive by its schedule",
    },
    // The tag on a block's tile.
    state: {
      notRunning: "Not running",
      running: "Running",
      runningLocked: "Running · locked",
      inactive: "Inactive by schedule",
    },
    start: "Start",
    stop: "Stop",
    lockLabel: "Lock",
    // In a block's ⋮ menu: its lock's settings.
    changeLock: "Change lock…",
    // The dialog with a lock's settings. {method}: the lock's name below (locks.*.label).
    lockHeading: "{method} lock for {name}",
    lockFailed: "Could not set the lock of {name}",
    startFailed: "Could not start {name}",
    stopFailed: "Could not stop {name}",
    // Stopping a block locked with a password.
    stopHeading: "Stop {name}?",
    stopBody: "It is locked with a password. Enter it to stop the block.",
    stopPassword: "Password",
    stopPasswordMissing: "Type its password",
  },

  // The lock methods, as chosen on a block's tile. `describe` is the tile's line for a block that
  // is not running; `confirm*` is the question before starting a locked block.
  locks: {
    none: {
      label: "No lock",
      describe: "Not locked: can be stopped at any time",
    },
    timer: {
      label: "Timer",
      field: "Locked for",
      placeholder: "e.g. 2h 30m",
      helper: "Combine as you like: 30m, 2h 30m, 1d",
      // {duration}: the length in words.
      describe: "Locked for {duration} once started",
      confirmHeading: "Start {name} for {duration}?",
      confirmBody: "It cannot be stopped, changed or deleted for {duration}. Only the unlock password in Settings ends it early.",
      confirmAction: "Start for {duration}",
    },
    scheduled: {
      label: "Schedule",
      field: "Locked during",
      helper: "It cannot be stopped while this schedule is active",
      missing: "Choose a schedule",
      // {schedule}: the lock schedule's name; {times}: its times in words.
      describe: "Locked while {schedule} is active ({times})",
      describeGone: "Locked by a schedule that is gone",
      confirmHeading: "Start {name} locked during {schedule}?",
      confirmBody: "It cannot be stopped while {schedule} is active ({times}). Outside those times you can stop it. Only the unlock password in Settings ends it early.",
      confirmAction: "Start locked",
    },
    password: {
      label: "Password",
      field: "Password",
      helper: "Best chosen by someone else. Only a hash of it is kept.",
      // When the block already has a password lock: the old password is never shown.
      helperSet: "A password is set and is not shown. Type a new one to replace it.",
      repeat: "Repeat password",
      missing: "Type a password",
      differ: "The passwords differ",
      describe: "Locked with a password once started",
      confirmHeading: "Start {name} locked with a password?",
      confirmBody: "It can only be stopped with its password, or with the unlock password in Settings.",
      confirmAction: "Start locked",
    },
  },

  lists: {
    title: "Block lists",
    noun: "block list",
    description: "What to block: sites, programs and apps, and how to close them. Blocks use them; one list can be in several blocks.",
    newLabel: "New block list",
    editTitle: "Edit block list",
    emptyTitle: "No block lists yet",
    emptyText: "A block list holds sites and programs to close, and the ways to close them. Use New block list to make one.",
    editorDescription: "Kept by the Bullseye service. A running block keeps the lists it was started with.",
    // {names}: the running blocks that use it, quoted.
    held: "{names} uses it and is running. It can be changed once that has ended.",
    namePlaceholder: "e.g. Social media",
    nameMissing: "Give the block list a name",
    nothing: "Nothing to block yet",
    notUsed: "Not in a block yet",
    // Above an editor section's checkboxes: with lists, and without.
    howLegend: "How",
    enforcersLegend: "Enforcers",

    // The lists of a block list. `count` is the tile's line.
    filters: {
      label: "Sites to block",
      placeholder: "youtube.com",
      helper: "One per line: a site and everything under it. * matches any part; a lone * blocks every site.",
      count: { one: "{n} site blocked", other: "{n} sites blocked" },
    },
    allowed: {
      label: "Always allowed",
      placeholder: "google.com",
      helper: "Exceptions to this list's sites. These always win.",
      count: { one: "{n} always allowed", other: "{n} always allowed" },
    },
    programs: {
      label: "Programs to close",
      placeholder: "steam.exe",
      helper: "One program per line, e.g. steam.exe; * matches any part. Windows' own programs cannot be listed.",
      count: { one: "{n} program closed", other: "{n} programs closed" },
    },

    // The editor's sections, with the enforcers in each. `short` is the tile's line for an
    // enforcer that needs no list.
    sections: {
      websites: {
        title: "Websites",
        description: "Closes pages on the list in Chrome, Edge and Brave.",
      },
      programs: {
        title: "Programs",
        description: "Asks the programs on the list to close, and ends them after 3 seconds.",
      },
      apps: {
        title: "Apps",
        description: "Need no list.",
      },
    },
    enforcers: {
      chromium_tabs: { label: "Close blocked tabs" },
      chromium_split_view: { label: "Close blocked pages in split view" },
      exe_names: { label: "Close these programs" },
      obsidian_web_viewer: { label: "Close Obsidian when it shows web pages", short: "Closes Obsidian's web viewer" },
    },
  },

  schedules: {
    title: "Schedules",
    noun: "schedule",
    description: "When a block is active, as a weekly schedule. A block without one is always active. Outside its schedule a running block blocks nothing.",
    newLabel: "New schedule",
    editTitle: "Edit schedule",
    emptyTitle: "No schedules yet",
    emptyText: "Blocks without a schedule are always active. Use New schedule to make one, e.g. work hours.",
    editorDescription: "Kept by the Bullseye service. A running block keeps the schedule it was started with.",
    held: "{names} uses it and is running. It can be changed once that has ended.",
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Give the schedule a name",
    // {times}: the schedule in words.
    active: "Active {times}",
    notUsed: "Not used by a block yet",
    timesLabel: "Times",
    timesPlaceholder: "e.g. Mon-Fri 08:00-17:00",
    timesHelper: "Days and times, e.g. Mon-Fri 08:00-17:00; Sat 10:00-12:00. Or drag over the week below to mark times, or to clear them by starting on a marked one.",
    timesMissing: "Type the days and times; a block that is always active needs no schedule",
    // The week grid's column heads, Monday first.
    days: { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" },
  },

  // Deleting a block, block list or schedule. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?",
    body: "The {noun} is removed. This cannot be undone.",
    failed: "Could not delete the {noun}",
  },

  editor: {
    saveFailed: "Could not save the {noun}",
    lockedTitle: "Locked while running",
  },

  settings: {
    title: "Settings",
    unlockTitle: "Unlock",
    unlockText: "Ends every running block, locked ones included, and quits Bullseye. The service stays installed and starts the app again at your next sign-in. This is the only way to end a locked block early.",
    passwordLabel: "Unlock password",
    unlock: "Unlock",
    failed: "Not unlocked",
  },
};
