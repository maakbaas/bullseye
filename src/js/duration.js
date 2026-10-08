// A duration as Carbon text inputs for numbers, one per unit (days, hours, minutes, seconds), in place of a
// text field to type `2h 30m` in. The service still takes and keeps the text (`text()` writes it the
// way `duration.rs` reads it), so nothing but this field changes. Which units show is up to where it
// is used: a field with no days takes `36` hours.

import { el, errorNote, invalid, t } from "./core.js";

const SECONDS = { d: 86400, h: 3600, m: 60, s: 1 };
const ALL = ["d", "h", "m", "s"];

// A duration typed or kept as text (`2h 30m`, `90 minutes`, `1d`), in seconds; its units go by their
// first letter. What `duration.rs` would refuse is not the field's to say: it only shows what it can.
function secondsOf(text) {
  let total = 0;
  for (const [, number, word] of String(text || "").toLowerCase().matchAll(/(\d+)\s*([a-z]+)/g)) total += Number(number) * (SECONDS[word[0]] || 0);
  return total;
}

// `label` and `helper` as a text field has them; `units`: those that show, largest first.
export function durationField({ label, helper = "", units = ALL }) {
  const title = el("p", { class: "cds--type-label-01" }, label);
  const inputs = units.map((unit) => {
    // Not Carbon's number input: its field has a width of its own that is too wide for four in a
    // row. Digits only; a unit that runs over its next one up (90 minutes) is carried by `text()`.
    const input = el("cds-text-input", { label: t(`duration.${unit}`), inputmode: "numeric", value: "0" });
    input.addEventListener("input", () => {
      const digits = String(input.value).replace(/\D/g, "").slice(0, 4);
      if (digits !== input.value) input.value = digits;
    });
    return input;
  });
  const row = el("div", { class: "duration-fields" }, ...inputs);
  const node = el("div", { class: "duration" }, title, row);
  const help = helper ? el("p", { class: "cds--type-helper-text-01" }, helper) : null;
  if (help) node.append(help);
  // What is wrong goes under the whole field, not under its last box: its boxes are marked, the
  // reason is a notification in the helper's place.
  const note = errorNote();
  node.append(note.node);

  const seconds = () =>
    inputs.reduce((total, input, at) => total + Math.max(0, Math.floor(Number(input.value) || 0)) * SECONDS[units[at]], 0);
  return {
    node,
    seconds,
    // In seconds, split over the units that show, the largest taking what it can.
    set(value) {
      let rest = typeof value === "number" ? value : secondsOf(value);
      inputs.forEach((input, at) => {
        const whole = Math.floor(rest / SECONDS[units[at]]);
        rest -= whole * SECONDS[units[at]];
        input.value = String(whole);
      });
    },
    // `2h 30m`, written the way the service reads it; empty for none.
    text() {
      let rest = seconds();
      return units
        .map((unit) => {
          const whole = Math.floor(rest / SECONDS[unit]);
          rest -= whole * SECONDS[unit];
          return whole ? `${whole}${unit}` : "";
        })
        .filter(Boolean)
        .join(" ");
    },
    // The reason it cannot be used, on the field (null clears it); true when there is one.
    error(why) {
      for (const input of inputs) invalid(input, why ? " " : null);
      if (help) help.hidden = !!why;
      note.show(why);
      return !!why;
    },
    onChange(handler) {
      node.addEventListener("input", handler);
    },
    disable(off) {
      for (const input of inputs) input.disabled = off;
    },
  };
}
