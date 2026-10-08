// What targets are made of (`LISTS`, `SECTIONS`) and the editors' forms: a session, a target,
// a condition (with its week grid or its flash drive). Every form has `nodes`, `fill(item)`, `read(item)`,
// `disable(off)`, `settle()` and `error()`.

import { el, invalid, invoke, lines, overview, t } from "./core.js";
import { durationField } from "./duration.js";
import { timingFields } from "./timing.js";

// ---- What things are made of. Three kinds, each a list of its own: targets (what to
// session), conditions (when) and sessions (one or more targets, a condition, an allowance and a
// lock: what is started; the allowance and the lock are set on the session itself). The editors, tiles and summaries are built from these tables, so a new kind of
// list, action or lock method is an entry here (plus its backend side, see AGENTS.md).

// The lists a target can have (`definitions::Target`), by field name.
const LISTS = Object.fromEntries(
  ["filters", "allowed", "programs", "store_apps", "titles", "allowed_titles"].map((key) => [
    key,
    {
      label: t(`targets.${key}.label`),
      placeholder: t(`targets.${key}.placeholder`),
      helper: t(`targets.${key}.helper`),
      count: (n) => t(`targets.${key}.count`, { n }),
      // The Store apps are ticked from the installed ones, not typed (`appPicker`).
      picker: key === "store_apps",
    },
  ]),
);

// The actions (`state::Actions`), grouped into the target editor's sections with the
// lists they use. Programs, Store apps and windows by title have none to switch on: a list acts
// when it has entries.
const action = (key) => ({ key, label: t(`targets.actions.${key}.label`), short: window.STRINGS.targets.actions[key].short });
const SECTIONS = [
  { key: "websites", lists: ["filters", "allowed"], actions: [action("chromium_tabs"), action("chromium_split_view"), action("chromium_app_windows"), action("firefox_tabs"), action("firefox_split_view")] },
  { key: "programs", lists: ["programs"], actions: [] },
  { key: "store", lists: ["store_apps"], actions: [] },
  // Closing (`choice`, the default: off) or minimizing: radio buttons, not a checkbox.
  { key: "titles", lists: ["titles", "allowed_titles"], choice: "close", actions: [action("window_titles_minimize")] },
  { key: "other", lists: [], actions: [action("obsidian_web_viewer")] },
].map((section) => ({ ...section, title: t(`targets.sections.${section.key}.title`), description: window.STRINGS.targets.sections[section.key].description && t(`targets.sections.${section.key}.description`) }));

const ACTIONS = SECTIONS.flatMap((section) => section.actions);

// ---- The editors. Each kind's form is built once and reused; every form has `nodes`,
// `fill(item)`, `read(item)` (the item with the form's values), `disable(off)`, `settle()`
// (finish what is being typed) and `error()` (what is wrong, or null).

function nameField(placeholder) {
  const input = el("cds-text-input", { label: t("common.name"), placeholder, "max-count": "60" });
  input.addEventListener("input", () => (input.invalid = false));
  return input;
}

// The name's place in a form: a text box. A new item has a name already, the first free "Target 3"
// (`freeName`); `more` goes under it in the same section.
function nameSection(input, ...more) {
  return el("section", {}, input, ...more);
}

// A part of an editor below its name: a heading and a line on what it is for, as each part of the
// Settings page has, with space between parts and no line (only the buttons have one above them).
const section = (title, text, ...content) =>
  el("section", {}, el("h2", { class: "cds--type-productive-heading-03" }, title), el("p", { class: "cds--type-body-01" }, text), ...content);

// Text areas side by side (the sites to block and the allowed ones) keep one height: dragging
// one's resize handle sizes the others in its row too. Carbon's textarea has no attribute or
// part for its height, so this sets the same inline height on the others' own <textarea> that
// the browser's resize handle set on the dragged one; nothing else of the component is touched.
// Stacked (a narrow window), each keeps its own height.
function sameHeight(areas) {
  if (areas.length < 2) return;
  Promise.all(areas.map((area) => customElements.whenDefined("cds-textarea").then(() => area.updateComplete))).then(() => {
    const inner = areas.map((area) => area.shadowRoot && area.shadowRoot.querySelector("textarea")).filter(Boolean);
    let syncing = false;
    const observer = new ResizeObserver((entries) => {
      if (syncing) return;
      const moved = entries.find((entry) => entry.target.style.height);
      if (!moved) return;
      const row = moved.target.getBoundingClientRect().top;
      syncing = true;
      for (const other of inner) {
        if (other !== moved.target && Math.abs(other.getBoundingClientRect().top - row) < 1 && other.style.height !== moved.target.style.height) {
          other.style.height = moved.target.style.height;
        }
      }
      // The others' resize comes back as an entry of its own: let it pass before listening again.
      requestAnimationFrame(() => (syncing = false));
    });
    for (const textarea of inner) observer.observe(textarea);
  });
}

// The Store apps of a target: the installed ones (`installed_apps`, which takes a second or
// two) to tick, with the ones on the list that are not installed here (not any more, or imported
// from another computer), named by the backend from their family name, so saving keeps them. It
// stands in for a text area: `value` is the ticked family names, one per line, and `disabled` is
// settable. The apps are asked for once the list's own are known (its first `value`).
function appPicker(list) {
  // The label is above the box, which has a background of its own; the group is named for readers.
  const group = el("cds-checkbox-group", { "aria-label": list.label });
  const label = el("p", { class: "cds--type-label-01" }, list.label);
  const status = el("p", { class: "cds--type-helper-text-01" }, t("targets.store_apps.loading"));
  const scroll = el("div", { class: "apps" }, group);
  const picker = el("div", { class: "app-picker" }, label, scroll, status);
  let chosen = new Set();
  let installed = [];
  let disabled = false;
  const same = (a, b) => a.toLowerCase() === b.toLowerCase();
  function draw() {
    const known = new Set(installed.map((app) => app.family.toLowerCase()));
    const name = (app) => (app.installed ? app.name : t("targets.store_apps.notInstalled", { name: app.name }));
    const rows = [...installed.map((app) => [app.family, name(app)]), ...[...chosen].filter((family) => !known.has(family.toLowerCase())).map((family) => [family, family])];
    group.replaceChildren(
      ...rows.map(([family, label]) => {
        const box = el("cds-checkbox", { "label-text": label });
        box.checked = [...chosen].some((entry) => same(entry, family));
        box.disabled = disabled;
        box.addEventListener("cds-checkbox-changed", () => {
          for (const entry of [...chosen]) if (same(entry, family)) chosen.delete(entry);
          if (box.checked) chosen.add(family);
        });
        return box;
      }),
    );
    scroll.hidden = !rows.length;
  }
  Object.defineProperties(picker, {
    value: {
      get: () => [...chosen].join("\n"),
      set: (text) => {
        chosen = new Set(lines(text));
        draw();
        load();
      },
    },
    disabled: {
      get: () => disabled,
      set: (off) => {
        disabled = off;
        for (const box of group.children) box.disabled = off;
      },
    },
  });
  let asked = false;
  function load() {
    if (asked) return;
    asked = true;
    invoke("installed_apps", { chosen: [...chosen] }).then(
      (apps) => {
        installed = apps;
        status.textContent = list.helper;
        draw();
      },
      (e) => {
        status.textContent = `${t("targets.store_apps.failed")}: ${e}`;
        draw();
      },
    );
  }
  return picker;
}

// A section whose actions exclude each other: radio buttons, the first for all of them off
// (`section.choice`, its label in `targets.actions`). Each action gets a stand-in for its
// checkbox in `checks` (`checked`, `disabled`), so the form reads and fills them as the others.
function choiceOf(section, legend, checks) {
  const group = el("cds-radio-button-group", { "legend-text": legend, name: `actions-${section.key}`, orientation: "vertical" });
  group.append(
    el("cds-radio-button", { "label-text": t(`targets.actions.${section.choice}`), value: "off" }),
    ...section.actions.map((action) => el("cds-radio-button", { "label-text": action.label, value: action.key })),
  );
  group.value = "off";
  for (const action of section.actions) {
    checks[action.key] = {
      get checked() {
        return group.value === action.key;
      },
      set checked(on) {
        if (on) group.value = action.key;
        else if (group.value === action.key) group.value = "off";
      },
      set disabled(off) {
        group.disabled = off;
      },
    };
  }
  return group;
}

function targetForm() {
  const name = nameField(t("targets.namePlaceholder"));
  const areas = {};
  const checks = {};
  // The sections are an accordion: each opens and closes, and the ones with content are open when
  // the editor opens (`fill`): a section with a list when it has entries (the websites also in a new
  // target), one with only actions ("other") when one is ticked, which a new target has.
  const accordion = el("cds-accordion", {});
  const items = [];
  const nodes = [nameSection(name), accordion];
  for (const section of SECTIONS) {
    const lists = el("div", { class: "lists" });
    for (const key of section.lists) {
      const list = LISTS[key];
      areas[key] = list.picker ? appPicker(list) : el("cds-textarea", { label: list.label, placeholder: list.placeholder, "helper-text": list.helper, rows: 5 });
      lists.append(areas[key]);
    }
    sameHeight(section.lists.map((key) => areas[key]));
    const legend = t(section.lists.length ? "targets.howLegend" : "targets.actionsLegend");
    const group = section.choice ? choiceOf(section, legend, checks) : section.actions.length ? el("cds-checkbox-group", { "legend-text": legend }) : null;
    if (!section.choice && group) {
      for (const action of section.actions) {
        checks[action.key] = el("cds-checkbox", { "label-text": action.label });
        group.append(checks[action.key]);
      }
    }
    const item = el(
      "cds-accordion-item",
      {},
      el("span", { slot: "title" }, section.title),
      section.description ? el("p", { class: "cds--type-body-01" }, section.description) : null,
      lists,
      group,
    );
    items.push([section, item]);
    accordion.append(item);
  }
  return {
    nodes,
    fill(list, isNew = false) {
      name.value = list.name;
      for (const [key, area] of Object.entries(areas)) area.value = (list[key] || []).join("\n");
      for (const [key, box] of Object.entries(checks)) box.checked = !!(list.actions && list.actions[key]);
      for (const [section, item] of items) {
        const entries = section.lists.some((key) => (list[key] || []).length > 0);
        const ticked = section.actions.some((e) => list.actions && list.actions[e.key]);
        // A new target has the websites open too, to start typing in.
        item.open = section.lists.length ? entries || (isNew && section.key === "websites") : ticked;
      }
    },
    read(list) {
      const out = { ...list, name: name.value.trim(), actions: { ...list.actions } };
      for (const [key, area] of Object.entries(areas)) out[key] = lines(area.value);
      for (const [key, box] of Object.entries(checks)) out.actions[key] = box.checked;
      return out;
    },
    disable(off) {
      for (const control of [...Object.values(areas), ...Object.values(checks)]) control.disabled = off;
    },
    // The example in an empty list (google.com under Always allowed) is only there for a list
    // that can be filled in freely: elsewhere it reads as an entry.
    examples(show) {
      for (const [key, area] of Object.entries(areas)) if (!LISTS[key].picker) area.placeholder = show ? LISTS[key].placeholder : "";
    },
    settle: async () => {},
    error: () => (name.value.trim() ? null : t("targets.nameMissing")),
    name,
  };
}

// A flash drive to choose among the connected ones: looked up when the form opens and on "Scan
// again" (`connected_drives`, which takes a moment). The drive a condition was set to stays in the
// list while it is away, so saving does not lose it. Drives are told apart by their hardware's
// serial number (`drives.rs`).
function drivePicker() {
  const select = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.drive.field"), "helper-text": t("conditions.drive.helper") });
  const rescan = el("cds-button", { kind: "tertiary", size: "md" }, t("conditions.drive.scan"), carbonIcon("renew", "icon"));
  let known = new Map();
  let had = {};
  let scanning = false;
  let off = false;
  const item = (value, label) => el("cds-dropdown-item", { value }, label);
  async function scan() {
    const keep = select.value || had.serial || "";
    scanning = true;
    rescan.disabled = true;
    invalid(select, null);
    // Nothing to choose while it looks: the dropdown says so in place of a choice.
    select.replaceChildren();
    select.setAttribute("label", t("conditions.drive.scanning"));
    select.value = "";
    let drives = [];
    let fault = null;
    try {
      drives = await invoke("connected_drives");
    } catch (e) {
      fault = String(e);
    }
    known = new Map(drives.map((drive) => [drive.serial, drive]));
    const away = had.serial && !known.has(had.serial) ? [{ serial: had.serial, name: had.name || had.serial }] : [];
    for (const drive of away) known.set(drive.serial, drive);
    select.setAttribute("label", t(drives.length || away.length ? "conditions.drive.choose" : "conditions.drive.none"));
    select.replaceChildren(
      ...drives.map((drive) => item(drive.serial, `${drive.name} (${drive.serial})`)),
      ...away.map((drive) => item(drive.serial, t("conditions.drive.away", { name: `${drive.name} (${drive.serial})` }))),
    );
    select.value = known.has(keep) ? keep : "";
    if (fault) invalid(select, fault);
    scanning = false;
    rescan.disabled = off;
  }
  rescan.addEventListener("click", scan);
  return {
    nodes: [select, el("div", { class: "inline-action" }, rescan)],
    fill(condition) {
      had = condition.type === "flash_drive" ? { serial: condition.serial, name: condition.drive } : {};
      scan();
    },
    read: () => {
      const drive = known.get(select.value);
      return { serial: drive.serial, drive: drive.name };
    },
    error: () => (select.value && known.has(select.value) ? null : t("conditions.drive.missing")),
    disable(value) {
      off = value;
      select.disabled = off;
      rescan.disabled = off || scanning;
    },
  };
}

// A webhook's match: what turns the condition from its fallback to the other state, by a dropdown
// (the HTTP code or the body) and its field. `title` names the state it gives ("Not active when").
function matcherFields() {
  const mode = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.matchOn") });
  for (const key of ["status", "body"]) mode.append(el("cds-dropdown-item", { value: key }, t(`conditions.webhook.on.${key}`)));
  const status = el("cds-text-input", { label: t("conditions.webhook.status"), placeholder: "200", "max-count": "3" });
  const body = el("cds-textarea", { label: t("conditions.webhook.body"), placeholder: t("conditions.webhook.bodyPlaceholder"), "helper-text": t("conditions.webhook.bodyHelper"), rows: 3 });
  const show = () => {
    status.hidden = mode.value !== "status";
    body.hidden = mode.value !== "body";
  };
  mode.addEventListener("cds-dropdown-selected", show);
  return {
    nodes: [mode, status, body],
    fill(matcher) {
      mode.value = (matcher && matcher.on) || "status";
      status.value = matcher && matcher.status != null ? String(matcher.status) : "";
      body.value = (matcher && matcher.body) || "";
      show();
    },
    read: () => ({
      on: mode.value,
      status: mode.value === "status" && status.value.trim() ? Number(status.value.trim()) : null,
      body: mode.value === "body" ? body.value : null,
    }),
    error() {
      invalid(status, null);
      invalid(body, null);
      if (mode.value === "status" && !/^[1-5]\d\d$/.test(status.value.trim())) {
        invalid(status, t("conditions.webhook.statusWrong"));
        return t("conditions.webhook.statusWrong");
      }
      if (mode.value === "body" && !body.value.trim()) {
        invalid(body, t("conditions.webhook.bodyMissing"));
        return t("conditions.webhook.bodyMissing");
      }
      return null;
    },
    disable(off) {
      for (const control of [mode, status, body]) control.disabled = off;
    },
  };
}

// A webhook: one web address asked every so often (`webhook.rs`), GET or POST, and what its answer means:
// first what the condition is by default (also with no answer), then the one match that makes it the
// other. A Test button asks it once (`test_webhook`).
function webhookPicker() {
  const url = el("cds-text-input", { label: t("conditions.webhook.url"), placeholder: "https://example.com/state?room=1", "helper-text": t("conditions.webhook.urlHelper") });
  const method = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.method") });
  for (const key of ["get", "post"]) method.append(el("cds-dropdown-item", { value: key }, key.toUpperCase()));
  // No seconds: it is asked at most once a minute.
  const every = durationField({ label: t("conditions.webhook.every"), helper: t("conditions.webhook.everyHelper"), units: ["d", "h", "m"] });
  const fallback = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.fallback"), "helper-text": t("conditions.webhook.fallbackHelper") });
  for (const key of ["active", "inactive"]) fallback.append(el("cds-dropdown-item", { value: key }, t(`conditions.webhook.fallback_${key}`)));
  const matcher = matcherFields();
  // The match's section is named after the state it gives: the other one than the fallback.
  const matchPart = section("", t("conditions.webhook.matchText"), ...matcher.nodes);
  const nameMatch = () => {
    matchPart.firstChild.textContent = t(fallback.value === "inactive" ? "conditions.webhook.matchActive" : "conditions.webhook.matchInactive");
  };
  fallback.addEventListener("cds-dropdown-selected", nameMatch);
  // What a test found, as a Carbon notification: how the condition comes out, and what was answered.
  const result = el("div", { class: "webhook-result" });
  const show = (kind, title, subtitle = "") => result.replaceChildren(el("cds-inline-notification", { kind, title, subtitle, "low-contrast": true, "hide-close-button": true }));
  const test = el("cds-button", { kind: "tertiary", size: "md" }, t("conditions.webhook.test"), carbonIcon("play", "icon"));
  const read = () => ({
    url: url.value.trim(),
    method: method.value || "get",
    interval: every.seconds(),
    fallback: fallback.value !== "inactive",
    matcher: matcher.read(),
  });
  const error = () => {
    invalid(url, null);
    every.error(null);
    if (!/^https?:\/\/\S{3,}$/.test(url.value.trim())) {
      invalid(url, t("conditions.webhook.urlWrong"));
      return t("conditions.webhook.urlWrong");
    }
    if (every.seconds() < 60) {
      every.error(t("conditions.webhook.everyWrong"));
      return t("conditions.webhook.everyWrong");
    }
    return matcher.error();
  };
  test.addEventListener("click", async () => {
    const wrong = error();
    if (wrong) return show("error", t("conditions.webhook.testNot"), wrong);
    test.disabled = true;
    show("info", t("conditions.webhook.testing"));
    try {
      const found = await invoke("test_webhook", { hook: read() });
      const answer = t("conditions.webhook.answer", { status: found.status, body: found.body });
      // Green: active. Yellow: not active. The title says whether the answer matched.
      const key = `${found.outcome === "matches" ? "matched" : "unmatched"}${found.active ? "Active" : "Inactive"}`;
      show(found.active ? "success" : "warning", t(`conditions.webhook.${key}`), answer);
    } catch (e) {
      show("error", t("conditions.webhook.testFailed"), String(e));
    }
    test.disabled = false;
  });
  return {
    nodes: [
      section(t("conditions.webhook.requestTitle"), t("conditions.webhook.requestText"), url, method, every.node),
      section(t("conditions.webhook.defaultTitle"), t("conditions.webhook.defaultText"), fallback),
      matchPart,
      section(t("conditions.webhook.testTitle"), t("conditions.webhook.testText"), el("div", { class: "inline-action" }, test), result),
    ],
    fill(condition) {
      const hook = condition.type === "webhook" ? condition : {};
      url.value = hook.url || "";
      method.value = hook.method || "get";
      every.set(hook.interval || 60);
      fallback.value = hook.fallback === false ? "inactive" : "active";
      matcher.fill(hook.matcher);
      nameMatch();
      result.replaceChildren();
    },
    read,
    error,
    disable(off) {
      for (const control of [url, method, fallback, test]) control.disabled = off;
      every.disable(off);
      matcher.disable(off);
    },
  };
}

// A combination: the conditions it is of, ticked among the others (never a combination, nor
// itself: no nesting), and whether any one of them or all of them make it active.
function combinePicker() {
  const mode = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.combine.modeLabel"), "helper-text": t("conditions.combine.modeHelper") });
  for (const key of ["any", "all"]) mode.append(el("cds-dropdown-item", { value: key }, t(`conditions.combine.${key}`)));
  const group = el("cds-checkbox-group", { "legend-text": t("conditions.combine.legend") });
  const none = el("p", { class: "cds--type-body-01" }, t("conditions.combine.none"));
  let boxes = [];
  let off = false;
  return {
    nodes: [mode, group, none],
    fill(condition) {
      const others = overview.conditions.filter((c) => c.type !== "combined" && c.id !== condition.id);
      const had = condition.type === "combined" ? condition.members : [];
      boxes = others.map((c) => {
        const box = el("cds-checkbox", { "label-text": c.name });
        box.checked = had.includes(c.id);
        box.disabled = off;
        box.dataset.id = c.id;
        return box;
      });
      group.replaceChildren(...boxes);
      none.hidden = boxes.length >= 2;
      mode.value = condition.type === "combined" ? condition.mode : "any";
    },
    read: () => ({ mode: mode.value, members: boxes.filter((box) => box.checked).map((box) => box.dataset.id) }),
    error: () => (boxes.filter((box) => box.checked).length >= 2 ? null : t("conditions.combine.missing")),
    disable(value) {
      off = value;
      mode.disabled = value;
      for (const box of boxes) box.disabled = value;
    },
  };
}

// A condition: a name, its type, and what the type needs: a week to drag over (`weekEditor`), a
// flash drive (`drivePicker`), a webhook (`webhookPicker`) or other conditions to combine (`combinePicker`). What is not of the
// chosen type is not kept.
function conditionForm() {
  const name = nameField(t("conditions.namePlaceholder"));
  const type = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.typeLabel") });
  type.append(
    el("cds-dropdown-item", { value: "schedule" }, t("conditions.typeSchedule")),
    el("cds-dropdown-item", { value: "flash_drive" }, t("conditions.typeDrive")),
    el("cds-dropdown-item", { value: "webhook" }, t("conditions.typeWebhook")),
    el("cds-dropdown-item", { value: "combined" }, t("conditions.typeCombined")),
  );
  const hook = webhookPicker();
  // Its own sections, with their headings, so a div here (a section in a section would be drawn as a divided one).
  const hookPart = el("div", { class: "form" }, ...hook.nodes);
  const week = weekEditor();
  const weekPart = section(t("conditions.weekTitle"), t("conditions.gridHelp"), ...week.nodes);
  const picker = drivePicker();
  const drivePart = section(t("conditions.drive.title"), t("conditions.drive.text"), ...picker.nodes);
  const combine = combinePicker();
  const combinePart = section(t("conditions.combine.title"), t("conditions.combine.text"), ...combine.nodes);
  const show = () => {
    weekPart.hidden = type.value !== "schedule";
    drivePart.hidden = type.value !== "flash_drive";
    hookPart.hidden = type.value !== "webhook";
    combinePart.hidden = type.value !== "combined";
  };
  type.addEventListener("cds-dropdown-selected", show);
  return {
    nodes: [nameSection(name, type), weekPart, drivePart, hookPart, combinePart],
    fill(condition, isNew = false) {
      name.value = condition.name;
      type.value = condition.type || "schedule";
      week.fill({ days: condition.days || [0, 1, 2, 3, 4, 5, 6].map(() => []) });
      picker.fill(condition);
      hook.fill(condition);
      combine.fill(condition);
      show();
    },
    read: (condition) => {
      const base = { id: condition.id, name: name.value.trim() };
      if (type.value === "flash_drive") return { ...base, type: "flash_drive", ...picker.read() };
      if (type.value === "webhook") return { ...base, type: "webhook", ...hook.read() };
      if (type.value === "combined") return { ...base, type: "combined", ...combine.read() };
      return { ...base, type: "schedule", days: week.read().days };
    },
    disable(off) {
      type.disabled = off;
      week.disable(off);
      picker.disable(off);
      hook.disable(off);
      combine.disable(off);
    },
    settle: () => week.settle(),
    error: () => {
      if (!name.value.trim()) return t("conditions.nameMissing");
      // A valid one answers null, which must not fall through to another type's check.
      const part = { flash_drive: picker, webhook: hook, combined: combine, schedule: week }[type.value] ?? week;
      return part.error();
    },
    name,
  };
}

// A session's allowance, a section of the session form: its type (none, a time budget or a Pomodoro
// cycle) and what the type needs: how much time, for the session or each item, and when it is given
// back; or blocked for, allowed for, counted from where. Under it the terms in words, written by the
// backend (`describe_allowance`), which is also what says what cannot be saved. `fill(terms or null)`,
// `read()` (the terms, or null for none), `settle()` and `error()`.
function allowanceSection() {
  const dropdown = (label, helper, keys, path) => {
    const field = el("cds-dropdown", { autoalign: true, "title-text": label, "helper-text": helper });
    for (const key of keys) field.append(el("cds-dropdown-item", { value: key }, t(`${path}.${key}`)));
    return field;
  };
  const type = dropdown(t("allowance.typeLabel"), t("allowance.typeHelper"), ["none", "budget", "pomodoro", "delayed"], "allowance.type");
  const text = (label, helper) => durationField({ label, helper });
  const allowed = text(t("allowance.budget.allowedLabel"), t("allowance.budget.allowedHelper"));
  const scope = dropdown(t("allowance.budget.scopeLabel"), t("allowance.budget.scopeHelper"), ["total", "each"], "allowance.budget");
  const reset = dropdown(t("allowance.budget.resetLabel"), t("allowance.budget.resetHelper"), ["never", "every", "condition"], "allowance.budget");
  // Given back every so long: how long, and counted from the session's start or the clock.
  const every = text(t("allowance.budget.everyLabel"), t("allowance.budget.everyHelper"));
  const resetAnchor = dropdown(t("allowance.budget.anchorLabel"), t("allowance.budget.anchorHelper"), ["start", "clock"], "allowance.budget");
  const everyPart = el("div", { class: "fields" }, every.node, resetAnchor);
  const blocked = text(t("allowance.cycle.blockedLabel"), t("allowance.cycle.helper"));
  const cycleAllowed = text(t("allowance.cycle.allowedLabel"), t("allowance.cycle.helper"));
  const anchor = dropdown(t("allowance.cycle.anchorLabel"), t("allowance.cycle.anchorHelper"), ["start", "clock"], "allowance.cycle");
  const budgetPart = el("div", { class: "fields" }, allowed.node, scope, reset, everyPart);
  const cyclePart = el("div", { class: "fields" }, blocked.node, cycleAllowed.node, anchor);
  // Nothing enforced for a while after the start, or until a time (`timing.js`).
  const delayed = timingFields(["after", "from"], "delayed");
  const delayedPart = el("div", { class: "fields" }, ...delayed.nodes);
  const summary = el("p", { class: "cds--type-body-01 week-summary" });
  let problem = null;
  // What is wrong is only said once a save was tried (or after, as it is changed).
  let tried = false;
  let asked = 0;
  let timer = null;

  const terms = () => {
    if (type.value === "pomodoro") return { type: "pomodoro", blocked: blocked.text(), allowed: cycleAllowed.text(), anchor: anchor.value };
    if (type.value === "delayed") return { type: "delayed", ...delayed.value() };
    if (type.value === "budget") {
      const often = reset.value === "every" ? { every: every.text(), anchor: resetAnchor.value } : {};
      return { type: "budget", allowed: allowed.text(), scope: scope.value, reset: reset.value, ...often };
    }
    return null;
  };
  // The terms in words; or why they cannot be saved. Only the latest answer counts.
  async function describe() {
    const mine = ++asked;
    const wanted = terms();
    let words = "";
    let wrong = null;
    if (wanted) {
      try {
        words = (await invoke("describe_allowance", { terms: wanted })).words;
      } catch (e) {
        wrong = String(e);
      }
    }
    if (mine !== asked) return;
    problem = wrong;
    summary.textContent = wrong ? (tried ? wrong : "") : words;
    summary.classList.toggle("problem", !!wrong && tried);
  }
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(describe, 300);
  };
  const show = () => {
    budgetPart.hidden = type.value !== "budget";
    cyclePart.hidden = type.value !== "pomodoro";
    delayedPart.hidden = type.value !== "delayed";
    everyPart.hidden = reset.value !== "every";
    summary.hidden = !terms();
  };
  for (const field of [allowed, blocked, cycleAllowed, every, delayed]) field.onChange(soon);
  for (const field of [scope, anchor, resetAnchor]) field.addEventListener("cds-dropdown-selected", describe);
  reset.addEventListener("cds-dropdown-selected", () => {
    show();
    describe();
  });
  type.addEventListener("cds-dropdown-selected", () => {
    show();
    describe();
  });
  return {
    nodes: [type, budgetPart, cyclePart, delayedPart, summary],
    // Both types' fields start from what the session has, else from an example of each.
    fill(allowance) {
      type.value = (allowance && allowance.type) || "none";
      const budget = allowance && allowance.type === "budget" ? allowance : {};
      const cycle = allowance && allowance.type === "pomodoro" ? allowance : {};
      const delay = allowance && allowance.type === "delayed" ? allowance : null;
      delayed.fill(delay);
      allowed.set(budget.allowed || 0);
      scope.value = budget.scope || "total";
      reset.value = budget.reset || "every";
      every.set(budget.every || 0);
      resetAnchor.value = budget.reset === "every" ? budget.anchor : "clock";
      // A new cycle is the classic one; the rest start empty.
      blocked.set(cycle.blocked || "25m");
      cycleAllowed.set(cycle.allowed || "5m");
      anchor.value = cycle.anchor || "start";
      show();
      tried = false;
      summary.textContent = t("allowance.describing");
      describe();
    },
    read: terms,
    settle: async () => {
      clearTimeout(timer);
      tried = true;
      await describe();
    },
    error: () => problem,
  };
}

// The weekly schedule: as text (what the CLI takes too) and as the week, a column per day and a
// cell per half hour, that can be dragged over. Carbon has no calendar or time-grid component, so
// the week is plain layout in Carbon's tokens; the text field is the accessible way to set it,
// and the week mirrors it.
const DAY_NAMES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].map((day) => t(`conditions.days.${day}`));
const SLOT = 30; // minutes per cell

function weekEditor() {
  // What is marked, written out by the backend, below it (how to use it is its section's line).
  const grid = el("div", { class: "week", "aria-hidden": "true" });
  const summary = el("p", { class: "cds--type-body-01 week-summary" });
  const cells = DAY_NAMES.map(() => []);
  let days = DAY_NAMES.map(() => []);
  let disabled = false;
  let problem = null;

  const place = (node, column, row, rows) => {
    node.style.gridColumn = String(column);
    node.style.gridRow = `${row} / span ${rows}`;
    grid.append(node);
  };
  // Rows: the day names take the first three, then a row per half hour from row 4. An hour's
  // label spans the two rows above its line and the two below, and is centred on that line:
  // 00:00 on the top edge of the grid, 24:00 on its bottom edge.
  const SLOTS = (24 * 60) / SLOT;
  DAY_NAMES.forEach((name, day) => place(el("div", { class: "day cds--type-label-01" }, name), day + 2, 1, 3));
  for (let hour = 0; hour <= 24; hour += 2) place(el("div", { class: "hour cds--type-label-01" }, `${String(hour).padStart(2, "0")}:00`), 1, hour * 2 + 2, 4);
  for (let slot = 0; slot < SLOTS; slot++) {
    DAY_NAMES.forEach((_, day) => {
      const edges = [slot % 2 === 0 ? "hour-start" : "", slot === SLOTS - 1 ? "last" : ""].join(" ");
      const cell = el("div", { class: `cell ${edges}`.trim(), "data-day": day, "data-slot": slot });
      cells[day].push(cell);
      place(cell, day + 2, slot + 4, 1);
    });
  }

  // A cell is marked when any of its half hour is in the schedule (a typed one can be finer).
  function draw() {
    cells.forEach((column, day) =>
      column.forEach((cell, slot) => {
        const [start, end] = [slot * SLOT, slot * SLOT + SLOT];
        cell.classList.toggle("on", days[day].some(([from, to]) => from < end && to > start));
      }),
    );
    grid.classList.toggle("disabled", disabled);
  }

  // The week written out by the backend, as the CLI and the session tiles say it; or why it
  // cannot be saved (nothing marked).
  async function describe() {
    // Nothing marked yet (a new schedule): said plainly, not as an error, though it cannot be
    // saved like this.
    if (days.every((spans) => !spans.length)) {
      problem = t("conditions.nothingMarked");
      summary.textContent = problem;
      summary.classList.remove("problem");
      return;
    }
    try {
      summary.textContent = t("conditions.active", { times: await invoke("describe_schedule", { week: { days } }) });
      problem = null;
    } catch (e) {
      problem = String(e);
      summary.textContent = problem;
    }
    summary.classList.toggle("problem", !!problem);
  }

  // Dragging from one cell to another marks the rectangle between them (those days, those
  // times), or clears it when the drag starts on a marked cell. Only the days it covers are
  // rewritten, so a schedule made in the CLI keeps its exact minutes on the others.
  let painting = null;
  const cellAt = (event) => {
    const node = document.elementFromPoint(event.clientX, event.clientY);
    return node && node.classList.contains("cell") && grid.contains(node) ? node : null;
  };
  const position = (cell) => ({ day: Number(cell.dataset.day), slot: Number(cell.dataset.slot) });
  function paint(to) {
    const from = painting.from;
    const [firstDay, lastDay] = [Math.min(from.day, to.day), Math.max(from.day, to.day)];
    const [firstSlot, lastSlot] = [Math.min(from.slot, to.slot), Math.max(from.slot, to.slot)];
    cells.forEach((column, day) =>
      column.forEach((cell, slot) => {
        const inside = day >= firstDay && day <= lastDay && slot >= firstSlot && slot <= lastSlot;
        cell.classList.toggle("on", inside ? painting.on : painting.before[day][slot]);
      }),
    );
    for (let day = firstDay; day <= lastDay; day++) painting.days.add(day);
  }
  function spansOf(day) {
    const spans = [];
    cells[day].forEach((cell, slot) => {
      if (!cell.classList.contains("on")) return;
      const last = spans[spans.length - 1];
      if (last && last[1] === slot * SLOT) last[1] += SLOT;
      else spans.push([slot * SLOT, slot * SLOT + SLOT]);
    });
    return spans;
  }
  grid.addEventListener("pointerdown", (event) => {
    const cell = cellAt(event);
    if (disabled || !cell) return;
    event.preventDefault();
    grid.setPointerCapture(event.pointerId);
    const before = cells.map((column) => column.map((c) => c.classList.contains("on")));
    painting = { from: position(cell), on: !cell.classList.contains("on"), before, days: new Set() };
    paint(position(cell));
  });
  grid.addEventListener("pointermove", (event) => {
    const cell = painting && cellAt(event);
    if (cell) paint(position(cell));
  });
  const finish = () => {
    if (!painting) return;
    for (const day of painting.days) days[day] = spansOf(day);
    painting = null;
    draw();
    describe();
  };
  grid.addEventListener("pointerup", finish);
  grid.addEventListener("pointercancel", finish);

  return {
    nodes: [grid, summary],
    read: () => ({ days: days.map((spans) => spans.map((span) => [...span])) }),
    fill: (values) => {
      days = values.days.map((spans) => spans.map((span) => [...span]));
      draw();
      describe();
    },
    // Asked every second; only a change redraws, so a drag in progress is left alone.
    disable: (off) => {
      if (off === disabled) return;
      disabled = off;
      draw();
    },
    settle: async () => {},
    error: () => problem,
  };
}

export { LISTS, SECTIONS, ACTIONS, targetForm, conditionForm, allowanceSection };
