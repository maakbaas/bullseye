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
    name: "Bullseye", // reviewed
  },

  // The tabs under the header, one per page.
  nav: {
    // What a screen reader calls the tabs.
    label: "Pages", // reviewed
  },

  common: {
    cancel: "Cancel", // reviewed
    // The title of a refusal shown in a dialog, above the reason.
    notSaved: "Cannot be saved", // reviewed
    // The title of what is wrong in a form, above the reason, while it is being filled in.
    cannotSave: "Cannot be saved", // reviewed
    // The button of a list page: short, so it fits at any width.
    new: "New", // reviewed
    save: "Save", // reviewed
    delete: "Delete", // reviewed
    edit: "Edit", // reviewed
    duplicate: "Duplicate", // reviewed
    // {name}: the copy's (the service names it after the original, with a number).
    duplicated: "Duplicated {name} from {original}", // reviewed
    duplicateFailed: "Could not duplicate {name}", // reviewed
    options: "Options", // reviewed
    locked: "Locked", // reviewed
    name: "Name", // reviewed
    // A target or condition that a session names but that no longer exists.
    gone: "(deleted)", // reviewed
    // A name inside a sentence; {name}.
    quoted: "“{name}”", // reviewed
    // Between names in a list of them.
    separator: ", ", // reviewed
    // Kept apart from strings that happen to read the same: these mean the same wherever they are.
    type: "Type", // reviewed
    // A delay lock's wait (a session's) or the unlock countdown (Settings): ending it, and once it is
    // over, locking again.
    keepLocked: "Keep locked", // reviewed
    lockAgain: "Lock again", // reviewed
    // Under the title of a target's or condition's editor.
    editorDescription: "Settings made here can be used as part of any number of sessions", // reviewed
    // The service's words as a sentence of a question ({text}).
    sentence: "{text}.", // reviewed
    // Before the last of a list: a, b and c.
    and: " and ", // reviewed
    serviceDown: "Bullseye service not responding", // reviewed
    // A target's or condition's tile while a locked session uses it.
    inUse: "Used by a locked session",
    // {names}: the sessions that use it, as links; {n} how many.
    // While a weekly schedule's description is being asked of the backend.
    weeklyStandIn: "a weekly schedule",
  },

  sessions: {
    title: "Sessions", // reviewed
    noun: "session", // reviewed
    description: "A session defines which items should be blocked, and how. Sessions start and run independently", // reviewed
    // A new one is named by the first free number.
    autoName: "Session {n}", // reviewed
    newLabel: "New session", // reviewed
    emptyTitle: "No sessions yet", // reviewed
    emptyText: "Click New to define your first session", // reviewed
    nameMissing: "Name the session", // reviewed

    // A session is set up on its tile: a row each for its targets, condition, allowance and lock,
    // each opening a menu, and the modal of a new session. {name}: the session's name, quoted.
    fields: {
      block: "Targets", // reviewed
      // The Targets row's label: as many as it has.
      blockCount: { one: "Target", other: "Targets" }, // reviewed
      when: "Condition", // reviewed
      always: "Always", // reviewed
      unless: "Allowance", // reviewed
      lock: "Lock", // reviewed
      allowanceSettings: "Allowance settings", // reviewed
      // A row's edit button, its tooltip and name.
      changeBlock: "Change targets", // reviewed
      changeWhen: "Change condition", // reviewed
      // The dialog of a row: {what}: the row ("Condition"), {name}: the session.
      panelHeading: "{what} of {name}", // reviewed
      budgetHeading: "Time budget for {name}", // reviewed
      pomodoroHeading: "Pomodoro cycle for {name}", // reviewed
      delayedHeading: "Delayed start for {name}", // reviewed
      breaksHeading: "Breaks for {name}", // reviewed
      saveFailed: "Could not save {name}", // reviewed
      rename: "Rename", // reviewed
      renameHeading: "Rename {name}", // reviewed
    },

    // What a session's Condition, Allowance and Lock rows say beside their value, written by the
    // service (`status.rs`) for the tile and the CLI. Stopped: what is set; running: what is
    // happening. {change}, {left}, {reset}, {delay}, {time}: times in one unit, rounded; {date}: 18:00
    // today, October 6 otherwise; {n}: a number of items; {condition}: a condition's name (a link).
    now: {
      // Stopped: what switches it on and off.
      always: "At all times", // reviewed
      bySchedule: "By schedule", // reviewed
      byDevice: "By device", // reviewed
      byWebhook: "By webhook", // reviewed
      byCombination: "By combination", // reviewed
      // Running: whether it is on, for how long or by what.
      active: "Active", // reviewed
      activeFor: "Active for {change}", // reviewed
      inactive: "Inactive", // reviewed
      inactiveFor: "Inactive for {change}", // reviewed
      activeByDevice: "Active by device", // reviewed
      inactiveByDevice: "Inactive by device", // reviewed
      activeByWebhook: "Active by webhook", // reviewed
      inactiveByWebhook: "Inactive by webhook", // reviewed
      activeByCombination: "Active by combination", // reviewed
      inactiveByCombination: "Inactive by combination", // reviewed
      // The allowance. Stopped: what it gives ({blocked} / {allowed} a Pomodoro cycle's).
      neverPaused: "Never paused", // reviewed
      cycle: "{blocked} / {allowed}", // reviewed
      after: "After {time}", // reviewed
      from: "From {date}", // reviewed
      budget: "Allowed for {time}", // reviewed
      perItem: "{time} per item", // reviewed
      breaks: { one: "{n} break of {time}", other: "{n} breaks of {time}" }, // reviewed
      // Running; while the condition is off, nothing is blocked for it to allow.
      allowedFor: "Allowed for {change}", // reviewed
      allowedIn: "Allowed in {change}", // reviewed
      blocked: "Blocked", // reviewed
      startsIn: "Starts in {change}", // reviewed
      started: "Started", // reviewed
      left: "{left} left", // reviewed
      back: "Back in {reset}", // reviewed
      usedUp: "Used up", // reviewed
      noneUsedUp: "None used up", // reviewed
      itemsUsedUp: { one: "{n} item used up", other: "{n} items used up" }, // reviewed
      // Breaks: how many are left (one going on: what is left of it, as `left`; none, with a reset coming: `back`).
      breaksLeft: { one: "{n} break left", other: "{n} breaks left" }, // reviewed
      noneLeft: "None left", // reviewed
      // The lock: the same words running or stopped where no time is in them.
      notLocked: "Not locked", // reviewed
      restart: "Until restart", // reviewed
      text: "Type to stop", // reviewed
      password: "Password to stop", // reviewed
      // Stopped.
      for: "For {time}", // reviewed
      until: "Until {date}", // reviewed
      delay: "Delay {delay}", // reviewed
      during: "During {condition}", // reviewed
      by: "By {condition}", // reviewed
      // Running: a timed lock's time left, a delay's wait once stopping is asked for, a condition's.
      timeLeft: "{left} left", // reviewed
      in: "In {left}", // reviewed
      locked: "Locked", // reviewed
      lockedFor: "Locked for {change}", // reviewed
      unlocked: "Unlocked", // reviewed
      unlockedFor: "Unlocked for {change}", // reviewed
      lockedByDevice: "Locked by device", // reviewed
      unlockedByDevice: "Unlocked by device", // reviewed
      lockedByWebhook: "Locked by webhook", // reviewed
      unlockedByWebhook: "Unlocked by webhook", // reviewed
      lockedByCombination: "Locked by combination", // reviewed
      unlockedByCombination: "Unlocked by combination", // reviewed
    },
    // A session's tile: its Targets row with none, and its Condition row for a running session whose
    // condition was deleted (it keeps the rule it started with). {drive}: a flash drive's name, quoted.
    tile: {
      noTargets: "No targets", // reviewed
      active: "Active by schedule", // reviewed
      activeCombined: "Active by combination", // reviewed
      activeWebhook: "Active by a webhook", // reviewed
    },
    // The tag on a session's tile.
    state: {
      notRunning: "Not running", // reviewed
      running: "Running", // reviewed
      runningLocked: "Locked", // reviewed
      inactive: "Inactive", // reviewed
    },
    start: "Start", // reviewed
    // Under a session shown on its own (a link to it), when there are others.
    showAll: "Show all sessions", // reviewed
    stop: "Stop", // reviewed
    requestStop: "Request stop", // reviewed
    // The Lock row's settings button.
    lockSettings: "Lock settings", // reviewed
    // The dialog with a lock's settings. {method}: the lock's name below (locks.*.label).
    lockHeading: "{method} lock for {name}", // reviewed
    lockFailed: "Could not set the lock of {name}",
    lockNow: {
      heading: "Lock {name} now?", // reviewed
      action: "Lock now", // reviewed
    },
    startFailed: "Could not start {name}", // reviewed
    stopFailed: "Could not stop {name}", // reviewed
    takeBreak: "Break", // reviewed
    endBreak: "End break", // reviewed
    breakFailed: "Could not change the break of {name}", // reviewed
    // Stopping a session locked with a password.
    stopHeading: "Stop {name}?", // reviewed
    stopBody: "Enter the password to stop this session", // reviewed
    // Stopping a session locked with a text: the text is shown above the input.
    stopTextBody: "Type the text exactly as shown. Pasting is blocked", // reviewed
    stopTextLabel: "Type the text here", // reviewed
    stopTextDiffers: "Text does not match yet", // reviewed
    stopPasswordMissing: "Enter the password", // reviewed
  },

  // The question before a locked session is made stricter, like the one before it starts locked.
  // {names}: the session or sessions, quoted; {what}: what it will be, a sentence.
  stricter: {
    heading: "Make {names} stricter?", // reviewed
    body: "{what} Undo not possible until {names} will be unlocked", // reviewed
    action: "Confirm", // reviewed
    noAllowance: "Its allowance will be taken away, so nothing pauses it.", // reviewed
    moreTargets: "It will also block {targets}.", // reviewed
    always: "It will be active at all times.", // reviewed
    shared: "These changes further restrict your session.", // reviewed
  },
  // The units of a duration field (`duration.js`).
  // A target's or condition's tile: its rows' labels (a session's are `sessions.fields`), and what the
  // Used by row says when no session uses it.
  tiles: {
    blocks: "Blocks", // reviewed
    when: "Active", // reviewed
    usedBy: "Used by", // reviewed
    none: "No session", // reviewed
  },
  // A duration: a number and its unit (`duration.js`).
  duration: { unit: "Unit", d: "Days", h: "Hours", m: "Minutes", s: "Seconds" }, // reviewed
  // A time given as how long or as when (`timing.js`), for a timed lock and a delayed start.
  timing: {
    date: "Date", // reviewed
    datePlaceholder: "yyyy-mm-dd", // reviewed
    dateMissing: "Choose a date", // reviewed
    time: "Time", // reviewed
    timePlaceholder: "hh:mm", // reviewed
    timeMissing: "Enter a time, e.g. 18:00", // reviewed
    lock: {
      kind: "Locked", // reviewed
      for: "For a certain time", // reviewed
      until: "Until a certain time", // reviewed
      field: "Locked for", // reviewed
    },
    delayed: {
      kind: "Enforcing starts", // reviewed
      after: "After a certain time", // reviewed
      from: "From a certain time", // reviewed
      field: "Enforcing starts after", // reviewed
      helper: "Counted from the start of the session", // reviewed
    },
  },
  // The lock methods, as chosen on a session's tile. `describe` is what the lock is, in the question
  // when a locked session's lock is made stricter; `confirm*` is the question before starting a locked session, and `confirmAction`
  // its button, the same for every method: a few words, as Carbon's modal buttons have one fixed height.
  locks: {
    confirmAction: "Start locked", // reviewed
    none: {
      label: "No lock", // reviewed
      describe: "No active lock", // reviewed
    },
    // {duration}: the length in words; {when}: the end written out (Tuesday 6 October 2026, 18:00);
    // {left}: how long from now.
    timed: {
      label: "Timed", // reviewed
      describeFor: "Locks for {duration}", // reviewed
      describeUntil: "Locks until {when}", // reviewed
      forConfirmHeading: "Start {name} for {duration}?", // reviewed
      forConfirmBody: "Locked for {duration}. Only unlocking in Settings can end it early", // reviewed
      untilConfirmHeading: "Start {name} until {when}?", // reviewed
      untilConfirmBody: "Locked until {when}, {left} from now. Only unlocking in Settings can end it early", // reviewed
    },
    until_restart: {
      label: "Until restart", // reviewed
      describe: "Locks until Windows restarts", // reviewed
      confirmHeading: "Start {name} until Windows restarts?", // reviewed
      confirmBody: "Locked until Windows restarts. Only unlocking in Settings can end it early", // reviewed
    },
    condition: {
      label: "Condition", // reviewed
      field: "Locked during", // reviewed
      helper: "Locked while this condition is active", // reviewed
      missing: "Choose a condition", // reviewed
      // Chosen when there is no condition yet: offers to make one, which the lock is then set to
      // (headed conditions.emptyTitle, its button conditions.newLabel).
      noneBody: "A condition lock needs a condition. Make one now and use it for this lock?", // reviewed
      // What the lock is, for the Lock row's question. {name}: the condition, a link.
      describe: "Locks during {name}", // reviewed
      describeDrive: "Locks while {name} is unplugged", // reviewed
      describeCombined: "Locks while {name} is true", // reviewed
      confirmHeading: "Start {name} locked during {condition}?", // reviewed
      // {detail}: nothing for a schedule, the drive for a flash drive.
      confirmBody: "Locked while {condition} is active{detail}. Only unlocking in Settings can end it early", // reviewed
      driveDetail: " (while {drive} is unplugged)", // reviewed
    },
    delay: {
      label: "Delay", // reviewed
      field: "Time to wait before unlocking is possible", // reviewed
      helper: "Maximum 24h", // reviewed
      describe: "Delay unlock by {delay}", // reviewed
      confirmHeading: "Start {name} locked by a delay?", // reviewed
      confirmBody: "Stopping takes {delay} after requesting it", // reviewed
    },
    text: {
      label: "Custom text", // reviewed
      field: "Text to type", // reviewed
      placeholder: "e.g. I will not scroll tonight", // reviewed
      helper: "To unlock, type in this text exactly, no pasting", // reviewed
      missing: "Enter the text", // reviewed
      describe: "Locks until its text is typed", // reviewed
      confirmHeading: "Start {name} locked by a custom text?", // reviewed
      confirmBody: "Stopped only by typing its text here, or by unlocking in Settings", // reviewed
    },
    random_text: {
      label: "Random text", // reviewed
      field: "Number of characters", // reviewed
      helper: "This is the number of characters that has to be typed over, no pasting", // reviewed
      // A text of the chosen length, to show how long it is; not the one that is drawn.
      example: "Example:", // reviewed
      // {n}: the number of characters.
      describe: { one: "Locks until {n} random character is typed", other: "Locks until {n} random characters are typed" }, // reviewed
      confirmHeading: "Start {name} locked by a random text?", // reviewed
      confirmBody: { one: "Stopped only by typing over {n} random character, or by unlocking in Settings", other: "Stopped only by typing over {n} random characters, or by unlocking in Settings" }, // reviewed
    },
    password: {
      label: "Password", // reviewed
      field: "Password", // reviewed
      helper: "Set your own, or ask somebody else to set it for you", // reviewed
      // When the session already has a password lock: the old password is never shown.
      helperSet: "Password set. Enter a new one to replace it", // reviewed
      repeat: "Repeat password", // reviewed
      missing: "Enter a password", // reviewed
      differ: "Passwords differ", // reviewed
      describe: "Locks with a password", // reviewed
      confirmHeading: "Start {name} locked with a password?", // reviewed
      confirmBody: "Stopped only with its password, or by unlocking in Settings", // reviewed
    },
  },

  targets: {
    title: "Targets", // reviewed
    noun: "target", // reviewed
    description: "What to block: sites, programs and apps. Several sessions can share a target", // reviewed
    autoName: "Target {n}", // reviewed
    newLabel: "New target", // reviewed
    editTitle: "Edit target", // reviewed
    emptyTitle: "No targets yet", // reviewed
    emptyText: "Add sites, programs or apps to close, and how to close them", // reviewed
    // {names}: the locked sessions that use it, quoted.
    limited: "In use by {names}. You can add blocked entries, remove always allowed ones and turn actions on", // reviewed
    namePlaceholder: "e.g. Social media", // reviewed
    nameMissing: "Name the target", // reviewed
    nothing: "Nothing to block", // reviewed
    // Above an editor section's checkboxes: with lists, and without.
    howLegend: "How", // reviewed
    actionsLegend: "Actions", // reviewed

    // The lists of a target. `count` is what a target tile's row says of it.
    filters: {
      label: "Sites to block", // reviewed
      placeholder: "youtube.com", // reviewed
      helper: "One per line. * matches any part, a lone * blocks all sites", // reviewed
      count: { one: "{n} site blocked", other: "{n} sites blocked" }, // reviewed
    },
    allowed: {
      label: "Always allowed", // reviewed
      placeholder: "google.com", // reviewed
      helper: "Exceptions to the blocked sites. They always take priority", // reviewed
      count: { one: "{n} site always allowed", other: "{n} sites always allowed" }, // reviewed
    },
    programs: {
      label: "Programs to close", // reviewed
      placeholder: "steam.exe", // reviewed
      helper: "One per line. * matches any part. Windows programs cannot be added", // reviewed
      count: { one: "{n} program closed", other: "{n} programs closed" }, // reviewed
    },
    store_apps: {
      label: "Microsoft Store apps to close", // reviewed
      helper: "Tick the apps to close", // reviewed
      loading: "Loading installed apps", // reviewed
      // {why}: what Windows said.
      failedWhy: "Could not list installed apps: {why}", // reviewed
      // An app on the list that this PC has no app of that family name for (it came with an import); {name}:
      // the name in its family name.
      notInstalled: "{name} (not found on this PC)", // reviewed
      count: { one: "{n} Microsoft Store app closed", other: "{n} Microsoft Store apps closed" }, // reviewed
    },
    titles: {
      label: "Window titles", // reviewed
      placeholder: "*YouTube*", // reviewed
      helper: "One per line, case-insensitive. * matches any part and a lone * matches every window but Bullseye itself", // reviewed
      count: { one: "{n} window title", other: "{n} window titles" }, // reviewed
    },
    allowed_titles: {
      label: "Always allowed", // reviewed
      placeholder: "*Visual Studio Code", // reviewed
      helper: "Exceptions that take precedence over the window titles list", // reviewed
      count: { one: "{n} title always allowed", other: "{n} titles always allowed" }, // reviewed
    },

    // The editor's sections, with the actions in each. `short` is the target tile's row for an
    // action that needs no list.
    sections: {
      websites: {
        title: "Websites", // reviewed
        description: "Closes pages of listed sites wherever they open: tabs, split views and app windows", // reviewed
      },
      programs: {
        title: "Programs", // reviewed
        description: "Asks listed programs to close, then kills them after 3 seconds", // reviewed
      },
      // `short`: a target tile's row label, where the title is too long for it.
      store: {
        title: "Microsoft Store apps", // reviewed
        short: "Store apps", // reviewed
        description: "Asks ticked apps to close, then kills them after 3 seconds", // reviewed
      },
      titles: {
        title: "Windows by title", // reviewed
        short: "Titles", // reviewed
        description: "Closes or minimizes windows whose title matches", // reviewed
      },
      other: {
        title: "Other", // reviewed
      },
    },
    actions: {
      // The window title section's choice: closing (`close`, no action of its own) or minimizing.
      close: "Close these windows", // reviewed
      window_titles_minimize: { label: "Minimize these windows" }, // reviewed
      obsidian_web_viewer: { label: "Close Obsidian when web viewer is active", short: "Closes Obsidian web viewer" }, // reviewed
      steam_browser: { label: "Close Steam Browser", short: "Closes Steam Browser" }, // reviewed
    },
  },

  conditions: {
    title: "Conditions", // reviewed
    noun: "condition", // reviewed
    description: "When a session is active. A schedule, an unplugged drive, a webhook or a combination", // reviewed
    autoName: "Condition {n}", // reviewed
    newLabel: "New condition", // reviewed
    editTitle: "Edit condition", // reviewed
    emptyTitle: "No conditions yet", // reviewed
    emptyText: "Sessions without a condition are always active. Add one such as work hours", // reviewed
    // {names}: the locked sessions that use it, quoted.
    held: "{names} uses it and is locked. Therefore this setting cannot be adjusted", // reviewed
    limited: "{names} uses it and is locked. A schedule can only gain times, a drive cannot change and a combination can only get more strict", // reviewed
    namePlaceholder: "e.g. Work hours", // reviewed
    nameMissing: "Name the condition", // reviewed
    // A webhook: one web address asked every so often; the true matcher first, then the false one, else the fallback.
    webhook: {
      url: "Web address", // reviewed
      urlPlaceholder: "https://example.com/state?hook=bullseye", // reviewed
      urlHelper: "One address, GET parameters can be part of it. No tokens or headers", // reviewed
      urlWrong: "Enter an address starting with http:// or https://", // reviewed
      method: "Method", // reviewed
      methods: { get: "GET", post: "POST" }, // reviewed
      every: "Ask every", // reviewed
      everyHelper: "At least a minute", // reviewed
      everyWrong: "Ask at most once a minute", // reviewed
      requestTitle: "Request", // reviewed
      requestText: "The web address for the request and how often it will be polled to retrieve its status", // reviewed
      // First what the condition is by default, then the one answer that makes it the other.
      defaultTitle: "By default", // reviewed
      defaultText: "What the condition is unless the answer matches the settings entered below", // reviewed
      fallback: "The condition is", // reviewed
      fallbackHelper: "Also when connection to the webhook is lost", // reviewed
      fallback_active: "Active", // reviewed
      fallback_inactive: "Not active", // reviewed
      // The match's heading: the state it gives, the other one than the default.
      matchInactive: "Not active when", // reviewed
      matchActive: "Active when", // reviewed
      matchText: "Set the unique response details that define this state", // reviewed
      testText: "Test the request once, and inspect its result", // reviewed
      matchOn: "Match on", // reviewed
      on: { status: "HTTP code", body: "Body" }, // reviewed
      statusWrong: "Enter an HTTP status code, e.g. 200", // reviewed
      statusPlaceholder: "200", // reviewed
      bodyPlaceholder: "e.g. on", // reviewed
      bodyHelper: "The body must be exactly the same as what is entered here", // reviewed
      bodyMissing: "Enter the body text", // reviewed
      test: "Test", // reviewed
      testing: "Asking …", // reviewed
      testNot: "Not asked", // reviewed
      testFailed: "No answer", // reviewed
      answer: "HTTP {status}. Body: “{body}”", // reviewed
      matchedActive: "Matched as active", // reviewed
      matchedInactive: "Matched as inactive", // reviewed
      unmatchedActive: "Fallback to active", // reviewed
      unmatchedInactive: "Fallback to inactive", // reviewed
    },
    typeSchedule: "Weekly schedule", // reviewed
    typeDrive: "Flash drive", // reviewed
    typeWebhook: "Webhook", // reviewed
    // A webhook condition tile: {url}.
    activeWebhook: "Based on requests to {url}", // reviewed
    typeCombined: "Combination", // reviewed
    // {names}: the conditions combined, as links.
    activeAny: "Active when any of {names} is true", // reviewed
    activeAll: "Active when all of {names} are true", // reviewed
    // A combination: which conditions, and whether any or all of them make it active. Never of other combinations.
    combine: {
      text: "Two or more other conditions, of which any or all must be active", // reviewed
      modeLabel: "Active when", // reviewed
      modeHelper: "Any needs one, all needs every one together", // reviewed
      any: "Any is active", // reviewed
      all: "All are active", // reviewed
      legend: "Conditions", // reviewed
      none: "Combining needs at least two other conditions", // reviewed
      missing: "Choose at least two", // reviewed
    },
    // {times}: the schedule in words; {drive}: the drive's name, quoted.
    active: "Active {times}", // reviewed
    activeDrive: "Active while {drive} is unplugged", // reviewed
    // Above the week grid.
    nothingMarked: "Mark when sessions are active", // reviewed
    // The heading of a weekly schedule's grid; its line is `gridHelp`.
    weekTitle: "Week", // reviewed
    gridHelp: "Drag over the week to mark active times, drag from a marked time to clear it", // reviewed
    // The week grid's column heads (the grid starts on Sunday).
    days: { mon: "Mon", tue: "Tue", wed: "Wed", thu: "Thu", fri: "Fri", sat: "Sat", sun: "Sun" }, // reviewed
    // A flash drive condition: the drive is the key. The connected drives are looked up when the
    // editor opens and on "Scan again".
    drive: {
      title: "Drive", // reviewed
      text: "The drive is the key: the condition is active while it is unplugged", // reviewed
      helper: "Active while unplugged. Drives without a serial number are not listed", // reviewed
      scanning: "Looking for drives", // reviewed
      none: "No flash drive found", // reviewed
      // A drive in the list: its name and serial number.
      named: "{name} ({serial})", // reviewed
      // {name}: the drive (`named`) the condition was set to that is not connected now.
      away: "{name} (unplugged)", // reviewed
      scan: "Scan again", // reviewed
      missing: "Choose a flash drive", // reviewed
    },
  },

  // A session's allowance, set on its tile (its settings in the modal).
  allowance: {
    // The kinds, in the session's Allowance row and in the allowance's form.
    type: {
      none: "No allowance", // reviewed
      budget: "Time budget", // reviewed
      pomodoro: "Pomodoro cycle", // reviewed
      delayed: "Delayed start", // reviewed
      breaks: "Breaks", // reviewed
    },
    // What a budget's reset every so long, or a Pomodoro cycle, is counted from.
    anchor: {
      start: "The start of the session", // reviewed
      clock: "Midnight", // reviewed
    },
    // A time budget.
    budget: {
      allowedLabel: "Time allowed", // reviewed
      allowedHelper: "Counts while a blocked item is active", // reviewed
      scopeLabel: "Applies to", // reviewed
      scopeHelper: "Each item individually means that every site, program or app gets its own budget", // reviewed
      total: "The whole session", // reviewed
      each: "Each item individually", // reviewed
      resetLabel: "Time budget is reset", // reviewed
      resetHelper: "What was used becomes available again", // reviewed
      never: "Never while the session runs", // reviewed
      every: "Interval based", // reviewed
      condition: "Each time the condition activates", // reviewed
      // Given back every so long.
      everyLabel: "Every", // reviewed
      everyHelper: "Counted from the starting point chosen below", // reviewed
      anchorLabel: "Counted from", // reviewed
      anchorHelper: "The starting point decides when the allowance resets", // reviewed
    },
    // Breaks, taken by hand from the session's tile; given back as a budget's time is.
    breaks: {
      countLabel: "Number of breaks", // reviewed
      countHelper: "Start them with the Break button on the session", // reviewed
      lengthLabel: "Each break lasts", // reviewed
      lengthHelper: "Nothing is enforced during a break", // reviewed
      resetLabel: "Breaks are given back", // reviewed
    },
    // A Pomodoro cycle: blocked first, then everything is allowed, over and over.
    cycle: {
      blockedLabel: "Blocked for", // reviewed
      allowedLabel: "Allowed for", // reviewed
      helper: "Everything the session closes is allowed during the pause", // reviewed
      anchorLabel: "Cycles count from", // reviewed
      anchorHelper: "The starting point decides how the timer is aligned", // reviewed
    },
    // Under the fields: the terms in words, from the backend.
    describing: "Checking", // reviewed
  },

  // Deleting a session, target or condition. {noun}: the kind's noun above.
  delete: {
    heading: "Delete {name}?", // reviewed
    body: "Deletes the {noun} for good", // reviewed
    // What it does to the sessions that use a target or condition; {names}: those sessions, quoted.
    losesTarget: "The target will be deleted from: {names}", // reviewed
    becomesAlways: "Always active after removal: {names}", // reviewed
    losesLock: "Unlocked after removal: {names}", // reviewed
    shrinksCombination: "Will be removed from: {names}", // reviewed
    removesCombination: "Orphaned combinations: {names}", // reviewed
    stopsIt: "It is running and will be stopped", // reviewed
    failed: "Could not delete the {noun}", // reviewed
  },

  editor: {
    saveFailed: "Could not save the {noun}", // reviewed
    lockedTitle: "Locked", // reviewed
    limitedTitle: "Only stricter changes while locked", // reviewed
  },

  settings: {
    title: "Settings", // reviewed
    description: "Unlocking, the unlock delay, and import and export settings", // reviewed
    unlockText: "This is the only way to end a locked session early. Request it, wait for the countdown, then unlock. Ends every running session, locked ones too. Sessions, targets and conditions stay", // reviewed
    // No delay set: the state of a clean install.
    unlockFree: "Ends every running session, locked ones too. Sessions, targets and conditions stay. Set an unlock delay below to add a countdown", // reviewed
    // {delay}: how long it takes; {left}: the time left, in words.
    unlockAsk: "Unlocking takes {delay}. Request it to start the countdown", // reviewed
    unlockWait: "Unlock ready in {left}. Press Keep locked to cancel it", // reviewed
    unlockReady: "Unlock ready. Unlock now or lock again", // reviewed
    unlockNothing: "Nothing is locked", // reviewed
    // The other way to unlock lets go: no countdown needed.
    unlockReadyWay: "Unlock ready by secondary unlock method", // reviewed
    // The other way to unlock, besides the delay. Its fields are the lock methods' own, worded for unlocking.
    way: {
      title: "Secondary unlock method", // reviewed
      text: "Besides the delay set above, a second method can be set. If either one is true, Bullseye can be unlocked", // reviewed
      heading: "{method} to unlock", // reviewed
      saved: "Saved", // reviewed
      removed: "Only the delay unlocks now", // reviewed
      failed: "Could not set the way to unlock", // reviewed
      noConditions: "Make a condition first to set the condition unlock method", // reviewed
      answerText: "Unlock with text", // reviewed
      answerPassword: "Unlock with password", // reviewed
      answerHeading: "Unlock?", // reviewed
      answerBody: "Enter the password to end every running session now", // reviewed
    },
    unlock: "Unlock", // reviewed
    request: "Request unlock", // reviewed
    aborted: "Everything stays locked", // reviewed
    confirmHeading: "End every running session?", // reviewed
    confirmBody: "Locked sessions end too. Sessions, targets and conditions stay", // reviewed
    failed: "Not unlocked", // reviewed
    unlocked: "Unlocked, every session ended", // reviewed
    delayTitle: "Unlock delay", // reviewed
    delayText: "Time between requesting and unlocking, up to 24 hours. A longer delay can always be set, but to shorten it, no locked sessions can be active", // reviewed
    delayLabel: "Delay (up to 24 hours)", // reviewed
    setDelay: "Set delay", // reviewed
    changeDelay: "Change delay", // reviewed
    // {delay}: the new delay in words.
    delaySet: "Unlocking now takes {delay}", // reviewed
    delayFree: "Unlocking is free", // reviewed
    delayFailed: "Delay not changed", // reviewed
    transferTitle: "Move to another computer", // reviewed
    transferText: "Export saves all targets, conditions and sessions to a file. Import replaces all of them with the file's contents. Password locks are stored as a hash", // reviewed
    export: "Export", // reviewed
    import: "Import", // reviewed
    // {file}: the file's name.
    exported: "Exported to {file}", // reviewed
    exportFailed: "Not exported", // reviewed
    importHeading: "Replace everything with this file?", // reviewed
    // {targets}, {conditions}, {sessions}: the counts below, as in the file.
    importBody: "{targets}, {conditions} and {sessions} replace your current ones. This cannot be undone",
    importTargets: { one: "{n} target", other: "{n} targets" }, // reviewed
    importConditions: { one: "{n} condition", other: "{n} conditions" }, // reviewed
    importSessions: { one: "{n} session", other: "{n} sessions" }, // reviewed
    // {names}: the sessions, quoted; {n}: how many.
    importUnlocked: {
      one: "{names} has a password lock without its hash in the file and imports without a lock", // reviewed
      other: "{names} have password locks without their hash in the file and import without a lock", // reviewed
    },
    // Store apps in the file that are not installed on this PC, which are left out: a Store app is one
    // exact package, and this PC may have the app as another one, or not at all. {apps}: each as
    // importAppIn, {app} its name and {target} its target, quoted.
    importAppsLeftOut: "Left out, not installed on this PC: {apps}", // reviewed
    importAppIn: "{app} in {target}", // reviewed
    importAppsUnknown: "Could not tell which Store apps are installed here, so all are kept", // reviewed
    // Said too when a session runs without being locked: it ends.
    importStops: "Running sessions without a lock are stopped", // reviewed
    imported: "Imported", // reviewed
    importFailed: "Not imported", // reviewed
  },
};
