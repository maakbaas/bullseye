// Every text the window shows, in one place, so the copy can be fine-tuned without touching the
// page. `index.html` loads this file and looks texts up by their path (`t("blocks.title")`).
//
// - `{name}`, `{times}`, … are filled in by the page. Keep each placeholder a text uses (you may
//   move it); the comment above a group says what they stand for when it is not obvious.
// - A text with `one` and `other` is chosen by the number `{n}`.
// - Times in words (`2 hours and 5 minutes`), weekly schedules (`Mon-Fri 08:00-17:00`) and
//   the service's own messages (why something was refused) come from the Rust code, so the window
//   and the CLI say the same; they are not in this file.
// - Edit, then rebuild (or reopen the window in `npm run dev` after touching src-tauri/src/main.rs).
//   A missing text shows as its path, and the console says which.
window.STRINGS = {
  app: {
    name: "Bullseye",
  },

  // The tabs under the header, one per page.
  nav: {
    // What a screen reader calls the tabs.
    label: "Pages",
    blocks: "Blocks",
    lists: "Block lists",
    conditions: "Conditions",
    settings: "Settings",
  },

  common: {
    cancel: "Cancel",
    save: "Save",
    delete: "Delete",
    edit: "Edit",
    duplicate: "Duplicate",
    // {name}: the copy's (the service names it after the original, with a number).
    duplicated: "Made {name}, a copy of {original}",
    duplicateFailed: "Could not duplicate {name}",
    options: "Options",
    locked: "Locked",
    name: "Name",
    // A block list or condition that a block names but that no longer exists.
    gone: "(gone)",
    // A name inside a sentence; {name}.
    quoted: "“{name}”",
    // Between names in a list of them.
    separator: ", ",
    // Before the last of a list: a, b and c.
    and: " and ",
    serviceDown: "The Bullseye service does not answer",
    // A block list's or condition's tile while a running block uses it.
    inUse: "In use by a running block",
    // {names}: the blocks that use it, as links.
    usedBy: "Used by {names}",
    // While a weekly schedule's description is being asked of the backend.
    weeklyStandIn: "a weekly schedule",
  },

  blocks: {
    title: "Blocks",
    noun: "block",
    description: "A block is what you start: one or more block lists, a condition and a lock. Start any of them; they run independently.",
    newLabel: "New block",
    editTitle: "Edit block",
    emptyTitle: "No blocks yet",
    emptyText: "A block combines block lists (what to block) with a condition (when). Make block lists first, then use New block.",
    editorDescription: "A block enforces its block lists, when its condition has it active. Its lock is set on its tile, next to Start, and changed from its menu.",
    held: "This block is running. Its settings can be changed once it ends.",

    // The editor.
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Give the block a name",
    whatTitle: "What",
    whatText: "The block lists it enforces, each by its own enforcers and allowed sites.",
    listsLegend: "Block lists",
    noLists: "No block lists yet: a block needs at least one.",
    // Under the block lists and the condition in the block editor: made there, added to this block.
    newList: "New block list",
    newCondition: "New condition",
    editList: "Edit",
    editCondition: "Edit condition",
    whenTitle: "When",
    whenText: "While its condition is not active a running block blocks nothing, but keeps running.",
    conditionLabel: "Condition",
    conditionHelper: "When it is active once started. Conditions are made on the Conditions page.",
    always: "Always",

    // A block's tile. A weekly schedule is named, never written out (its times are on the
    // Conditions page, and only there); {drive}: a flash drive's name, quoted; {name}: the
    // condition's name; {left} and {change}: times in words.
    tile: {
      noLists: "No block lists yet",
      active: "Active by its schedule",
      activeNamed: "Active during {name}",
      activeDrive: "Active while {drive} is not connected",
      always: "Always active",
      unknownLock: "Unknown lock",
      lockedPassword: "Locked with a password: stop it with its password",
      lockedText: "Locked: type its text in to stop it",
      lockedRestart: "Locked until Windows restarts",
      lockedDrive: "Locked because {drive} is not connected",
      freeDrive: "Unlocked because {drive} is connected",
      lockedTimer: "Locked: {left} left",
      lockedSchedule: "Locked by its lock schedule, for another {left}",
      lockedScheduleOpen: "Locked by its lock schedule",
      freeSchedule: "Not locked now: can be stopped; locked again in {change}",
      freeScheduleOpen: "Not locked now: can be stopped",
      notLocked: "Not locked: can be stopped at any time",
      activeNow: "Active now; inactive in {change}",
      inactiveUntil: "Inactive by its schedule; active again in {change}",
      inactive: "Inactive by its schedule",
      inactiveDrive: "Inactive because {drive} is connected",
    },
    // The tag on a block's tile.
    state: {
      notRunning: "Not running",
      running: "Running",
      runningLocked: "Running · locked",
      inactive: "Inactive by condition",
    },
    start: "Start",
    stop: "Stop",
    lockLabel: "Lock",
    // In a block's ⋮ menu: its lock's settings.
    lockSettings: "Lock settings",
    // The dialog with a lock's settings. {method}: the lock's name below (locks.*.label).
    lockHeading: "{method} lock for {name}",
    lockFailed: "Could not set the lock of {name}",
    startFailed: "Could not start {name}",
    stopFailed: "Could not stop {name}",
    // Stopping a block locked with a password.
    stopHeading: "Stop {name}?",
    stopBody: "It is locked with a password. Enter it to stop the block.",
    stopPassword: "Password",
    // Stopping a block locked with a text: the text is shown above the input.
    stopTextBody: "It is locked until its text is typed in. Type the text above exactly as it is: pasting is not possible.",
    stopTextLabel: "Type the text here",
    stopTextDiffers: "This is not the text above yet",
    stopPasswordMissing: "Type its password",
  },

  // The lock methods, as chosen on a block's tile. `describe` is the tile's line for a block that
  // is not running; `confirm*` is the question before starting a locked block. A confirmAction is
  // the modal's button: keep it to a few words without placeholders, as Carbon's modal buttons
  // have one fixed height and a long label runs out of them.
  locks: {
    none: {
      label: "No lock",
      describe: "Not locked: can be stopped at any time",
    },
    duration: {
      label: "Duration",
      field: "Locked for",
      placeholder: "e.g. 2h 30m",
      helper: "Combine as you like: 30m, 2h 30m, 1d",
      // {duration}: the length in words.
      describe: "Locked for {duration} once started",
      confirmHeading: "Start {name} for {duration}?",
      confirmBody: "It cannot be stopped, changed or deleted for {duration}. Only unlocking in Settings ends it early.",
      confirmAction: "Start locked",
    },
    // {when}: the end written out (Tuesday 6 October 2026, 18:00); {left}: how long from now.
    until: {
      label: "Until",
      date: "Date",
      datePlaceholder: "yyyy-mm-dd",
      dateMissing: "Choose a date",
      time: "Time",
      timePlaceholder: "hh:mm",
      timeMissing: "Type a time, e.g. 18:00",
      describe: "Locked until {when} once started",
      confirmHeading: "Start {name} until {when}?",
      confirmBody: "It cannot be stopped, changed or deleted until {when}, {left} from now. Only unlocking in Settings ends it early.",
      confirmAction: "Start locked",
    },
    until_restart: {
      label: "Until restart",
      describe: "Locked until Windows restarts once started",
      confirmHeading: "Start {name} until Windows restarts?",
      confirmBody: "It cannot be stopped, changed or deleted until Windows restarts, and it ends then. Shutting down with Fast Startup on is not a restart. Only unlocking in Settings ends it early.",
      confirmAction: "Start locked",
    },
    condition: {
      label: "Condition",
      field: "Locked during",
      helper: "It cannot be stopped while this condition is active",
      missing: "Choose a condition",
      // Chosen when there is no condition yet: offers to make one, which the lock is then set to.
      noneHeading: "No conditions yet",
      noneBody: "A condition lock needs a condition to lock by. Make one now? Once it is saved, the lock is set to it.",
      noneAction: "New condition",
      // {condition}: its name, quoted.
      describe: "Locked while {condition} is active",
      // {drive}: a flash drive's name, quoted: said without the condition's name, which is long enough.
      describeDrive: "Locked while {drive} is not connected",
      describeGone: "Locked by a condition that is gone",
      confirmHeading: "Start {name} locked during {condition}?",
      // {detail}: nothing for a schedule, the drive for a flash drive.
      confirmBody: "It cannot be stopped while {condition} is active{detail}. Otherwise you can stop it. Only unlocking in Settings ends it early.",
      driveDetail: " (the flash drive {drive} is not connected)",
      confirmAction: "Start locked",
    },
    text: {
      label: "Custom text",
      field: "Text to type",
      placeholder: "e.g. I will not scroll tonight",
      helper: "To stop the block, this text has to be typed in exactly. It cannot be pasted. Anyone who can see the block sees the text.",
      missing: "Give the text to type",
      describe: "Locked until its text is typed in",
      confirmHeading: "Start {name} locked by a custom text?",
      confirmBody: "It can only be stopped by typing its text in exactly, in this window, or by unlocking in Settings.",
      confirmAction: "Start locked",
    },
    password: {
      label: "Password",
      field: "Password",
      helper: "Best chosen by someone else. Only a hash of it is kept.",
      // When the block already has a password lock: the old password is never shown.
      helperSet: "A password is set. Type a new one to replace it.",
      repeat: "Repeat password",
      missing: "Type a password",
      differ: "The passwords differ",
      describe: "Locked with a password once started",
      confirmHeading: "Start {name} locked with a password?",
      confirmBody: "It can only be stopped with its password, or by unlocking in Settings.",
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
      helper: "One per line: a site with its subdomains and everything under it. * matches any part; a lone * blocks every site.",
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
    store_apps: {
      label: "Microsoft Store apps to close",
      placeholder: "",
      helper: "Apps installed from the Microsoft Store: tick the ones to close. Windows' own apps are not listed.",
      loading: "Looking up the installed apps...",
      failed: "The installed apps could not be listed",
      count: { one: "{n} Microsoft Store app closed", other: "{n} Microsoft Store apps closed" },
    },
    titles: {
      label: "Window titles to close",
      placeholder: "*YouTube*",
      helper: "One per line: the whole title, where * matches any part and letter case does not matter. The whole window closes, a browser's with all its tabs.",
      count: { one: "{n} window title closed", other: "{n} window titles closed" },
    },

    // The editor's sections, with the enforcers in each. `short` is the tile's line for an
    // enforcer that needs no list.
    sections: {
      websites: {
        title: "Websites",
        description: "Closes pages on the list in Chrome, Edge, Brave and Firefox.",
      },
      programs: {
        title: "Programs",
        description: "Asks the programs on the list to close, and ends them after 3 seconds.",
      },
      store: {
        title: "Microsoft Store apps",
        description: "Asks the apps ticked to close, and ends them after 3 seconds.",
      },
      titles: {
        title: "Windows by title",
        description: "Closes a window when its title matches one on the list.",
      },
      other: {
        title: "Other",
      },
    },
    enforcers: {
      chromium_tabs: { label: "Close blocked tabs in Chromium" },
      chromium_split_view: { label: "Close blocked pages in Chromium split view" },
      chromium_app_windows: { label: "Close blocked app windows in Chromium" },
      firefox_tabs: { label: "Close blocked tabs in Firefox" },
      firefox_split_view: { label: "Close blocked pages in Firefox split view" },
      exe_names: { label: "Close these programs" },
      store_apps: { label: "Close these Microsoft Store apps" },
      window_titles: { label: "Close these windows" },
      obsidian_web_viewer: { label: "Close Obsidian when web viewer is active", short: "Closes Obsidian's web viewer" },
    },
  },

  conditions: {
    title: "Conditions",
    noun: "condition",
    description: "When a block is active: at the times of a weekly schedule, or while a flash drive is not connected. A block without one is always active. While its condition is not active a running block blocks nothing.",
    newLabel: "New condition",
    editTitle: "Edit condition",
    emptyTitle: "No conditions yet",
    emptyText: "Blocks without a condition are always active. Use New condition to make one, e.g. work hours.",
    editorDescription: "Kept by the Bullseye service. A running block keeps the condition it was started with.",
    held: "{names} uses it and is running. It can be changed once that has ended.",
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Give the condition a name",
    typeLabel: "Type",
    typeSchedule: "Weekly schedule",
    typeDrive: "Flash drive",
    // {times}: the schedule in words; {drive}: the drive's name, quoted.
    active: "Active {times}",
    activeDrive: "Active while {drive} is not connected",
    notUsed: "Not used by a block yet",
    // Above the week grid.
    nothingMarked: "Nothing marked yet: mark when blocks are active.",
    gridHelp: "Drag over the week to mark when blocks are active. Start on a marked time to clear it instead.",
    // The week grid's column heads, Monday first.
    days: { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" },
    // A flash drive condition: the drive is the key. The connected drives are looked up when the
    // editor opens and on "Scan again".
    drive: {
      field: "Flash drive",
      helper: "Active while this drive is not connected: plug it in and blocks that use it are inactive (and a lock by it opens), take it out and they are active again. Drives that report no serial number are not listed.",
      choose: "Choose a flash drive",
      scanning: "Looking for connected drives...",
      none: "No flash drive found: connect it and scan again",
      // {name}: the drive's name and serial number, for the one the condition was set to that is not connected now.
      away: "{name}, not connected",
      scan: "Scan again",
      missing: "Choose a flash drive",
    },
  },

  // Deleting a block, block list or condition. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?",
    body: "The {noun} is removed. This cannot be undone.",
    // What it does to the blocks that use a block list or condition; {names}: those blocks, quoted.
    deletesToo: "{names} is deleted too, as it has no other block list.",
    keepsOthers: "{names} keeps its other block lists.",
    becomesAlways: "{names} is always active from then on.",
    losesLock: "{names} has no lock from then on.",
    failed: "Could not delete the {noun}",
  },

  editor: {
    saveFailed: "Could not save the {noun}",
    lockedTitle: "Locked while running",
  },

  settings: {
    title: "Settings",
    unlockTitle: "Unlock",
    unlockText: "Ends every running block, locked ones included. Bullseye stays open, and your blocks, block lists and conditions stay as they are. This is the only way to end a locked block early. Ask for it, wait for the countdown, then unlock; or abort, and everything stays locked.",
    // No delay set: the state of a clean install.
    unlockFree: "Ends every running block, locked ones included. Bullseye stays open, and your blocks, block lists and conditions stay as they are. Unlocking is free right now: set an unlock delay below to make it a countdown.",
    // {delay}: how long it takes; {left}: the time left, in words.
    unlockAsk: "Unlocking takes {delay}. Ask for it to start the countdown.",
    unlockWait: "Unlocking is ready in {left}. Abort to stay locked.",
    unlockReady: "Unlocking is ready. Unlock now, or abort to stay locked.",
    unlockNothing: "Nothing is locked right now, so there is nothing to unlock.",
    unlock: "Unlock",
    request: "Request unlock",
    abort: "Abort",
    aborted: "Aborted: everything stays locked",
    confirmHeading: "End every running block?",
    confirmBody: "Locked blocks end too. Your blocks, block lists and conditions stay as they are.",
    failed: "Not unlocked",
    unlocked: "Unlocked: every block has ended",
    delayTitle: "Unlock delay",
    delayText: "How long unlocking takes after you ask for it, from nothing to 24 hours: enough to sleep on it, and to abort with a fresh mind. A longer delay can be set at any time; a shorter one only once unlocking is ready, or it would be a way around the wait.",
    delayLabel: "Delay (up to 24 hours)",
    delayPlaceholder: "e.g. 2h 30m, or 0",
    setDelay: "Set delay",
    // {delay}: the new delay in words.
    delaySet: "Unlocking now takes {delay}",
    delayFree: "Unlocking is free",
    delayFailed: "The delay was not changed",
    transferTitle: "Move to another computer",
    transferText: "Export saves every block list, condition and block to a file. Import, on the other computer, replaces all of them there with what is in the file. Password locks are not in the file: a block with one comes without a lock.",
    export: "Export",
    import: "Import",
    // {file}: the file's name.
    exported: "Exported to {file}",
    exportFailed: "Not exported",
    importHeading: "Replace everything with this file?",
    // {lists}, {conditions}, {blocks}: the counts below, as in the file.
    importBody: "Its {lists}, {conditions} and {blocks} replace the block lists, conditions and blocks you have now. This cannot be undone.",
    importLists: { one: "{n} block list", other: "{n} block lists" },
    importConditions: { one: "{n} condition", other: "{n} conditions" },
    importBlocks: { one: "{n} block", other: "{n} blocks" },
    // {names}: the blocks, quoted; {n}: how many.
    importUnlocked: {
      one: "{names} had a password lock, which is not in the file: it comes without a lock.",
      other: "{names} had a password lock, which is not in the file: they come without a lock.",
    },
    importAction: "Import",
    imported: "Imported: everything is as in the file",
    importFailed: "Not imported",
  },
};
