// The stand-in for the Tauri backend (`window.__TAURI__`), put into the window before its scripts, after
// `mark.js`. It answers every command the page sends from a library of sample targets, conditions and
// sessions (`window.__STATE`), which a scene changes before the page starts (`window.__SETUP`, the
// scene's `setup`, as source) and may set to refuse a command (`fail`), never answer one (`pending`) or
// answer one its own way (`answers`).
//
// What the service writes in the window's words (a session's rows and tag, `status.rs`) is written here
// from the same marked texts, so those texts are found on screen like the page's own.

(() => {
  const t = (path, values = {}) => {
    let text = path.split(".").reduce((node, key) => (node == null ? node : node[key]), window.STRINGS);
    if (text && typeof text === "object" && "other" in text) text = Number(values.n) === 1 ? text.one : text.other;
    if (typeof text !== "string") return path;
    return text.replace(/\{(\w+)\}/g, (whole, key) => (key in values ? String(values[key]) : whole));
  };
  const quoted = (name) => t("common.quoted", { name });

  const week = (from, to, days = [0, 1, 2, 3, 4]) => [0, 1, 2, 3, 4, 5, 6].map((d) => (days.includes(d) ? [[from, to]] : []));
  const target = (id, name, lists = {}, actions = {}) => ({
    id,
    name,
    filters: [],
    allowed: [],
    programs: [],
    store_apps: [],
    titles: [],
    allowed_titles: [],
    ...lists,
    actions: { window_titles_minimize: false, obsidian_web_viewer: false, steam_browser: false, ...actions },
  });

  const S = {
    targets: [
      target(
        "social",
        "Social media",
        {
          filters: ["youtube.com", "reddit.com", "x.com", "instagram.com"],
          allowed: ["music.youtube.com", "old.reddit.com/r/rust"],
          titles: ["*YouTube*", "*Reddit*"],
          allowed_titles: ["*Visual Studio Code", "*Obsidian"],
        },
        { window_titles_minimize: true, obsidian_web_viewer: true, steam_browser: true },
      ),
      target("games", "Games", {
        filters: ["store.steampowered.com"],
        allowed: ["help.steampowered.com"],
        programs: ["EpicGamesLauncher.exe"],
        store_apps: ["Microsoft.MinecraftUWP_8wekyb3d8bbwe"],
        titles: ["*Steam*"],
        allowed_titles: ["Steam Settings"],
      }),
      target("news", "News", {
        filters: ["nu.nl", "nos.nl", "news.ycombinator.com"],
        programs: ["discord.exe", "slack.exe"],
        store_apps: ["SpotifyAB.SpotifyMusic_zpdnekdrzrea0", "5319275A.WhatsAppDesktop_cv1g1gvanyjgm"],
      }),
      target("spare", "Spare"),
    ],
    conditions: [
      { id: "hours", name: "Work hours", type: "schedule", days: week(510, 1050) },
      { id: "key", name: "USB key", type: "flash_drive", serial: "4C530001", drive: "SanDisk Cruzer" },
      {
        id: "home",
        name: "At home",
        type: "webhook",
        url: "https://home.example.com/api/presence?room=office",
        method: "get",
        interval: 300,
        fallback: true,
        matcher: { on: "status", status: 200, body: null },
      },
      { id: "out", name: "Out and about", type: "combined", mode: "any", members: ["key", "home"] },
      { id: "both", name: "Weekdays with key", type: "combined", mode: "all", members: ["hours", "key"] },
    ],
    sessions: [
      {
        id: "deep",
        name: "Deep work",
        targets: ["social", "games"],
        condition: "hours",
        allowance: { type: "budget", allowed: "30m", scope: "total", reset: "every", every: "1d", anchor: "clock" },
        lock: { method: "timed", kind: "for", value: "2h" },
        kept_locks: [],
      },
      {
        id: "reading",
        name: "Reading",
        targets: ["news"],
        condition: "home",
        allowance: { type: "breaks", count: 3, length: "10m", reset: "never" },
        lock: { method: "delay", delay: "15m" },
        kept_locks: [],
      },
      {
        id: "evening",
        name: "Evening",
        targets: ["social"],
        condition: null,
        allowance: { type: "pomodoro", blocked: "25m", allowed: "5m", anchor: "start" },
        lock: { method: "condition", condition: "key" },
        kept_locks: [],
      },
      { id: "weekend", name: "Weekend", targets: [], condition: "out", allowance: null, lock: { method: "none" }, kept_locks: [] },
    ],
    // Running sessions by id: what differs from a plain unlocked run (`run`).
    running: {
      deep: { locked: true, remaining: 4980, remaining_words: "83 minutes" },
      reading: { stop_delay: 900, stop_delay_words: "15 minutes", allowance: { terms: { type: "breaks" }, open: false, break_ready: true } },
    },
    // What a session's rows say, by id, where it is not what `stopped` works out: [key of
    // sessions.now, values], and its tag.
    status: {
      deep: { state: "locked", c: ["activeFor", { change: "4 hours" }], a: ["left", { left: "18 minutes" }], l: ["timeLeft", { left: "83 minutes" }] },
      reading: { state: "running", c: ["activeByWebhook"], a: ["breaksLeft", { n: 2 }], l: ["delay", { delay: "15 minutes" }] },
    },
    unlock: { delay: 0, delay_words: "0 seconds", remaining: null, remaining_words: null, way: null, way_open: false, way_left: null, way_left_words: null },
    fail: {},
    pending: [],
    answers: {},
  };
  window.__STATE = S;
  window.__week = week;
  window.__target = target;

  // ---- The service's words.

  const RULE = { schedule: "Schedule", flash_drive: "Device", webhook: "Webhook", combined: "Combination" };
  const UNITS = { d: ["day", "days"], h: ["hour", "hours"], m: ["minute", "minutes"], s: ["second", "seconds"] };
  // `90m` as the service writes a length: `90 minutes`, `2 hours and 30 minutes`.
  const words = (text) => {
    const parts = [...String(text || "").matchAll(/(\d+)\s*([dhms])/g)].map(([, n, u]) => `${n} ${UNITS[u][Number(n) === 1 ? 0 : 1]}`);
    return parts.length > 1 ? `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}` : parts[0] || "";
  };
  const shortDate = (text) => {
    const [day, time] = String(text || "").split(" ");
    const date = new Date(`${day}T${time || "00:00"}`);
    if (Number.isNaN(date.getTime())) return text;
    return new Date().toDateString() === date.toDateString() ? time : date.toLocaleDateString("en-US", { month: "long", day: "numeric" });
  };
  const condition = (id) => S.conditions.find((c) => c.id === id);
  const line = ([key, values = {}] = [], named = null) => ({ text: t(`sessions.now.${key}`, values), ...(named ? { condition: named } : {}) });

  // A stopped session's rows, as `status::stopped_lines` writes them.
  function stopped(session) {
    const c = condition(session.condition);
    const a = session.allowance;
    const l = session.lock;
    const conditionLine = c ? line([`by${RULE[c.type]}`]) : line(["always"]);
    let allowanceLine = line(["neverPaused"]);
    if (a && a.type === "pomodoro") allowanceLine = line(["cycle", { blocked: words(a.blocked), allowed: words(a.allowed) }]);
    if (a && a.type === "delayed") allowanceLine = a.kind === "from" ? line(["from", { date: shortDate(a.value) }]) : line(["after", { time: words(a.value) }]);
    if (a && a.type === "breaks") allowanceLine = line(["breaks", { n: a.count, time: words(a.length) }]);
    if (a && a.type === "budget") allowanceLine = line([a.scope === "each" ? "perItem" : "budget", { time: words(a.allowed) }]);
    let lockLine = line(["notLocked"]);
    if (l.method === "timed") lockLine = l.kind === "until" ? line(["until", { date: shortDate(l.value) }]) : line(["for", { time: words(l.value) }]);
    if (l.method === "until_restart") lockLine = line(["restart"]);
    if (l.method === "delay") lockLine = line(["delay", { delay: words(l.delay) }]);
    if (l.method === "text" || l.method === "random_text") lockLine = line(["text"]);
    if (l.method === "password") lockLine = line(["password"]);
    if (l.method === "condition") {
      const lc = condition(l.condition);
      lockLine = lc ? line([lc.type === "schedule" ? "during" : "by"], { id: lc.id, name: lc.name }) : { text: t("locks.condition.label") };
    }
    return { state: "not_running", condition: conditionLine, allowance: allowanceLine, lock: lockLine };
  }

  function rows(session) {
    const set = S.status[session.id];
    if (!S.running[session.id] || !set) return { id: session.id, ...stopped(session), ...(S.running[session.id] ? { state: "running" } : {}) };
    const named = set.lc ? { id: set.lc, name: (condition(set.lc) || {}).name || set.lc } : null;
    return { id: session.id, state: set.state, condition: line(set.c), allowance: line(set.a), lock: line(set.l, named) };
  }

  function rule(id) {
    const c = condition(id);
    if (!c) return null;
    const { id: _, name, ...rest } = c;
    return rest;
  }

  function run(session) {
    const extra = S.running[session.id];
    return {
      id: session.id,
      name: session.name,
      targets: [],
      remaining: null,
      lock_condition: null,
      until_restart: false,
      lock_text: null,
      stop_delay: null,
      stop_delay_words: null,
      stop_remaining: null,
      stop_remaining_words: null,
      password: false,
      locked: false,
      condition: rule(session.condition),
      enforcing: true,
      condition_change: null,
      lock_change: null,
      remaining_words: null,
      condition_change_words: null,
      lock_change_words: null,
      allowance: null,
      ...extra,
    };
  }

  function overview() {
    return structuredClone({
      targets: S.targets,
      conditions: S.conditions,
      sessions: S.sessions,
      running: S.sessions.filter((s) => S.running[s.id]).map(run),
      unlock: S.unlock,
      clock_synced: true,
      status: S.sessions.map(rows),
    });
  }

  // `Mon-Fri 08:30-17:30`, as `schedule::Week::describe` writes it.
  const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const hm = (m) => `${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
  function describeWeek(days) {
    const spans = days.map((d) => d.map(([a, b]) => `${hm(a)}-${hm(b)}`).join(", "));
    const out = [];
    for (let i = 0; i < 7; i++) {
      if (!spans[i]) continue;
      let j = i;
      while (j + 1 < 7 && spans[j + 1] === spans[i]) j++;
      out.push(`${j > i ? `${DAYS[i]}-${DAYS[j]}` : DAYS[i]} ${spans[i]}`);
      i = j;
    }
    return out.join("; ");
  }

  function describeAllowance(terms) {
    if (terms.type === "budget") return `${words(terms.allowed)} ${terms.scope === "each" ? "for each item" : "in all"}, given back ${terms.reset === "every" ? `every ${words(terms.every)}` : terms.reset === "condition" ? "each time the condition turns on" : "never"}`;
    if (terms.type === "pomodoro") return `Blocked for ${words(terms.blocked)}, then allowed for ${words(terms.allowed)}, counted from ${terms.anchor === "clock" ? "midnight" : "the start"}`;
    if (terms.type === "delayed") return terms.kind === "from" ? `Nothing is enforced until ${terms.value}` : `Nothing is enforced for the first ${words(terms.value)}`;
    if (terms.type === "breaks") return `${terms.count} breaks of ${words(terms.length)}, given back ${terms.reset === "never" ? "never" : `every ${words(terms.every)}`}`;
    return "";
  }

  const id = (x) => x.id;
  const put = (list, item) => {
    const at = list.findIndex((x) => x.id === item.id);
    if (at >= 0) list[at] = item;
    else list.push(item);
  };
  const kinds = { session: "sessions", target: "targets", condition: "conditions" };

  const COMMANDS = {
    status: () => overview(),
    window_ready: () => null,
    describe_schedule: ({ week }) => describeWeek(week.days),
    describe_duration: ({ text }) => {
      if (!/\d+\s*[dhms]/.test(text || "")) throw "Enter a length, such as 90m or 2h";
      return words(text);
    },
    describe_until: ({ text }) => {
      if (!/^\d{4}-\d\d-\d\d \d\d:\d\d$/.test(text || "")) throw "Enter a date and a time";
      return { when: "Tuesday 13 October 2026, 18:00", left: "4 days" };
    },
    describe_allowance: ({ terms }) => ({ words: describeAllowance(terms) }),
    deletion_fallout: () => ({ without_it: [], always: [], unlocked: [], shrinks: [], removed: [] }),
    installed_apps: () => [
      { family: "Microsoft.MinecraftUWP_8wekyb3d8bbwe", name: "Minecraft", installed: false },
      { family: "SpotifyAB.SpotifyMusic_zpdnekdrzrea0", name: "Spotify", installed: true },
      { family: "5319275A.WhatsAppDesktop_cv1g1gvanyjgm", name: "WhatsApp", installed: true },
      { family: "Microsoft.ZuneVideo_8wekyb3d8bbwe", name: "Films & TV", installed: true },
    ],
    connected_drives: () => [
      { serial: "4C530001", name: "SanDisk Cruzer" },
      { serial: "AA1100BB", name: "Kingston DataTraveler" },
    ],
    test_webhook: () => ({ status: 200, body: "on", outcome: "matches", active: false }),
    save_session: ({ session }) => put(S.sessions, { kept_locks: [], ...session }),
    save_target: ({ target }) => put(S.targets, target),
    save_condition: ({ condition }) => put(S.conditions, condition),
    delete_session: ({ id: gone }) => (S.sessions = S.sessions.filter((s) => s.id !== gone)),
    delete_target: ({ id: gone }) => (S.targets = S.targets.filter((s) => s.id !== gone)),
    delete_condition: ({ id: gone }) => (S.conditions = S.conditions.filter((s) => s.id !== gone)),
    duplicate: ({ kind, id: from, newId }) => {
      const list = S[kinds[kind]];
      const original = list.find((x) => x.id === from);
      list.push({ ...structuredClone(original), id: newId, name: `${original.name} (2)` });
    },
    start_session: ({ id: started }) => (S.running[started] = {}),
    stop_session: ({ id: stopped }) => delete S.running[stopped],
    request_stop: () => null,
    abort_stop: () => null,
    take_break: () => null,
    end_break: () => null,
    lock_session: () => null,
    check_stricter: () => null,
    unlock_all: () => (S.running = {}),
    unlock_with: () => (S.running = {}),
    request_unlock: () => null,
    abort_unlock: () => null,
    set_unlock_delay: ({ delay }) => {
      const seconds = [...String(delay).matchAll(/(\d+)([dhm])/g)].reduce((sum, [, n, u]) => sum + Number(n) * { d: 86400, h: 3600, m: 60 }[u], 0);
      S.unlock = { ...S.unlock, delay: seconds, delay_words: words(delay) || "0 seconds" };
    },
    set_unlock_way: ({ way }) => (S.unlock = { ...S.unlock, way }),
    export_settings: () => "C:\\Users\\you\\Documents\\bullseye-settings.json",
    open_settings: () => ({
      path: "C:\\Users\\you\\Documents\\bullseye-settings.json",
      contents: { targets: 4, conditions: 5, sessions: 4, unlocked: [] },
      left_out: [],
      apps_checked: true,
    }),
    import_settings: () => null,
  };

  // The scene's changes, before the page asks anything.
  if (window.__SETUP) new Function("S", "t", "quoted", window.__SETUP)(S, t, quoted);

  async function invoke(command, args = {}) {
    await new Promise((resolve) => setTimeout(resolve, 5));
    if (S.pending.includes(command)) return new Promise(() => {});
    if (command in S.fail) throw S.fail[command];
    if (command in S.answers) {
      const answer = S.answers[command];
      if (answer && answer.reject) throw answer.reject;
      return structuredClone(answer);
    }
    const handler = COMMANDS[command];
    if (!handler) throw `The stand-in has no command ${command}`;
    const result = handler(args);
    return result === undefined ? null : structuredClone(result);
  }

  window.__TAURI__ = { core: { invoke }, event: { listen: async () => () => {} } };
})();
