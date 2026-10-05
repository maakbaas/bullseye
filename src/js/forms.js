// What block lists are made of (`LISTS`, `SECTIONS`) and the editors' forms: a block, a block list,
// a schedule (with its week grid). Every form has `nodes`, `fill(item)`, `read(item)`,
// `disable(off)`, `settle()` and `error()`.

import { el, invoke, lines, overview, t } from "./core.js";

// ---- What things are made of. Three kinds, each a list of its own: block lists (what to
// block), schedules (when) and blocks (one or more block lists, a schedule and a lock: what is
// started). The editors, tiles and summaries are built from these tables, so a new kind of
// list, enforcer or lock method is an entry here (plus its backend side, see CLAUDE.md).

// The lists a block list can have (`definitions::BlockList`), by field name.
const LISTS = Object.fromEntries(
  ["filters", "allowed", "programs"].map((key) => [
    key,
    {
      label: t(`lists.${key}.label`),
      placeholder: t(`lists.${key}.placeholder`),
      helper: t(`lists.${key}.helper`),
      count: (n) => t(`lists.${key}.count`, { n }),
    },
  ]),
);

// The enforcers (`state::Enforcers`), grouped into the block list editor's sections with the
// lists they use.
const enforcer = (key) => ({ key, label: t(`lists.enforcers.${key}.label`), short: window.STRINGS.lists.enforcers[key].short });
const SECTIONS = [
  { key: "websites", lists: ["filters", "allowed"], enforcers: [enforcer("chromium_tabs"), enforcer("chromium_split_view")] },
  { key: "programs", lists: ["programs"], enforcers: [enforcer("exe_names")] },
  { key: "apps", lists: [], enforcers: [enforcer("obsidian_web_viewer")] },
].map((section) => ({ ...section, title: t(`lists.sections.${section.key}.title`), description: t(`lists.sections.${section.key}.description`) }));

const ENFORCERS = SECTIONS.flatMap((section) => section.enforcers);

// ---- The editors. Each kind's form is built once and reused; every form has `nodes`,
// `fill(item)`, `read(item)` (the item with the form's values), `disable(off)`, `settle()`
// (finish what is being typed) and `error()` (what is wrong, or null).

function nameField(placeholder) {
  const input = el("cds-text-input", { label: t("common.name"), placeholder, "max-count": "60" });
  input.addEventListener("input", () => (input.invalid = false));
  return input;
}

// The schedule select's choice for no schedule (an empty value reads as nothing chosen).
const ALWAYS = "always";

function blockForm() {
  const name = nameField(t("blocks.namePlaceholder"));
  const lists = el("cds-checkbox-group", { "legend-text": t("blocks.listsLegend") });
  const noLists = el("p", { class: "cds--type-body-01" }, t("blocks.noLists"));
  const schedule = el("cds-select", { "label-text": t("blocks.scheduleLabel"), "helper-text": t("blocks.scheduleHelper") });
  // Made here and added to this block: the editor comes back with it.
  const newList = el("cds-button", { kind: "tertiary", size: "md" }, t("blocks.newList"), carbonIcon("add", "icon"));
  const newSchedule = el("cds-button", { kind: "tertiary", size: "md" }, t("blocks.newSchedule"), carbonIcon("add", "icon"));
  // The editor makes it (`editor.js`, the `bullseye-make` event).
  const make = (kind) => () => newList.dispatchEvent(new CustomEvent("bullseye-make", { detail: kind, bubbles: true }));
  newList.addEventListener("click", make("lists"));
  newSchedule.addEventListener("click", make("schedules"));
  let boxes = {};
  const section = (title, text, ...content) =>
    el("section", {}, el("h2", { class: "cds--type-productive-heading-03" }, title), el("p", { class: "cds--type-body-01" }, text), ...content);
  return {
    nodes: [
      el("section", {}, name),
      section(t("blocks.whatTitle"), t("blocks.whatText"), lists, noLists, el("div", { class: "inline-action" }, newList)),
      section(t("blocks.whenTitle"), t("blocks.whenText"), schedule, el("div", { class: "inline-action" }, newSchedule)),
    ],
    fill(block) {
      name.value = block.name;
      boxes = {};
      lists.replaceChildren(
        ...overview.lists.map((list) => {
          const box = el("cds-checkbox", { "label-text": list.name });
          box.checked = block.lists.includes(list.id);
          boxes[list.id] = box;
          return box;
        }),
      );
      lists.hidden = !overview.lists.length;
      noLists.hidden = !!overview.lists.length;
      schedule.replaceChildren(
        el("cds-select-item", { value: ALWAYS, label: t("blocks.always") }, t("blocks.always")),
        ...overview.schedules.map((s) => el("cds-select-item", { value: s.id, label: s.name }, s.name)),
      );
      schedule.value = block.schedule || ALWAYS;
    },
    read: (block) => ({
      ...block,
      name: name.value.trim(),
      lists: overview.lists.filter((l) => boxes[l.id] && boxes[l.id].checked).map((l) => l.id),
      schedule: schedule.value && schedule.value !== ALWAYS ? schedule.value : null,
    }),
    disable(off) {
      for (const control of [name, schedule, newList, newSchedule, ...Object.values(boxes)]) control.disabled = off;
    },
    settle: async () => {},
    error: () => (name.value.trim() ? null : t("blocks.nameMissing")),
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

function listForm() {
  const name = nameField(t("lists.namePlaceholder"));
  const areas = {};
  const checks = {};
  const nodes = [el("section", {}, name)];
  for (const section of SECTIONS) {
    const lists = el("div", { class: "lists" });
    for (const key of section.lists) {
      const list = LISTS[key];
      areas[key] = el("cds-textarea", { label: list.label, placeholder: list.placeholder, "helper-text": list.helper, rows: 5 });
      lists.append(areas[key]);
    }
    sameHeight(section.lists.map((key) => areas[key]));
    const group = el("cds-checkbox-group", { "legend-text": t(section.lists.length ? "lists.howLegend" : "lists.enforcersLegend") });
    for (const enforcer of section.enforcers) {
      checks[enforcer.key] = el("cds-checkbox", { "label-text": enforcer.label });
      group.append(checks[enforcer.key]);
    }
    nodes.push(
      el(
        "section",
        {},
        el("h2", { class: "cds--type-productive-heading-03" }, section.title),
        el("p", { class: "cds--type-body-01" }, section.description),
        lists,
        group,
      ),
    );
  }
  return {
    nodes,
    fill(list) {
      name.value = list.name;
      for (const [key, area] of Object.entries(areas)) area.value = (list[key] || []).join("\n");
      for (const [key, box] of Object.entries(checks)) box.checked = !!(list.enforcers && list.enforcers[key]);
    },
    read(list) {
      const out = { ...list, name: name.value.trim(), enforcers: { ...list.enforcers } };
      for (const [key, area] of Object.entries(areas)) out[key] = lines(area.value);
      for (const [key, box] of Object.entries(checks)) out.enforcers[key] = box.checked;
      return out;
    },
    disable(off) {
      for (const control of [name, ...Object.values(areas), ...Object.values(checks)]) control.disabled = off;
    },
    settle: async () => {},
    error: () => (name.value.trim() ? null : t("lists.nameMissing")),
    name,
  };
}

function scheduleForm() {
  const name = nameField(t("schedules.namePlaceholder"));
  const week = weekEditor();
  return {
    nodes: [el("section", {}, name), el("section", {}, ...week.nodes)],
    fill(schedule) {
      name.value = schedule.name;
      week.fill(schedule);
    },
    read: (schedule) => ({ ...schedule, name: name.value.trim(), days: week.read().days }),
    disable(off) {
      name.disabled = off;
      week.disable(off);
    },
    settle: () => week.settle(),
    error: () => (name.value.trim() ? week.error() : t("schedules.nameMissing")),
    name,
  };
}

// The weekly schedule: as text (what the CLI takes too) and as the week, a column per day and a
// cell per half hour, that can be dragged over. Carbon has no calendar or time-grid component, so
// the week is plain layout in Carbon's tokens; the text field is the accessible way to set it,
// and the week mirrors it.
const DAY_NAMES = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"].map((day) => t(`schedules.days.${day}`));
const SLOT = 30; // minutes per cell

function weekEditor() {
  // How to use it above, and what is marked, written out by the backend, below.
  const help = el("p", { class: "cds--type-body-01 week-help" }, t("schedules.gridHelp"));
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
      problem = t("schedules.nothingMarked");
      summary.textContent = problem;
      summary.classList.remove("problem");
      return;
    }
    try {
      summary.textContent = t("schedules.active", { times: await invoke("describe_schedule", { week: { days } }) });
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

export { LISTS, SECTIONS, ENFORCERS, ALWAYS, blockForm, listForm, scheduleForm };
