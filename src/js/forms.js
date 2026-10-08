// What block lists are made of (`LISTS`, `SECTIONS`) and the editors' forms: a block, a block list,
// a condition (with its week grid or its flash drive). Every form has `nodes`, `fill(item)`, `read(item)`,
// `disable(off)`, `settle()` and `error()`.

import { el, invalid, invoke, lines, overview, t } from "./core.js";

// ---- What things are made of. Three kinds, each a list of its own: block lists (what to
// block), conditions (when) and blocks (one or more block lists, a condition, an allowance and a
// lock: what is started; the allowance and the lock are set on the block itself). The editors, tiles and summaries are built from these tables, so a new kind of
// list, enforcer or lock method is an entry here (plus its backend side, see CLAUDE.md).

// The lists a block list can have (`definitions::BlockList`), by field name.
const LISTS = Object.fromEntries(
  ["filters", "allowed", "programs", "store_apps", "titles"].map((key) => [
    key,
    {
      label: t(`lists.${key}.label`),
      placeholder: t(`lists.${key}.placeholder`),
      helper: t(`lists.${key}.helper`),
      count: (n) => t(`lists.${key}.count`, { n }),
      // The Store apps are ticked from the installed ones, not typed (`appPicker`).
      picker: key === "store_apps",
    },
  ]),
);

// The enforcers (`state::Enforcers`), grouped into the block list editor's sections with the
// lists they use.
const enforcer = (key) => ({ key, label: t(`lists.enforcers.${key}.label`), short: window.STRINGS.lists.enforcers[key].short });
const SECTIONS = [
  { key: "websites", lists: ["filters", "allowed"], enforcers: [enforcer("chromium_tabs"), enforcer("chromium_split_view"), enforcer("chromium_app_windows"), enforcer("firefox_tabs"), enforcer("firefox_split_view")] },
  { key: "programs", lists: ["programs"], enforcers: [enforcer("exe_names")] },
  { key: "store", lists: ["store_apps"], enforcers: [enforcer("store_apps")] },
  { key: "titles", lists: ["titles"], enforcers: [enforcer("window_titles")] },
  { key: "other", lists: [], enforcers: [enforcer("obsidian_web_viewer")] },
].map((section) => ({ ...section, title: t(`lists.sections.${section.key}.title`), description: window.STRINGS.lists.sections[section.key].description && t(`lists.sections.${section.key}.description`) }));

const ENFORCERS = SECTIONS.flatMap((section) => section.enforcers);

// ---- The editors. Each kind's form is built once and reused; every form has `nodes`,
// `fill(item)`, `read(item)` (the item with the form's values), `disable(off)`, `settle()`
// (finish what is being typed) and `error()` (what is wrong, or null).

function nameField(placeholder) {
  const input = el("cds-text-input", { label: t("common.name"), placeholder, "max-count": "60" });
  input.addEventListener("input", () => (input.invalid = false));
  return input;
}

// The name's place in a form: a text box. A new item has a name already, the first free "Block list 3"
// (`freeName`); `more` goes under it in the same section.
function nameSection(input, ...more) {
  return el("section", {}, input, ...more);
}

// The condition select's choice for no condition (an empty value reads as nothing chosen).
const ALWAYS = "always";
function blockForm() {
  const name = nameField(t("blocks.namePlaceholder"));
  const lists = el("cds-checkbox-group", { "legend-text": t("blocks.listsLegend") });
  const noLists = el("p", { class: "cds--type-body-01" }, t("blocks.noLists"));
  const condition = el("cds-dropdown", { autoalign: true, "title-text": t("blocks.conditionLabel"), "helper-text": t("blocks.conditionHelper") });
  // Made here and added to this block: the editor comes back with it.
  const newList = el("cds-button", { kind: "tertiary", size: "md" }, t("blocks.newList"), carbonIcon("add", "icon"));
  const newCondition = el("cds-button", { kind: "tertiary", size: "md" }, t("blocks.newCondition"), carbonIcon("add", "icon"));
  // The editor makes it (`editor.js`, the `bullseye-make` event).
  const make = (kind, id = null) => () => newList.dispatchEvent(new CustomEvent("bullseye-make", { detail: { kind, id }, bubbles: true }));
  newList.addEventListener("click", make("lists"));
  newCondition.addEventListener("click", make("conditions"));
  // Changing one that exists: the same detour, to its editor.
  const editCondition = el("cds-button", { kind: "tertiary", size: "md" }, t("blocks.editCondition"), carbonIcon("edit", "icon"));
  editCondition.addEventListener("click", () => condition.value && condition.value !== ALWAYS && make("conditions", condition.value)());
  // The allowance, set here on the block like its lock is on the tile (`allowanceSection`).
  const allowance = allowanceSection();
  let off = false;
  const syncEdit = () => {
    editCondition.disabled = off || !condition.value || condition.value === ALWAYS;
  };
  condition.addEventListener("cds-dropdown-selected", syncEdit);
  let boxes = {};
  let edits = [];
  const section = (title, text, ...content) =>
    el("section", {}, el("h2", { class: "cds--type-productive-heading-03" }, title), el("p", { class: "cds--type-body-01" }, text), ...content);
  return {
    nodes: [
      nameSection(name),
      section(t("blocks.whatTitle"), t("blocks.whatText"), lists, noLists, el("div", { class: "inline-action" }, newList)),
      section(t("blocks.whenTitle"), t("blocks.whenText"), condition, el("div", { class: "inline-action" }, newCondition, editCondition)),
      section(t("blocks.allowanceTitle"), t("blocks.allowanceText"), ...allowance.nodes),
    ],
    fill(block, isNew = false) {
      name.value = block.name;
      boxes = {};
      edits = [];
      lists.replaceChildren(
        ...overview.lists.map((list) => {
          const box = el("cds-checkbox", { "label-text": list.name });
          box.checked = block.lists.includes(list.id);
          boxes[list.id] = box;
          const edit = el("cds-icon-button", { kind: "ghost", size: "sm", align: "right" }, carbonIcon("edit"), el("span", { slot: "tooltip-content" }, t("blocks.editList")));
          edit.addEventListener("click", make("lists", list.id));
          edits.push(edit);
          edit.disabled = off;
          return el("div", { class: "list-row" }, box, edit);
        }),
      );
      lists.hidden = !overview.lists.length;
      noLists.hidden = !!overview.lists.length;
      condition.replaceChildren(
        el("cds-dropdown-item", { value: ALWAYS }, t("blocks.always")),
        ...overview.conditions.map((s) => el("cds-dropdown-item", { value: s.id }, s.name)),
      );
      condition.value = block.condition || ALWAYS;
      allowance.fill(block.allowance);
      syncEdit();
    },
    read: (block) => ({
      ...block,
      name: name.value.trim(),
      lists: overview.lists.filter((l) => boxes[l.id] && boxes[l.id].checked).map((l) => l.id),
      condition: condition.value && condition.value !== ALWAYS ? condition.value : null,
      allowance: allowance.read(),
    }),
    // A locked block: only its name and its allowance stay open, as the service takes a new name and a
    // stricter allowance from it (and judges which is stricter).
    disable(value) {
      off = value;
      for (const control of [condition, newList, newCondition, ...edits, ...Object.values(boxes)]) control.disabled = off;
      syncEdit();
    },
    settle: () => allowance.settle(),
    error: () => (name.value.trim() ? allowance.error() : t("blocks.nameMissing")),
    name,
  };
}

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

// The Store apps of a block list: the installed ones (`installed_apps`, which takes a second or
// two) to tick, with the ones on the list that are not among them (not installed any more, or a
// pattern typed in the CLI) so saving keeps them. It stands in for a text area: `value` is the
// ticked family names, one per line, and `disabled` is settable.
function appPicker(list) {
  // The label is above the box, which has a background of its own; the group is named for readers.
  const group = el("cds-checkbox-group", { "aria-label": list.label });
  const label = el("p", { class: "cds--type-label-01" }, list.label);
  const status = el("p", { class: "cds--type-helper-text-01" }, t("lists.store_apps.loading"));
  const scroll = el("div", { class: "apps" }, group);
  const picker = el("div", { class: "app-picker" }, label, scroll, status);
  let chosen = new Set();
  let installed = [];
  let disabled = false;
  const same = (a, b) => a.toLowerCase() === b.toLowerCase();
  function draw() {
    const known = new Set(installed.map((app) => app.family.toLowerCase()));
    const rows = [...installed.map((app) => [app.family, app.name]), ...[...chosen].filter((family) => !known.has(family.toLowerCase())).map((family) => [family, family])];
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
  invoke("installed_apps").then(
    (apps) => {
      installed = apps;
      status.textContent = list.helper;
      draw();
    },
    (e) => {
      status.textContent = `${t("lists.store_apps.failed")}: ${e}`;
      draw();
    },
  );
  return picker;
}

function listForm() {
  const name = nameField(t("lists.namePlaceholder"));
  const areas = {};
  const checks = {};
  // The sections are an accordion: each opens and closes, and the ones with content are open when
  // the editor opens (`fill`): a section with a list when it has entries (the websites also in a new
  // block list), one with only enforcers ("other") when one is ticked, which a new block list has.
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
    const group = el("cds-checkbox-group", { "legend-text": t(section.lists.length ? "lists.howLegend" : "lists.enforcersLegend") });
    for (const enforcer of section.enforcers) {
      checks[enforcer.key] = el("cds-checkbox", { "label-text": enforcer.label });
      group.append(checks[enforcer.key]);
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
      for (const [key, box] of Object.entries(checks)) box.checked = !!(list.enforcers && list.enforcers[key]);
      for (const [section, item] of items) {
        const entries = section.lists.some((key) => (list[key] || []).length > 0);
        const ticked = section.enforcers.some((e) => list.enforcers && list.enforcers[e.key]);
        // A new block list has the websites open too, to start typing in.
        item.open = section.lists.length ? entries || (isNew && section.key === "websites") : ticked;
      }
    },
    read(list) {
      const out = { ...list, name: name.value.trim(), enforcers: { ...list.enforcers } };
      for (const [key, area] of Object.entries(areas)) out[key] = lines(area.value);
      for (const [key, box] of Object.entries(checks)) out.enforcers[key] = box.checked;
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
    error: () => (name.value.trim() ? null : t("lists.nameMissing")),
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

// One of a webhook's two matchers: what the answer must be, by a dropdown (HTTP code only, body only or
// both) and the fields it needs.
function matcherFields() {
  const mode = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.matchOn") });
  for (const key of ["status", "body", "both"]) mode.append(el("cds-dropdown-item", { value: key }, t(`conditions.webhook.on.${key}`)));
  const status = el("cds-text-input", { label: t("conditions.webhook.status"), placeholder: "200", "max-count": "3" });
  const body = el("cds-text-input", { label: t("conditions.webhook.body"), placeholder: t("conditions.webhook.bodyPlaceholder"), "helper-text": t("conditions.webhook.bodyHelper") });
  const show = () => {
    status.hidden = mode.value === "body";
    body.hidden = mode.value === "status";
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
      status: mode.value !== "body" && status.value.trim() ? Number(status.value.trim()) : null,
      body: mode.value !== "status" ? body.value : null,
    }),
    error() {
      invalid(status, null);
      invalid(body, null);
      if (mode.value !== "body" && !/^[1-5]\d\d$/.test(status.value.trim())) {
        invalid(status, t("conditions.webhook.statusWrong"));
        return t("conditions.webhook.statusWrong");
      }
      if (mode.value !== "status" && !body.value.trim()) {
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

// A webhook: one web address asked every so often (`webhook.rs`), GET or POST, and what its answer means: the
// true matcher first, then the false one, else the fallback. A Test button asks it once (`test_webhook`).
function webhookPicker() {
  const url = el("cds-text-input", { label: t("conditions.webhook.url"), placeholder: "https://example.com/state?room=1", "helper-text": t("conditions.webhook.urlHelper") });
  const method = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.method") });
  for (const key of ["get", "post"]) method.append(el("cds-dropdown-item", { value: key }, key.toUpperCase()));
  const every = el("cds-text-input", { label: t("conditions.webhook.every"), placeholder: "5", "helper-text": t("conditions.webhook.everyHelper") });
  const fallback = el("cds-dropdown", { autoalign: true, "title-text": t("conditions.webhook.fallback"), "helper-text": t("conditions.webhook.fallbackHelper") });
  for (const key of ["active", "inactive"]) fallback.append(el("cds-dropdown-item", { value: key }, t(`conditions.webhook.fallback_${key}`)));
  const whenTrue = matcherFields();
  const whenFalse = matcherFields();
  // What a test found, as a Carbon notification: how the condition comes out, and what was answered.
  const result = el("div", { class: "webhook-result" });
  const show = (kind, title, subtitle = "") => result.replaceChildren(el("cds-inline-notification", { kind, title, subtitle, "low-contrast": true, "hide-close-button": true }));
  // Headings as in a block's editor: what is asked, when it is active, when not, and otherwise.
  const part = (title, text, ...content) =>
    el("section", {}, el("h2", { class: "cds--type-productive-heading-03" }, title), el("p", { class: "cds--type-body-01" }, text), ...content);
  const test = el("cds-button", { kind: "tertiary", size: "md" }, t("conditions.webhook.test"), carbonIcon("play", "icon"));
  const read = () => ({
    url: url.value.trim(),
    method: method.value || "get",
    interval: Math.round(Number(every.value.trim()) * 60),
    fallback: fallback.value !== "inactive",
    when_true: whenTrue.read(),
    when_false: whenFalse.read(),
  });
  const error = () => {
    invalid(url, null);
    invalid(every, null);
    if (!/^https?:\/\/\S{3,}$/.test(url.value.trim())) {
      invalid(url, t("conditions.webhook.urlWrong"));
      return t("conditions.webhook.urlWrong");
    }
    const minutes = Number(every.value.trim());
    if (!Number.isFinite(minutes) || minutes < 1) {
      invalid(every, t("conditions.webhook.everyWrong"));
      return t("conditions.webhook.everyWrong");
    }
    return whenTrue.error() || whenFalse.error();
  };
  test.addEventListener("click", async () => {
    const wrong = error();
    if (wrong) return show("error", t("conditions.webhook.testNot"), wrong);
    test.disabled = true;
    show("info", t("conditions.webhook.testing"));
    try {
      const found = await invoke("test_webhook", { hook: read() });
      const answer = t("conditions.webhook.answer", { status: found.status, body: found.body });
      // Green: active. Yellow: not active. Red: neither matched, so the fallback decides.
      if (found.outcome === "neither") show("error", t(found.active ? "conditions.webhook.fellActive" : "conditions.webhook.fellInactive"), answer);
      else show(found.active ? "success" : "warning", t(found.active ? "conditions.webhook.isActive" : "conditions.webhook.isInactive"), answer);
    } catch (e) {
      show("error", t("conditions.webhook.testFailed"), String(e));
    }
    test.disabled = false;
  });
  return {
    nodes: [
      part(t("conditions.webhook.requestTitle"), t("conditions.webhook.requestText"), url, method, every),
      part(t("conditions.webhook.activeTitle"), t("conditions.webhook.activeText"), ...whenTrue.nodes),
      part(t("conditions.webhook.inactiveTitle"), t("conditions.webhook.inactiveText"), ...whenFalse.nodes),
      part(t("conditions.webhook.otherwiseTitle"), t("conditions.webhook.otherwiseText"), fallback),
      part(t("conditions.webhook.testTitle"), t("conditions.webhook.testText"), el("div", { class: "inline-action" }, test), result),
    ],
    fill(condition) {
      const hook = condition.type === "webhook" ? condition : {};
      url.value = hook.url || "";
      method.value = hook.method || "get";
      every.value = hook.interval ? String(Math.round(hook.interval / 60)) : "5";
      fallback.value = hook.fallback === false ? "inactive" : "active";
      whenTrue.fill(hook.when_true);
      whenFalse.fill(hook.when_false);
      result.replaceChildren();
    },
    read,
    error,
    disable(off) {
      for (const control of [url, method, every, fallback, test]) control.disabled = off;
      whenTrue.disable(off);
      whenFalse.disable(off);
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
  const weekPart = el("section", {}, ...week.nodes);
  const picker = drivePicker();
  const drivePart = el("section", {}, ...picker.nodes);
  const combine = combinePicker();
  const combinePart = el("section", {}, ...combine.nodes);
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

// A block's allowance, a section of the block form: its type (none, a time budget or a Pomodoro
// cycle) and what the type needs: how much time, for the block or each item, and when it is given
// back; or blocked for, allowed for, counted from where. Under it the terms in words, written by the
// backend (`describe_allowance`), which is also what says what cannot be saved. `fill(terms or null)`,
// `read()` (the terms, or null for none), `settle()` and `error()`.
function allowanceSection() {
  const dropdown = (label, helper, keys, path) => {
    const field = el("cds-dropdown", { autoalign: true, "title-text": label, "helper-text": helper });
    for (const key of keys) field.append(el("cds-dropdown-item", { value: key }, t(`${path}.${key}`)));
    return field;
  };
  const type = dropdown(t("allowance.typeLabel"), t("allowance.typeHelper"), ["none", "budget", "pomodoro"], "allowance.type");
  const text = (label, placeholder, helper) => el("cds-text-input", { label, placeholder, "helper-text": helper });
  const allowed = text(t("allowance.budget.allowedLabel"), t("allowance.budget.allowedPlaceholder"), t("allowance.budget.allowedHelper"));
  const scope = dropdown(t("allowance.budget.scopeLabel"), t("allowance.budget.scopeHelper"), ["total", "each"], "allowance.budget");
  const reset = dropdown(t("allowance.budget.resetLabel"), t("allowance.budget.resetHelper"), ["never", "hour", "day", "week", "condition"], "allowance.budget");
  const blocked = text(t("allowance.cycle.blockedLabel"), t("allowance.cycle.blockedPlaceholder"), t("allowance.cycle.helper"));
  const cycleAllowed = text(t("allowance.cycle.allowedLabel"), t("allowance.cycle.allowedPlaceholder"), t("allowance.cycle.helper"));
  const anchor = dropdown(t("allowance.cycle.anchorLabel"), t("allowance.cycle.anchorHelper"), ["start", "clock"], "allowance.cycle");
  const budgetPart = el("div", { class: "fields" }, allowed, scope, reset);
  const cyclePart = el("div", { class: "fields" }, blocked, cycleAllowed, anchor);
  const summary = el("p", { class: "cds--type-body-01 week-summary" });
  let problem = null;
  let asked = 0;
  let timer = null;

  const terms = () => {
    if (type.value === "pomodoro") return { type: "pomodoro", blocked: blocked.value.trim(), allowed: cycleAllowed.value.trim(), anchor: anchor.value };
    if (type.value === "budget") return { type: "budget", allowed: allowed.value.trim(), scope: scope.value, reset: reset.value };
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
    summary.textContent = wrong || words;
    summary.classList.toggle("problem", !!wrong);
  }
  const soon = () => {
    clearTimeout(timer);
    timer = setTimeout(describe, 300);
  };
  const show = () => {
    budgetPart.hidden = type.value !== "budget";
    cyclePart.hidden = type.value !== "pomodoro";
    summary.hidden = !terms();
  };
  for (const field of [allowed, blocked, cycleAllowed]) field.addEventListener("input", soon);
  for (const field of [scope, reset, anchor]) field.addEventListener("cds-dropdown-selected", describe);
  type.addEventListener("cds-dropdown-selected", () => {
    show();
    describe();
  });
  return {
    nodes: [type, budgetPart, cyclePart, summary],
    // Both types' fields start from what the block has, else from an example of each.
    fill(allowance) {
      type.value = (allowance && allowance.type) || "none";
      const budget = allowance && allowance.type === "budget" ? allowance : {};
      const cycle = allowance && allowance.type === "pomodoro" ? allowance : {};
      allowed.value = budget.allowed || "30m";
      scope.value = budget.scope || "total";
      reset.value = budget.reset || "day";
      blocked.value = cycle.blocked || "50m";
      cycleAllowed.value = cycle.allowed || "10m";
      anchor.value = cycle.anchor || "start";
      show();
      summary.textContent = t("allowance.describing");
      describe();
    },
    read: terms,
    settle: async () => {
      clearTimeout(timer);
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
  // How to use it above, and what is marked, written out by the backend, below.
  const help = el("p", { class: "cds--type-body-01 week-help" }, t("conditions.gridHelp"));
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

  // The week written out by the backend, as the CLI and the block tiles say it; or why it
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
    nodes: [help, grid, summary],
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

export { LISTS, SECTIONS, ENFORCERS, ALWAYS, blockForm, listForm, conditionForm };
