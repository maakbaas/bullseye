// A time given either as how long (`2h 30m`) or as when (a date and a time): the fields of a timed
// lock and of a delayed start. A dropdown chooses which, and the fields of the one chosen show. The
// service reads both (`describe_duration`, `describe_until`), so a wrong or past time is said in its
// words.

import { el, invalid, invoke, localDate, t } from "./core.js";
import { durationField } from "./duration.js";

// `kinds`: how long and when, as the service names them (`for` and `until` for a lock, `after` and
// `from` for a delayed start); `text`: where their words are, `timing.<text>` in `strings.js`.
export function timingFields([long, when], text) {
  const words = (key) => t(`timing.${text}.${key}`);
  const kind = el("cds-dropdown", { autoalign: true, "title-text": words("kind") });
  kind.append(el("cds-dropdown-item", { value: long }, words(long)), el("cds-dropdown-item", { value: when }, words(when)));
  const input = durationField({ label: words("field"), helper: words("helper") });
  // Carbon's date picker and time picker side by side.
  const datePicker = el("cds-date-picker", { "date-format": "Y-m-d", "min-date": localDate(new Date()) });
  const dateInput = el("cds-date-picker-input", { kind: "single", "label-text": t("timing.date"), placeholder: t("timing.datePlaceholder") });
  datePicker.append(dateInput);
  const timePicker = el("cds-time-picker", { "label-text": t("timing.time"), placeholder: t("timing.timePlaceholder"), "max-length": "5" });
  const row = el("div", { class: "date-time" }, datePicker, timePicker);
  kind.value = long;
  const show = () => {
    input.node.hidden = kind.value !== long;
    row.hidden = kind.value !== when;
  };
  kind.addEventListener("cds-dropdown-selected", show);
  show();
  return {
    nodes: [kind, input.node, row],
    // What the session had (`{ kind, value }`), or the first kind with nothing in it.
    fill(settings) {
      const { kind: chosen, value = "" } = settings || {};
      kind.value = chosen === when ? when : long;
      const [day, time] = chosen === when ? value.split(" ") : [];
      input.set(chosen === when ? 0 : value);
      datePicker.value = day || "";
      timePicker.value = time || "";
      show();
    },
    // The kind and value as they stand, unchecked: the service says what is wrong with them.
    value() {
      if (kind.value !== when) return { kind: long, value: input.text() };
      return { kind: when, value: `${datePicker.value} ${timePicker.value.trim()}`.trim() };
    },
    onChange(handler) {
      kind.addEventListener("cds-dropdown-selected", handler);
      input.onChange(handler);
      datePicker.addEventListener("cds-date-picker-changed", handler);
      timePicker.addEventListener("input", handler);
    },
    // `{ kind, value }`, or null with the fault shown on the field it belongs to.
    async read() {
      input.error(null);
      invalid(dateInput, null);
      invalid(timePicker, null);
      if (kind.value === long) {
        const value = input.text();
        const fault = await invoke("describe_duration", { text: value }).then(() => null, (e) => String(e));
        return input.error(fault) ? null : { kind: long, value };
      }
      if (invalid(dateInput, datePicker.value ? null : t("timing.dateMissing"))) return null;
      if (invalid(timePicker, timePicker.value.trim() ? null : t("timing.timeMissing"))) return null;
      const value = `${datePicker.value} ${timePicker.value.trim()}`;
      const fault = await invoke("describe_until", { text: value }).then(() => null, (e) => String(e));
      return invalid(timePicker, fault) ? null : { kind: when, value };
    },
  };
}
