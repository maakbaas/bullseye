// A duration as one Carbon text input for a number and a dropdown for its unit (days, hours, minutes,
// and seconds where they make sense), in place of a text field to type `2h 30m` in: an hour and a half
// is 90 minutes. The service still takes and keeps the text (`text()` writes it the way `duration.rs`
// reads it), so nothing but this field changes.

import { el, invalid, t } from "./core.js";

const SECONDS = { d: 86400, h: 3600, m: 60, s: 1 };
const ALL = ["d", "h", "m", "s"];

// A duration typed or kept as text (`2h 30m`, `90 minutes`, `1d`), in seconds; its units go by their
// first letter. What `duration.rs` would refuse is not the field's to say: it only shows what it can.
function secondsOf(text) {
  let total = 0;
  for (const [, number, word] of String(text || "").toLowerCase().matchAll(/(\d+)\s*([a-z]+)/g)) total += Number(number) * (SECONDS[word[0]] || 0);
  return total;
}

// A whole number typed into a Carbon text input, digits only and at most `length` of them: not
// Carbon's number input, whose stepper buttons make a row busy. Also a random text's length and the
// number of breaks.
export function numberField({ label, helper = "", length = 6, value = "0" }) {
  const input = el("cds-text-input", { label, "helper-text": helper, inputmode: "numeric", value });
  input.addEventListener("input", () => {
    const digits = String(input.value).replace(/\D/g, "").slice(0, length);
    if (digits !== input.value) input.value = digits;
  });
  return input;
}

// `label` and `helper` as a text field has them; `units`: those the dropdown offers, largest first.
export function durationField({ label, helper = "", units = ALL }) {
  const input = numberField({ label, helper });
  const unit = el("cds-dropdown", { autoalign: true, "title-text": t("duration.unit") });
  for (const u of units) unit.append(el("cds-dropdown-item", { value: u }, t(`duration.${u}`)));
  unit.value = units.includes("m") ? "m" : units[units.length - 1];
  const node = el("div", { class: "duration" }, input, unit);

  const seconds = () => Math.max(0, Math.floor(Number(input.value) || 0)) * SECONDS[unit.value];
  return {
    node,
    seconds,
    // In seconds: in the largest unit it is a whole number of (a new one, zero, keeps the unit shown).
    set(value) {
      const total = typeof value === "number" ? value : secondsOf(value);
      if (total) unit.value = units.find((u) => total % SECONDS[u] === 0) || units[units.length - 1];
      input.value = String(Math.floor(total / SECONDS[unit.value]));
    },
    // `90m`, written the way the service reads it; empty for none.
    text() {
      const n = Math.max(0, Math.floor(Number(input.value) || 0));
      return n ? `${n}${unit.value}` : "";
    },
    // The reason it cannot be used, on the field (null clears it); true when there is one.
    error(why) {
      return invalid(input, why);
    },
    onChange(handler) {
      input.addEventListener("input", handler);
      unit.addEventListener("cds-dropdown-selected", handler);
    },
    disable(off) {
      input.disabled = off;
      unit.disabled = off;
    },
  };
}
