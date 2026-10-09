// Every text the window shows, in one place, so the copy can be fine-tuned without touching the
// page. `index.html` loads this file and looks texts up by their path (`t("sessions.title")`).
//
// - `{name}`, `{times}`, … are filled in by the page. Keep each placeholder a text uses (you may
//   move it); the comment above a group says what they stand for when it is not obvious.
// - A text with `one` and `other` is chosen by the number `{n}`.
// - Times (`2 hours`, `90 minutes`), weekly schedules (`Mon-Fri 08:00-17:00`) and the service's
//   own messages (why something was refused) come from the Rust code, so the window and the CLI
//   say the same; they are not in this file.
// - Style: GUI labels, short but complete, with little punctuation. A state is one word or two
//   ("Running", "Locked").
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
  },

  common: {
    cancel: "Cancel",
    // The title of a refusal shown in a dialog, above the reason.
    notSaved: "Could not be saved",
    // The title of what is wrong in a form, above the reason, while it is being filled in.
    cannotSave: "Cannot be saved",
    // The button of a list page: short, so it fits at any width.
    new: "New",
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
    // A target or condition that a session names but that no longer exists.
    gone: "(deleted)",
    // A name inside a sentence; {name}.
    quoted: "“{name}”",
    // Between names in a list of them.
    separator: ", ",
    // Kept apart from strings that happen to read the same: these mean the same wherever they are.
    type: "Type",
    // A delay lock's wait (a session's) or the unlock countdown (Settings): ending it, and once it is
    // over, locking again.
    keepLocked: "Keep locked",
    lockAgain: "Lock again",
    // Under the title of a target's or condition's editor.
    editorDescription: "Running sessions follow changes made here; a locked one only accepts stricter ones",
    // The service's words as a sentence of a question ({text}).
    sentence: "{text}.",
    // Before the last of a list: a, b and c.
    and: " and ",
    serviceDown: "Bullseye service not responding",
    // A target's or condition's tile while a locked session uses it.
    inUse: "Used by a locked session",
    // {names}: the sessions that use it, as links; {n} how many.
    // While a weekly schedule's description is being asked of the backend.
    weeklyStandIn: "a weekly schedule",
  },

  sessions: {
    title: "Sessions",
    noun: "session",
    description: "Targets under a condition and a lock. Sessions start and run independently",
    // A new one is named by the first free number.
    autoName: "Session {n}",
    newLabel: "New session",
    emptyTitle: "No sessions yet",
    emptyText: "Make a target first, then a session that uses it",
    nameMissing: "Name the session",

    // A session is set up on its tile: a row each for its targets, condition, allowance and lock,
    // each opening a menu, and the modal of a new session. {name}: the session's name, quoted.
    fields: {
      block: "Targets",
      // The Targets row's label: as many as it has.
      blockCount: { one: "Target", other: "Targets" },
      when: "Condition",
      always: "Always",
      unless: "Allowance",
      lock: "Lock",
      allowanceSettings: "Allowance settings",
      // A row's edit button, its tooltip and name.
      changeBlock: "Change targets",
      changeWhen: "Change condition",
      // The dialog of a row: {what}: the row ("Condition"), {name}: the session.
      panelHeading: "{what} of {name}",
      budgetHeading: "Time budget of {name}",
      pomodoroHeading: "Pomodoro cycle of {name}",
      delayedHeading: "Delayed start of {name}",
      saveFailed: "Could not save {name}",
      rename: "Rename",
      renameHeading: "Rename {name}",
    },

    // A session's tile: its Targets row with none, and its Condition row for a running session whose
    // condition was deleted (it keeps the rule it started with). {drive}: a flash drive's name, quoted.
    tile: {
      noTargets: "No targets",
      active: "Active by schedule",
      activeCombined: "Active by combination",
      activeWebhook: "Active by a webhook",
    },
    // The tag on a session's tile.
    state: {
      notRunning: "Not running",
      running: "Running",
      runningLocked: "Running · locked",
      inactive: "Inactive",
    },
    start: "Start",
    // Under a session shown on its own (a link to it), when there are others.
    showAll: "Show all sessions",
    stop: "Stop",
    requestStop: "Request stop",
    // The Lock row's settings button.
    lockSettings: "Lock settings",
    // The dialog with a lock's settings. {method}: the lock's name below (locks.*.label).
    lockHeading: "{method} lock for {name}",
    lockFailed: "Could not set the lock of {name}",
    lockNow: {
      heading: "Lock {name} now?",
      action: "Lock now",
    },
    startFailed: "Could not start {name}",
    stopFailed: "Could not stop {name}",
    // Stopping a session locked with a password.
    stopHeading: "Stop {name}?",
    stopBody: "Enter the password to stop this session",
    // Stopping a session locked with a text: the text is shown above the input.
    stopTextBody: "Type the text above exactly. Pasting is blocked",
    stopTextLabel: "Type the text here",
    stopTextDiffers: "Text does not match yet",
    stopPasswordMissing: "Enter the password",
  },

  // The question before a locked session is made stricter, like the one before it starts locked.
  // {names}: the session or sessions, quoted; {what}: what it will be, a sentence.
  stricter: {
    heading: "Make {names} stricter?",
    body: "{what} It cannot be loosened until {names} can be stopped. Only unlocking in Settings ends it early",
    action: "Make stricter",
    noAllowance: "Its allowance is taken away, so nothing pauses it.",
    shared: "These changes only make what it uses stricter.",
  },
  // The units of a duration field (`duration.js`).
  // A target's or condition's tile: its rows' labels (a session's are `sessions.fields`), and what the
  // Used by row says when no session uses it.
  tiles: {
    blocks: "Blocks",
    when: "Active",
    usedBy: "Used by",
    none: "No session",
  },
  // A duration: a number and its unit (`duration.js`).
  duration: { unit: "Unit", d: "Days", h: "Hours", m: "Minutes", s: "Seconds" },
  // A time given as how long or as when (`timing.js`), for a timed lock and a delayed start.
  timing: {
    date: "Date",
    datePlaceholder: "yyyy-mm-dd",
    dateMissing: "Choose a date",
    time: "Time",
    timePlaceholder: "hh:mm",
    timeMissing: "Enter a time, e.g. 18:00",
    lock: {
      kind: "Locked",
      for: "For a while",
      until: "Until a time",
      field: "Locked for",
      helper: "",
    },
    delayed: {
      kind: "Enforcing starts",
      after: "After a while",
      from: "From a time",
      field: "Enforcing starts after",
      helper: "Counted from the start of the session",
    },
  },
  // The lock methods, as chosen on a session's tile. `describe` is what the lock is, in the question
  // when a locked session's lock is made stricter; `confirm*` is the question before starting a locked session, and `confirmAction`
  // its button, the same for every method: a few words, as Carbon's modal buttons have one fixed height.
  locks: {
    confirmAction: "Start locked",
    none: {
      label: "No lock",
      describe: "No active lock",
    },
    // {duration}: the length in words; {when}: the end written out (Tuesday 6 October 2026, 18:00);
    // {left}: how long from now.
    timed: {
      label: "Timed",
      describeFor: "Locks for {duration}",
      describeUntil: "Locks until {when}",
      forConfirmHeading: "Start {name} for {duration}?",
      forConfirmBody: "Locked for {duration}. Only unlocking in Settings ends it early",
      untilConfirmHeading: "Start {name} until {when}?",
      untilConfirmBody: "Locked until {when}, {left} from now. Only unlocking in Settings ends it early",
    },
    until_restart: {
      label: "Until restart",
      describe: "Locks until Windows restarts",
      confirmHeading: "Start {name} until Windows restarts?",
      confirmBody: "Locked until Windows restarts. Shutting down with Fast Startup is not a restart. Only unlocking in Settings ends it early",
    },
    condition: {
      label: "Condition",
      field: "Locked during",
      helper: "Locked while this condition is active",
      missing: "Choose a condition",
      // Chosen when there is no condition yet: offers to make one, which the lock is then set to
      // (headed conditions.emptyTitle, its button conditions.newLabel).
      noneBody: "A condition lock needs a condition. Make one now and use it for this lock?",
      // What the lock is, for the Lock row's question. {name}: the condition, a link.
      describe: "Locks during {name}",
      describeDrive: "Locks while {name} is unplugged",
      describeCombined: "Locks while {name} is true",
      confirmHeading: "Start {name} locked during {condition}?",
      // {detail}: nothing for a schedule, the drive for a flash drive.
      confirmBody: "Locked while {condition} is active{detail}. Otherwise you can stop it. Only unlocking in Settings ends it early",
      driveDetail: " (while {drive} is unplugged)",
    },
    delay: {
      label: "Delay",
      field: "Time to wait before stopping",
      helper: "Stopping is asked for, counts down this long and can be aborted. At most 24 hours",
      describe: "Locks until stopping has waited {delay}",
      confirmHeading: "Start {name} locked by a delay?",
      confirmBody: "Stopping takes {delay} after asking for it, or unlocking in Settings",
    },
    text: {
      label: "Custom text",
      field: "Text to type",
      placeholder: "e.g. I will not scroll tonight",
      helper: "Stopping needs this text typed exactly, no pasting. Anyone who sees the session sees the text",
      missing: "Enter the text",
      describe: "Locks until its text is typed",
      confirmHeading: "Start {name} locked by a custom text?",
      confirmBody: "Stopped only by typing its text here, or by unlocking in Settings",
    },
    password: {
      label: "Password",
      field: "Password",
      helper: "Best set by someone else. Only a hash is kept",
      // When the session already has a password lock: the old password is never shown.
      helperSet: "Password set. Enter a new one to replace it",
      repeat: "Repeat password",
      missing: "Enter a password",
      differ: "Passwords differ",
      describe: "Locks with a password",
      confirmHeading: "Start {name} locked with a password?",
      confirmBody: "Stopped only with its password, or by unlocking in Settings",
    },
  },

  targets: {
    title: "Targets",
    noun: "target",
    description: "What to block: sites, programs and apps. Several sessions can share a target",
    autoName: "Target {n}",
    newLabel: "New target",
    editTitle: "Edit target",
    emptyTitle: "No targets yet",
    emptyText: "Add sites, programs or apps to close, and how to close them",
    // {names}: the locked sessions that use it, quoted.
    limited: "{names} uses it and is locked. You can add blocked entries, remove always allowed ones and turn actions on, nothing else. It cannot be deleted",
    namePlaceholder: "e.g. Social media",
    nameMissing: "Name the target",
    nothing: "Nothing to block",
    // Above an editor section's checkboxes: with lists, and without.
    howLegend: "How",
    actionsLegend: "Actions",

    // The lists of a target. `count` is what a target tile's row says of it.
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
      count: { one: "{n} site always allowed", other: "{n} sites always allowed" },
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
      // An app on the list that this PC has no app of that family name for (it came with an import); {name}:
      // the name in its family name.
      notInstalled: "{name} (not found on this PC)",
      count: { one: "{n} Microsoft Store app closed", other: "{n} Microsoft Store apps closed" },
    },
    titles: {
      label: "Window titles",
      placeholder: "*YouTube*",
      helper: "One per line, the whole title. * matches any part, case is ignored, a lone * matches every window but Bullseye. The whole window goes, with all its tabs",
      count: { one: "{n} window title", other: "{n} window titles" },
    },
    allowed_titles: {
      label: "Always allowed",
      placeholder: "*Visual Studio Code",
      helper: "Exceptions to the window titles. They always win",
      count: { one: "{n} title always allowed", other: "{n} titles always allowed" },
    },

    // The editor's sections, with the actions in each. `short` is the target tile's row for an
    // action that needs no list.
    sections: {
      websites: {
        title: "Websites",
        description: "Closes listed pages in Chrome, Edge, Brave and Firefox",
      },
      programs: {
        title: "Programs",
        description: "Asks listed programs to close, ends them after 3 seconds",
      },
      // `short`: a target tile's row label, where the title is too long for it.
      store: {
        title: "Microsoft Store apps",
        short: "Store apps",
        description: "Asks ticked apps to close, ends them after 3 seconds",
      },
      titles: {
        title: "Windows by title",
        short: "Titles",
        description: "Closes or minimizes windows whose title matches",
      },
      other: {
        title: "Other",
      },
    },
    actions: {
      chromium_tabs: { label: "Close blocked tabs in Chromium" },
      chromium_split_view: { label: "Close blocked pages in Chromium split view" },
      chromium_app_windows: { label: "Close blocked app windows in Chromium" },
      firefox_tabs: { label: "Close blocked tabs in Firefox" },
      firefox_split_view: { label: "Close blocked pages in Firefox split view" },
      // The window title section's choice: closing (`close`, no action of its own) or minimizing.
      close: "Close these windows",
      window_titles_minimize: { label: "Minimize these windows" },
      obsidian_web_viewer: { label: "Close Obsidian when web viewer is active", short: "Closes Obsidian's web viewer" },
    },
  },

  conditions: {
    title: "Conditions",
    noun: "condition",
    description: "When a session is active: a schedule, an unplugged drive or a combination",
    autoName: "Condition {n}",
    newLabel: "New condition",
    editTitle: "Edit condition",
    emptyTitle: "No conditions yet",
    emptyText: "Sessions without a condition are always active. Add one such as work hours",
    // {names}: the locked sessions that use it, quoted.
    held: "{names} uses it and is locked. A drive cannot change. It cannot be deleted",
    limited: "{names} uses it and is locked. A schedule can only gain times, a drive cannot change and a combination can only get more active. It cannot be deleted",
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Name the condition",
    // A webhook: one web address asked every so often; the true matcher first, then the false one, else the fallback.
    webhook: {
      url: "Web address",
      urlHelper: "One address, GET parameters can be part of it. No tokens or headers",
      urlWrong: "Enter an address starting with http:// or https://",
      method: "Method",
      methods: { get: "GET", post: "POST" },
      every: "Ask every",
      everyHelper: "At least a minute",
      everyWrong: "Ask at most once a minute",
      requestTitle: "Request",
      requestText: "The web address that is asked, and how often. No tokens or headers",
      // First what the condition is by default, then the one answer that makes it the other.
      defaultTitle: "By default",
      defaultText: "What the condition is unless the answer matches below, also when no answer comes",
      fallback: "The condition is",
      fallbackHelper: "Also when the last answer is older than three times the interval",
      fallback_active: "Active",
      fallback_inactive: "Not active",
      // The match's heading: the state it gives, the other one than the default.
      matchInactive: "Not active when",
      matchActive: "Active when",
      matchText: "The answer that turns the condition around",
      testText: "Asks the address once, with these settings",
      matchOn: "Match on",
      on: { status: "HTTP code", body: "Body" },
      statusWrong: "Enter an HTTP status code, e.g. 200",
      bodyPlaceholder: "e.g. on",
      bodyHelper: "The whole body must equal this, white space around it aside",
      bodyMissing: "Enter the body text",
      test: "Test",
      testing: "Asking …",
      testNot: "Not asked",
      testFailed: "No answer",
      answer: "HTTP {status}, body “{body}”",
      matchedActive: "Matches: active",
      matchedInactive: "Matches: not active",
      unmatchedActive: "No match: active by default",
      unmatchedInactive: "No match: not active by default",
    },
    typeSchedule: "Weekly schedule",
    typeDrive: "Flash drive",
    typeWebhook: "Webhook",
    // A webhook condition tile: {url}.
    activeWebhook: "Active by the answer of {url}",
    typeCombined: "Combination",
    // {names}: the conditions combined, as links.
    activeAny: "Active when any of {names} is true",
    activeAll: "Active when all of {names} are true",
    // A combination: which conditions, and whether any or all of them make it active. Never of other combinations.
    combine: {
      text: "Two or more other conditions, of which any or all must be active",
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
    // Above the week grid.
    nothingMarked: "Mark when sessions are active",
    // The heading of a weekly schedule's grid; its line is `gridHelp`.
    weekTitle: "Week",
    gridHelp: "Drag over the week to mark active times, start on a marked time to clear it",
    // The week grid's column heads, Monday first.
    days: { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" },
    // A flash drive condition: the drive is the key. The connected drives are looked up when the
    // editor opens and on "Scan again".
    drive: {
      title: "Drive",
      text: "The drive that is the key: the condition is active while it is unplugged",
      helper: "Active while unplugged. Plugging it in switches sessions off and opens its lock. Drives without a serial number are not listed",
      scanning: "Looking for drives",
      none: "No flash drive found. Plug it in and scan again",
      // {name}: the drive's name and serial number, for the one the condition was set to that is not connected now.
      away: "{name} (unplugged)",
      scan: "Scan again",
      missing: "Choose a flash drive",
    },
  },

  // A session's allowance, set on its tile (its settings in the modal).
  allowance: {
    typeHelper: "As long as the session runs. A running session takes changes at once; a locked one only stricter ones",
    // The kinds, in the session's Allowance row and in the allowance's form.
    type: {
      none: "No allowance",
      budget: "Time budget",
      pomodoro: "Pomodoro cycle",
      delayed: "Delayed start",
    },
    // What a budget's reset every so long, or a Pomodoro cycle, is counted from.
    anchor: {
      start: "The start of the session",
      clock: "The clock, from midnight",
    },
    // A time budget.
    budget: {
      allowedLabel: "Time allowed",
      allowedHelper: "Counts while a blocked page is shown in a tab, a blocked window or app is open, or a blocked program runs",
      scopeLabel: "Applies to",
      scopeHelper: "Each item is a site, program, app or window title on the target, with its own time",
      total: "The whole session",
      each: "Each item",
      resetLabel: "Time is given back",
      resetHelper: "What was used becomes available again",
      never: "Never while the session runs",
      every: "Every so long",
      condition: "Each time the condition turns on",
      // Given back every so long.
      everyLabel: "Every",
      everyHelper: "Counted from the point chosen below",
      anchorLabel: "Counted from",
      anchorHelper: "On the clock, every 1d comes at midnight and every 1h on the hour. It has to fit a day evenly",
    },
    // A Pomodoro cycle: blocked first, then everything is allowed, over and over.
    cycle: {
      blockedLabel: "Blocked for",
      allowedLabel: "Allowed for",
      helper: "Everything the session closes is allowed during the allowed time",
      anchorLabel: "Cycles count from",
      anchorHelper: "On the clock, a cycle of an hour starts at every full hour. It has to fit a day evenly",
    },
    // Under the fields: the terms in words, from the backend.
    describing: "Checking",
  },

  // Deleting a session, target or condition. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?",
    body: "Deletes the {noun} for good",
    // What it does to the sessions that use a target or condition; {names}: those sessions, quoted.
    losesTarget: "Goes on without it: {names}",
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
    description: "Unlocking, the unlock delay, and moving everything to another computer",
    unlockText: "Ends every running session, locked ones too. Sessions, targets and conditions stay. The only way to end a locked session early. Request it, wait for the countdown, then unlock. Keep locked cancels it and keeps everything locked; once it is ready the button reads Lock again",
    // No delay set: the state of a clean install.
    unlockFree: "Ends every running session, locked ones too. Sessions, targets and conditions stay. Free right now: set an unlock delay below for a countdown",
    // {delay}: how long it takes; {left}: the time left, in words.
    unlockAsk: "Unlocking takes {delay}. Request it to start the countdown",
    unlockWait: "Unlock ready in {left}. Keep locked to cancel it",
    unlockReady: "Unlock ready. Unlock now or lock again",
    unlockNothing: "Nothing is locked",
    // The other way to unlock lets go: no countdown needed.
    unlockReadyWay: "Unlock ready: the other way to unlock is open. Unlock now",
    // The other way to unlock, besides the delay. Its fields are the lock methods' own, worded for unlocking.
    way: {
      title: "Another way to unlock",
      text: "Besides the delay, which is always there. Unlocking is ready when the delay is over or this way lets go, one is enough. Setting one needs unlocking to be ready, or nothing to be locked",
      heading: "{method} to unlock",
      saved: "Saved",
      removed: "Only the delay unlocks now",
      failed: "Could not set the way to unlock",
      noConditions: "Make a condition first: this way is locked while a condition is active",
      answerText: "Unlock with text",
      answerPassword: "Unlock with password",
      answerHeading: "Unlock?",
      answerBody: "Enter the password to end every running session now",
    },
    unlock: "Unlock",
    request: "Request unlock",
    aborted: "Everything stays locked",
    confirmHeading: "End every running session?",
    confirmBody: "Locked sessions end too. Sessions, targets and conditions stay",
    failed: "Not unlocked",
    unlocked: "Unlocked, every session ended",
    delayTitle: "Unlock delay",
    delayText: "Time between requesting and unlocking, up to 24 hours. A longer delay can always be set. A shorter one needs unlocking to be ready, or no locked session",
    delayLabel: "Delay (up to 24 hours)",
    setDelay: "Set delay",
    changeDelay: "Change delay",
    // {delay}: the new delay in words.
    delaySet: "Unlocking now takes {delay}",
    delayFree: "Unlocking is free",
    delayFailed: "Delay not changed",
    transferTitle: "Move to another computer",
    transferText: "Export saves all targets, conditions and sessions to a file. Import replaces all of them with the file's contents. Password locks go with their hash, so keep the file private",
    export: "Export",
    import: "Import",
    // {file}: the file's name.
    exported: "Exported to {file}",
    exportFailed: "Not exported",
    importHeading: "Replace everything with this file?",
    // {targets}, {conditions}, {sessions}: the counts below, as in the file.
    importBody: "{targets}, {conditions} and {sessions} replace your current ones. This cannot be undone",
    importTargets: { one: "{n} target", other: "{n} targets" },
    importConditions: { one: "{n} condition", other: "{n} conditions" },
    importSessions: { one: "{n} session", other: "{n} sessions" },
    // {names}: the sessions, quoted; {n}: how many.
    importUnlocked: {
      one: "{names} has a password lock without its hash in the file and imports without a lock",
      other: "{names} have password locks without their hash in the file and import without a lock",
    },
    // Store apps in the file that are not installed on this PC, which are left out: a Store app is one
    // exact package, and this PC may have the app as another one, or not at all. {apps}: each as
    // importAppIn, {app} its name and {target} its target, quoted.
    importAppsLeftOut: "Left out, not installed on this PC: {apps}",
    importAppIn: "{app} in {target}",
    importAppsUnknown: "Could not tell which Store apps are installed here, so all are kept",
    // Said too when a session runs without being locked: it ends.
    importStops: "Running sessions without a lock are stopped",
    imported: "Imported",
    importFailed: "Not imported",
  },
};
