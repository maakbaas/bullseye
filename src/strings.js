// Every text the window shows, in one place, so the copy can be fine-tuned without touching the
// page. `index.html` loads this file and looks texts up by their path (`t("blocks.title")`).
//
// - `{name}`, `{times}`, … are filled in by the page. Keep each placeholder a text uses (you may
//   move it); the comment above a group says what they stand for when it is not obvious.
// - A text with `one` and `other` is chosen by the number `{n}`.
// - Times (`2 hours and 5 minutes`), weekly schedules (`Mon-Fri 08:00-17:00`) and the service's
//   own messages (why something was refused) come from the Rust code, so the window and the CLI
//   say the same; they are not in this file.
// - Style: GUI labels, short but complete, with little punctuation. A time counting down reads
//   "for the next {time}"; a state is one word or two ("Locked", "No active lock").
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
    gone: "(deleted)",
    // A name inside a sentence; {name}.
    quoted: "“{name}”",
    // Between names in a list of them.
    separator: ", ",
    // Before the last of a list: a, b and c.
    and: " and ",
    serviceDown: "Bullseye service not responding",
    // A block list's or condition's tile while a locked block uses it.
    inUse: "Used by a locked block",
    // {names}: the blocks that use it, as links.
    usedBy: "Used by {names}",
    // While a weekly schedule's description is being asked of the backend.
    weeklyStandIn: "a weekly schedule",
  },

  blocks: {
    title: "Blocks",
    noun: "block",
    description: "Block lists under a condition and a lock. Blocks start and run independently",
    newLabel: "New block",
    editTitle: "Edit block",
    emptyTitle: "No blocks yet",
    emptyText: "Make a block list first, then a block that uses it",
    editorDescription: "Enforces its block lists while its condition is active. The lock is set on the tile",
    held: "Locked, so it cannot be changed until it can be stopped",

    // The editor.
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Name the block",
    whatTitle: "What",
    whatText: "Block lists to enforce, each with its own enforcers and allowed sites",
    listsLegend: "Block lists",
    noLists: "A block needs at least one block list",
    // Under the block lists and the condition in the block editor: made there, added to this block.
    newList: "New block list",
    newCondition: "New condition",
    editList: "Edit",
    editCondition: "Edit condition",
    whenTitle: "When",
    whenText: "A block enforces nothing while its condition is inactive",
    conditionLabel: "Condition",
    conditionHelper: "When the block is active once started",
    always: "Always",

    // A block's tile. A weekly schedule is named, never written out (its times are on the
    // Conditions page, and only there); {drive}: a flash drive's name, quoted; {name}: the
    // condition's name; {left} and {change}: times in words.
    tile: {
      noLists: "No block lists",
      active: "Active by schedule",
      activeCombined: "Active by combination",
      activeNamed: "Active during {name}",
      activeDrive: "Active while {drive} is unplugged",
      always: "Always active",
      unknownLock: "Unknown lock",
      lockedPassword: "Locked with a password",
      lockedText: "Locked until its text is typed",
      lockedRestart: "Locked until Windows restarts",
      lockedDrive: "Locked until {drive} is plugged in",
      freeDrive: "Unlocked while {drive} is plugged in",
      lockedFor: "Locked for the next {left}",
      locked: "Locked",
      unlockedFor: "Unlocked for the next {change}",
      unlocked: "Unlocked",
      notLocked: "No active lock",
      // Whether it enforces now.
      on: "Active",
      onFor: "Active for the next {change}",
      off: "Inactive",
      offFor: "Inactive for the next {change}",
      offDrive: "Inactive while {drive} is plugged in",
    },
    // The tag on a block's tile.
    state: {
      notRunning: "Not running",
      running: "Running",
      runningLocked: "Running · locked",
      inactive: "Inactive",
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
    stopBody: "Enter the password to stop this block",
    stopPassword: "Password",
    // Stopping a block locked with a text: the text is shown above the input.
    stopTextBody: "Type the text above exactly. Pasting is blocked",
    stopTextLabel: "Type the text here",
    stopTextDiffers: "Text does not match yet",
    stopPasswordMissing: "Enter the password",
  },

  // The lock methods, as chosen on a block's tile. `describe` is the tile's line for a block that
  // is not running; `confirm*` is the question before starting a locked block. A confirmAction is
  // the modal's button: keep it to a few words without placeholders, as Carbon's modal buttons
  // have one fixed height and a long label runs out of them.
  locks: {
    none: {
      label: "No lock",
      describe: "No active lock",
    },
    duration: {
      label: "Duration",
      field: "Locked for",
      placeholder: "e.g. 2h 30m",
      helper: "e.g. 30m, 2h 30m or 1d",
      // {duration}: the length in words.
      describe: "Locks for {duration}",
      confirmHeading: "Start {name} for {duration}?",
      confirmBody: "Locked for {duration}. Only unlocking in Settings ends it early",
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
      timeMissing: "Enter a time, e.g. 18:00",
      describe: "Locks until {when}",
      confirmHeading: "Start {name} until {when}?",
      confirmBody: "Locked until {when}, {left} from now. Only unlocking in Settings ends it early",
      confirmAction: "Start locked",
    },
    until_restart: {
      label: "Until restart",
      describe: "Locks until Windows restarts",
      confirmHeading: "Start {name} until Windows restarts?",
      confirmBody: "Locked until Windows restarts. Shutting down with Fast Startup is not a restart. Only unlocking in Settings ends it early",
      confirmAction: "Start locked",
    },
    condition: {
      label: "Condition",
      field: "Locked during",
      helper: "Locked while this condition is active",
      missing: "Choose a condition",
      // Chosen when there is no condition yet: offers to make one, which the lock is then set to.
      noneHeading: "No conditions yet",
      noneBody: "A condition lock needs a condition. Make one now and use it for this lock?",
      noneAction: "New condition",
      // {condition}: its name, quoted.
      describe: "Locks while {condition} is active",
      // {drive}: a flash drive's name, quoted: said without the condition's name, which is long enough.
      describeDrive: "Locks while {drive} is unplugged",
      describeGone: "Condition deleted",
      confirmHeading: "Start {name} locked during {condition}?",
      // {detail}: nothing for a schedule, the drive for a flash drive.
      confirmBody: "Locked while {condition} is active{detail}. Otherwise you can stop it. Only unlocking in Settings ends it early",
      driveDetail: " (while {drive} is unplugged)",
      confirmAction: "Start locked",
    },
    text: {
      label: "Custom text",
      field: "Text to type",
      placeholder: "e.g. I will not scroll tonight",
      helper: "Stopping needs this text typed exactly, no pasting. Anyone who sees the block sees the text",
      missing: "Enter the text",
      describe: "Locks until its text is typed",
      confirmHeading: "Start {name} locked by a custom text?",
      confirmBody: "Stopped only by typing its text here, or by unlocking in Settings",
      confirmAction: "Start locked",
    },
    password: {
      label: "Password",
      field: "Password",
      helper: "Best set by someone else. Only a hash is kept",
      // When the block already has a password lock: the old password is never shown.
      helperSet: "Password set. Enter a new one to replace it",
      repeat: "Repeat password",
      missing: "Enter a password",
      differ: "Passwords differ",
      describe: "Locks with a password",
      confirmHeading: "Start {name} locked with a password?",
      confirmBody: "Stopped only with its password, or by unlocking in Settings",
      confirmAction: "Start locked",
    },
  },

  lists: {
    title: "Block lists",
    noun: "block list",
    description: "What to block: sites, programs and apps. Blocks can share a list",
    newLabel: "New block list",
    editTitle: "Edit block list",
    emptyTitle: "No block lists yet",
    emptyText: "Add sites, programs or apps to close, and how to close them",
    editorDescription: "Running blocks follow changes made here",
    // {names}: the locked blocks that use it, quoted.
    limited: "{names} uses it and is locked. You can add blocked entries, remove allowed sites and turn enforcers on, nothing else. It cannot be deleted",
    namePlaceholder: "e.g. Social media",
    nameMissing: "Name the block list",
    nothing: "Nothing to block",
    notUsed: "Not used by a block",
    // Above an editor section's checkboxes: with lists, and without.
    howLegend: "How",
    enforcersLegend: "Enforcers",

    // The lists of a block list. `count` is the tile's line.
    filters: {
      label: "Sites to block",
      placeholder: "youtube.com",
      helper: "One per line, with subdomains and pages. * matches any part, a lone * blocks all sites",
      count: { one: "{n} site blocked", other: "{n} sites blocked" },
    },
    allowed: {
      label: "Always allowed",
      placeholder: "google.com",
      helper: "Exceptions to the blocked sites. They always win",
      count: { one: "{n} always allowed", other: "{n} always allowed" },
    },
    programs: {
      label: "Programs to close",
      placeholder: "steam.exe",
      helper: "One per line, e.g. steam.exe. * matches any part. Windows programs cannot be added",
      count: { one: "{n} program closed", other: "{n} programs closed" },
    },
    store_apps: {
      label: "Microsoft Store apps to close",
      placeholder: "",
      helper: "Tick the apps to close. Windows apps are not listed",
      loading: "Loading installed apps",
      failed: "Could not list installed apps",
      count: { one: "{n} Microsoft Store app closed", other: "{n} Microsoft Store apps closed" },
    },
    titles: {
      label: "Window titles to close",
      placeholder: "*YouTube*",
      helper: "One per line, the whole title. * matches any part, case is ignored. Closes the whole window with all its tabs",
      count: { one: "{n} window title closed", other: "{n} window titles closed" },
    },

    // The editor's sections, with the enforcers in each. `short` is the tile's line for an
    // enforcer that needs no list.
    sections: {
      websites: {
        title: "Websites",
        description: "Closes listed pages in Chrome, Edge, Brave and Firefox",
      },
      programs: {
        title: "Programs",
        description: "Asks listed programs to close, ends them after 3 seconds",
      },
      store: {
        title: "Microsoft Store apps",
        description: "Asks ticked apps to close, ends them after 3 seconds",
      },
      titles: {
        title: "Windows by title",
        description: "Closes windows whose title matches",
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
    description: "When a block is active: a weekly schedule, an unplugged flash drive or a combination. No condition means always active",
    newLabel: "New condition",
    editTitle: "Edit condition",
    emptyTitle: "No conditions yet",
    emptyText: "Blocks without a condition are always active. Add one such as work hours",
    editorDescription: "Running blocks follow changes made here",
    // {names}: the locked blocks that use it, quoted.
    limited: "{names} uses it and is locked. A schedule can only gain times, a drive cannot change and a combination can only get more active. It cannot be deleted",
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Name the condition",
    typeLabel: "Type",
    typeSchedule: "Weekly schedule",
    typeDrive: "Flash drive",
    typeCombined: "Combination",
    // {names}: the conditions combined, as links.
    activeAny: "Active when any of {names}",
    activeAll: "Active when all of {names}",
    // A combination: which conditions, and whether any or all of them make it active. Never of other combinations.
    combine: {
      modeLabel: "Active when",
      modeHelper: "Any needs one, all needs every one",
      any: "Any is active",
      all: "All are active",
      legend: "Conditions",
      none: "Combining needs at least two other conditions. Combinations cannot hold combinations",
      missing: "Choose at least two",
    },
    // {times}: the schedule in words; {drive}: the drive's name, quoted.
    active: "Active {times}",
    activeDrive: "Active while {drive} is unplugged",
    notUsed: "Not used by a block",
    // Above the week grid.
    nothingMarked: "Mark when blocks are active",
    gridHelp: "Drag over the week to mark active times, start on a marked time to clear it",
    // The week grid's column heads, Monday first.
    days: { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" },
    // A flash drive condition: the drive is the key. The connected drives are looked up when the
    // editor opens and on "Scan again".
    drive: {
      field: "Flash drive",
      helper: "Active while unplugged. Plugging it in switches blocks off and opens its lock. Drives without a serial number are not listed",
      choose: "Choose a flash drive",
      scanning: "Looking for drives",
      none: "No flash drive found. Plug it in and scan again",
      // {name}: the drive's name and serial number, for the one the condition was set to that is not connected now.
      away: "{name} (unplugged)",
      scan: "Scan again",
      missing: "Choose a flash drive",
    },
  },

  // Deleting a block, block list or condition. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?",
    body: "Deletes the {noun} for good",
    // What it does to the blocks that use a block list or condition; {names}: those blocks, quoted.
    deletesToo: "Also deleted, no other block list: {names}",
    keepsOthers: "Keeps its other block lists: {names}",
    becomesAlways: "Always active from now: {names}",
    losesLock: "No lock from now: {names}",
    shrinksCombination: "Continues without it: {names}",
    removesCombination: "Also deleted, one condition left: {names}",
    stopsIt: "It is running and will be stopped",
    failed: "Could not delete the {noun}",
  },

  editor: {
    saveFailed: "Could not save the {noun}",
    lockedTitle: "Locked",
    limitedTitle: "Only stricter changes while locked",
  },

  settings: {
    title: "Settings",
    unlockTitle: "Unlock",
    unlockText: "Ends every running block, locked ones too. Blocks, block lists and conditions stay. The only way to end a locked block early. Request it, wait for the countdown, then unlock. Abort keeps everything locked",
    // No delay set: the state of a clean install.
    unlockFree: "Ends every running block, locked ones too. Blocks, block lists and conditions stay. Free right now: set an unlock delay below for a countdown",
    // {delay}: how long it takes; {left}: the time left, in words.
    unlockAsk: "Unlocking takes {delay}. Request it to start the countdown",
    unlockWait: "Unlock ready in {left}. Abort to stay locked",
    unlockReady: "Unlock ready. Unlock now or abort to stay locked",
    unlockNothing: "Nothing is locked",
    unlock: "Unlock",
    request: "Request unlock",
    abort: "Abort",
    aborted: "Aborted, everything stays locked",
    confirmHeading: "End every running block?",
    confirmBody: "Locked blocks end too. Blocks, block lists and conditions stay",
    failed: "Not unlocked",
    unlocked: "Unlocked, every block ended",
    delayTitle: "Unlock delay",
    delayText: "Time between requesting and unlocking, up to 24 hours. A longer delay can always be set. A shorter one needs unlocking to be ready, or no locked block",
    delayLabel: "Delay (up to 24 hours)",
    delayPlaceholder: "e.g. 2h 30m or 0",
    setDelay: "Set delay",
    // {delay}: the new delay in words.
    delaySet: "Unlocking now takes {delay}",
    delayFree: "Unlocking is free",
    delayFailed: "Delay not changed",
    transferTitle: "Move to another computer",
    transferText: "Export saves all block lists, conditions and blocks to a file. Import replaces all of them with the file's contents. Password locks are left out and import without a lock",
    export: "Export",
    import: "Import",
    // {file}: the file's name.
    exported: "Exported to {file}",
    exportFailed: "Not exported",
    importHeading: "Replace everything with this file?",
    // {lists}, {conditions}, {blocks}: the counts below, as in the file.
    importBody: "{lists}, {conditions} and {blocks} replace your current ones. This cannot be undone",
    importLists: { one: "{n} block list", other: "{n} block lists" },
    importConditions: { one: "{n} condition", other: "{n} conditions" },
    importBlocks: { one: "{n} block", other: "{n} blocks" },
    // {names}: the blocks, quoted; {n}: how many.
    importUnlocked: {
      one: "{names} had a password lock and imports without a lock",
      other: "{names} had password locks and import without a lock",
    },
    // Said too when a block runs without being locked: it ends.
    importStops: "Running blocks without a lock are stopped",
    importAction: "Import",
    imported: "Imported",
    importFailed: "Not imported",
  },
};
