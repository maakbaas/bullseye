// Every text the window shows, in one place, so the copy can be fine-tuned without touching the
// page. `index.html` loads this file and looks texts up by their path (`t("sessions.title")`).
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
    sessions: "Sessions",
    targets: "Targets",
    conditions: "Conditions",
    settings: "Settings",
  },

  common: {
    cancel: "Cancel",
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
    // Before the last of a list: a, b and c.
    and: " and ",
    serviceDown: "Bullseye service not responding",
    // A target's or condition's tile while a locked session uses it.
    inUse: "Used by a locked session",
    // {names}: the sessions that use it, as links; {n} how many.
    usedBy: { one: "Used by session {names}", other: "Used by sessions {names}" },
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
    noTargets: "A session needs at least one target",

    // A session is set up on its tile: a row each for its targets, condition, allowance and lock,
    // each opening a menu, and the modal of a new session. {name}: the session's name, quoted.
    fields: {
      block: "Targets",
      // The Targets row's label: as many as it has.
      blockCount: { one: "Target", other: "Targets" },
      targets: "Targets",
      chooseTargets: "Choose targets",
      newTarget: "New target",
      when: "Condition",
      always: "Always",
      newCondition: "New condition",
      unless: "Allowance",
      lock: "Lock",
      allowanceSettings: "Allowance settings",
      // A row's edit button, its tooltip and name. {what}: the row's label.
      change: "Change {what}",
      // Under a running session's condition. {change}: how long, in words.
      active: "Active now",
      activeFor: "Active for the next {change}",
      inactive: "Inactive now",
      inactiveFor: "Inactive for the next {change}",
      noAllowance: "No allowance",
      budget: "Time budget…",
      pomodoro: "Pomodoro cycle…",
      budgetHeading: "Time budget of {name}",
      pomodoroHeading: "Pomodoro cycle of {name}",
      saveFailed: "Could not save {name}",
      rename: "Rename",
      renameHeading: "Rename {name}",
    },

    // A session's tile. A weekly schedule is named, never written out (its times are on the
    // Conditions page, and only there); {drive}: a flash drive's name, quoted; {name}: the
    // condition's name; {left} and {change}: times in words.
    tile: {
      noTargets: "No targets",
      active: "Active by schedule",
      activeCombined: "Active by combination",
      activeWebhook: "Active by a webhook",
      activeNamed: "Active during {name}",
      activeDrive: "Active while {drive} is unplugged",
      activeDriveNamed: "Active while {name} is unplugged",
      activeCombinedNamed: "Active while {name} is true",
      always: "Always active",
      unknownLock: "Unknown lock",
      lockedPassword: "Locked with a password",
      lockedText: "Locked until its text is typed",
      lockedDelay: "Locked, stopping takes {delay}",
      delayWait: "Can be stopped in {left}",
      delayReady: "Ready to stop",
      lockedRestart: "Locked until Windows restarts",
      lockedFor: "Locked for the next {left}",
      notLocked: "No active lock",
      noAllowance: "No allowance",
      // A running session's condition line: its state now and why ({change}: when that changes, if known).
      // A schedule is "during" / "outside", a drive or a combination with one is "since …" (it is plugged
      // in or out by hand, so it has no time).
      activeDuring: "Active during {name}",
      activeDuringFor: "Active during {name} for the next {change}",
      inactiveOutside: "Inactive outside {name}",
      inactiveOutsideFor: "Inactive outside {name} for the next {change}",
      activeSinceDrive: "Active since {name} is unplugged",
      inactiveSinceDrive: "Inactive since {name} is plugged in",
      activeSinceTrue: "Active since {name} is true",
      inactiveSinceFalse: "Inactive since {name} is false",
      conditionGone: "a condition",
      // A running session's allowance line (not running, the backend's short label: "Allowed 30 minutes a
      // day"). A Pomodoro: {change}: the time until it turns.
      allowanceOpenFor: "Allowed for the next {change}",
      allowanceBlockedFor: "Allowed again in {change}",
      allowanceOpen: "Allowed now",
      allowanceBlocked: "Not allowed now",
      // A budget: {left}: the time left; {reset}: the time until it is given back.
      allowanceLeft: "{left} left to use",
      allowanceLeftReset: "{left} left to use, reset in {reset}",
      allowanceUsedUp: "Allowed time used up",
      allowanceUsedUpReset: "Allowed time used up, reset in {reset}",
      // A budget for each item: {items}: "youtube.com 12 minutes left, …".
      allowanceItems: "Allowed per item: {items}",
      allowanceItem: "{item} {left} left",
      allowanceItemUsedUp: "{item} used up",
    },
    // The tag on a session's tile.
    state: {
      notRunning: "Not running",
      running: "Running",
      runningLocked: "Running · locked",
      inactive: "Inactive",
    },
    start: "Start",
    stop: "Stop",
    requestStop: "Request stop",
    abortStop: "Keep locked",
    // Once the wait is over there is nothing left to keep: it locks again.
    relock: "Lock again",
    lockLabel: "Lock",
    // In a session's ⋮ menu: its lock's settings.
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
    stopPassword: "Password",
    // Stopping a session locked with a text: the text is shown above the input.
    stopTextBody: "Type the text above exactly. Pasting is blocked",
    stopTextLabel: "Type the text here",
    stopTextDiffers: "Text does not match yet",
    stopPasswordMissing: "Enter the password",
  },

  // The lock methods, as chosen on a session's tile. `describe` is the tile's line for a session that
  // is not running; `confirm*` is the question before starting a locked session. A confirmAction is
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
      // {name}: the condition, a link. Worded like the condition's line on a tile ("Active during …"),
      // "Locks" for a session that is not running and "Locked" for one that is, locking now or not.
      describe: "Locks during {name}",
      describeDrive: "Locks while {name} is unplugged",
      describeCombined: "Locks while {name} is true",
      // Running, by its state now ({change}: when it changes, when that is known). A flash drive is the key.
      lockedDuring: "Locked during {name}",
      lockedDuringFor: "Locked during {name} for the next {change}",
      unlockedOutside: "Unlocked outside {name}",
      unlockedOutsideFor: "Unlocked outside {name} for the next {change}",
      lockedDrive: "Locked since {name} is unplugged",
      unlockedDrive: "Unlocked since {name} is plugged in",
      lockedTrue: "Locked since {name} is true",
      unlockedFalse: "Unlocked since {name} is false",
      // The condition has been deleted from a running session.
      goneName: "a condition",
      describeGone: "Condition deleted",
      confirmHeading: "Start {name} locked during {condition}?",
      // {detail}: nothing for a schedule, the drive for a flash drive.
      confirmBody: "Locked while {condition} is active{detail}. Otherwise you can stop it. Only unlocking in Settings ends it early",
      driveDetail: " (while {drive} is unplugged)",
      confirmAction: "Start locked",
    },
    delay: {
      label: "Delay",
      field: "Time to wait before stopping",
      placeholder: "e.g. 30m",
      helper: "Stopping is asked for, counts down this long and can be aborted. At most 24 hours",
      describe: "Locks until stopping has waited {delay}",
      confirmHeading: "Start {name} locked by a delay?",
      confirmBody: "Stopping takes {delay} after asking for it, or unlocking in Settings",
      confirmAction: "Start locked",
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
      confirmAction: "Start locked",
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
      confirmAction: "Start locked",
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
    editorDescription: "Running sessions follow changes made here; a locked one only accepts stricter ones",
    // {names}: the locked sessions that use it, quoted.
    limited: "{names} uses it and is locked. You can add blocked entries, remove always allowed ones and turn actions on, nothing else. It cannot be deleted",
    namePlaceholder: "e.g. Social media",
    nameMissing: "Name the target",
    nothing: "Nothing to block",
    notUsed: "Not used by a session",
    // Above an editor section's checkboxes: with lists, and without.
    howLegend: "How",
    actionsLegend: "Actions",

    // The lists of a target. `count` is the tile's line.
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

    // The editor's sections, with the actions in each. `short` is the tile's line for an
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
      store: {
        title: "Microsoft Store apps",
        description: "Asks ticked apps to close, ends them after 3 seconds",
      },
      titles: {
        title: "Windows by title",
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
    editorDescription: "Running sessions follow changes made here; a locked one only accepts stricter ones",
    // {names}: the locked sessions that use it, quoted.
    held: "{names} uses it and is locked. A drive cannot change. It cannot be deleted",
    limited: "{names} uses it and is locked. A schedule can only gain times, a drive cannot change and a combination can only get more active. It cannot be deleted",
    namePlaceholder: "e.g. Work hours",
    nameMissing: "Name the condition",
    typeLabel: "Type",
    // A webhook: one web address asked every so often; the true matcher first, then the false one, else the fallback.
    webhook: {
      url: "Web address",
      urlHelper: "One address, GET parameters can be part of it. No tokens or headers",
      urlWrong: "Enter an address starting with http:// or https://",
      method: "Method",
      every: "Ask every (minutes)",
      everyHelper: "At least 1",
      everyWrong: "Enter a whole number of minutes, at least 1",
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
      testTitle: "Test",
      testText: "Asks the address once, with these settings",
      matchOn: "Match on",
      on: { status: "HTTP code", body: "Body" },
      status: "HTTP code",
      statusWrong: "Enter an HTTP status code, e.g. 200",
      body: "Body",
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
      title: "Combination",
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
    notUsed: "Not used by a session",
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
      field: "Flash drive",
      helper: "Active while unplugged. Plugging it in switches sessions off and opens its lock. Drives without a serial number are not listed",
      choose: "Choose a flash drive",
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
    typeLabel: "Type",
    typeHelper: "As long as the session runs. A running session takes changes at once; a locked one only stricter ones",
    type: {
      none: "None",
      budget: "Time budget",
      pomodoro: "Pomodoro",
    },
    // A time budget.
    budget: {
      allowedLabel: "Time allowed",
      allowedPlaceholder: "e.g. 30m",
      allowedHelper: "e.g. 30m, 2h 30m. Counts while a blocked page is shown in a tab, a blocked window or app is open, or a blocked program runs",
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
      everyPlaceholder: "e.g. 1d",
      everyHelper: "e.g. 1d, 12h, 90m",
      anchorLabel: "Counted from",
      anchorHelper: "On the clock, every 1d comes at midnight and every 1h on the hour. It has to fit a day evenly",
      start: "The start of the session",
      clock: "The clock, from midnight",
    },
    // A Pomodoro cycle: blocked first, then everything is allowed, over and over.
    cycle: {
      blockedLabel: "Blocked for",
      blockedPlaceholder: "e.g. 50m",
      allowedLabel: "Allowed for",
      allowedPlaceholder: "e.g. 10m",
      helper: "e.g. 30m, 2h 30m. Everything the session closes is allowed during this time",
      anchorLabel: "Cycles count from",
      anchorHelper: "On the clock, a cycle of an hour starts at every full hour. It has to fit a day evenly",
      start: "The start of the session",
      clock: "The clock, from midnight",
    },
    // Under the fields: the terms in words, from the backend.
    describing: "Checking",
  },

  // Deleting a session, target or condition. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?",
    body: "Deletes the {noun} for good",
    // What it does to the sessions that use a target or condition; {names}: those sessions, quoted.
    deletesToo: "Also deleted, no other target: {names}",
    keepsOthers: "Keeps its other targets: {names}",
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
    unlockTitle: "Unlock",
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
      stateNone: "Only the delay unlocks",
      stateText: "Typing the text unlocks at once",
      statePassword: "The password unlocks at once",
      stateOpen: "Open now: its condition is not active, so unlocking needs no wait",
      stateCondition: "Locked while its condition is active",
      answerText: "Unlock with text",
      answerPassword: "Unlock with password",
      answerHeading: "Unlock?",
      answerBody: "Enter the password to end every running session now",
      answerTextBody: "Type the text above exactly. Pasting is blocked",
    },
    unlock: "Unlock",
    request: "Request unlock",
    abort: "Keep locked",
    relock: "Lock again",
    aborted: "Everything stays locked",
    confirmHeading: "End every running session?",
    confirmBody: "Locked sessions end too. Sessions, targets and conditions stay",
    failed: "Not unlocked",
    unlocked: "Unlocked, every session ended",
    delayTitle: "Unlock delay",
    delayText: "Time between requesting and unlocking, up to 24 hours. A longer delay can always be set. A shorter one needs unlocking to be ready, or no locked session",
    delayLabel: "Delay (up to 24 hours)",
    delayPlaceholder: "e.g. 2h 30m or 0",
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
    importAppsLeftOut: {
      one: "Left out, not installed on this PC: {apps}",
      other: "Left out, not installed on this PC: {apps}",
    },
    importAppIn: "{app} in {target}",
    importAppsUnknown: "Could not tell which Store apps are installed here, so all are kept",
    // Said too when a session runs without being locked: it ends.
    importStops: "Running sessions without a lock are stopped",
    importAction: "Import",
    imported: "Imported",
    importFailed: "Not imported",
  },
};
