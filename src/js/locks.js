// The lock methods (`lock::Lock`), by `method`: their form in the dialog, the tile's line for a
// session that is not running, and what starting a session with one asks first.

import { durationText, el, invalid, invoke, localDate, overview, quoted, editHref, ref, t, tn, untilText, wordingOf } from "./core.js";

// A condition lock in words, the condition's name (`name`, a link to it) worded as its own line on a
// tile does. Not running (`state` null): "Locks while …". Running: the state it is in now, with when it
// changes if that is known: "Locked during …" / "Unlocked outside …" for a schedule, "Locked since … is unplugged" for a drive.
function conditionLockText(wording, name, state) {
  const kind = { drive: "Drive", true: "Combined", during: "" }[wording];
  if (!state) return tn(`locks.condition.describe${kind}`, { name });
  const { locked, change } = state;
  if (wording === "drive") return tn(`locks.condition.${locked ? "lockedDrive" : "unlockedDrive"}`, { name });
  if (wording === "true") return tn(`locks.condition.${locked ? "lockedTrue" : "unlockedFalse"}`, { name });
  return tn(`locks.condition.${locked ? "lockedDuring" : "unlockedOutside"}${change ? "For" : ""}`, { name, change });
}

// How a started session is kept from being stopped (`lock::Lock`), by `method`. Set on the
// session's tile, next to Start. `confirm` says what starting it asks first (null: nothing).
// What a password lock's fields show for a password that is set: 8 dots, masked or revealed.
const KEPT = "•".repeat(8);

// A lock method's settings for a session: its lock's if it has that method, else the ones the
// service kept from when it last had it (`kept_locks`; a password only as being set), else none.
const settingsOf = (session, method) =>
  (session.lock && session.lock.method === method ? session.lock : (session.kept_locks || []).find((lock) => lock.method === method)) || {};

const LOCKS = {
  none: {
    label: t("locks.none.label"),
    describe: () => t("locks.none.describe"),
    confirm: async () => null,
  },
  duration: {
    label: t("locks.duration.label"),
    form: (session) => {
      const input = el("cds-text-input", { label: t("locks.duration.field"), placeholder: t("locks.duration.placeholder"), "helper-text": t("locks.duration.helper") });
      input.value = settingsOf(session, "duration").duration || "";
      return {
        fields: [input],
        read: async () => {
          const duration = input.value.trim();
          const fault = await invoke("describe_duration", { text: duration }).then(() => null, (e) => String(e));
          return invalid(input, fault) ? null : { method: "duration", duration };
        },
      };
    },
    describe: (lock) => t("locks.duration.describe", { duration: durationText(lock.duration) }),
    confirm: async (session) => {
      const duration = await invoke("describe_duration", { text: session.lock.duration || "" }).catch(() => session.lock.duration);
      const values = { name: quoted(session.name), duration };
      return {
        heading: t("locks.duration.confirmHeading", values),
        body: t("locks.duration.confirmBody", values),
        action: t("locks.duration.confirmAction", values),
      };
    },
  },
  until: {
    label: t("locks.until.label"),
    // Carbon's date picker and time picker side by side; the backend reads the two together
    // (`describe_until`), so a wrong or past time is said in its words.
    form: (session) => {
      const [day, time] = (settingsOf(session, "until").until || "").split(" ");
      const datePicker = el("cds-date-picker", { "date-format": "Y-m-d", "min-date": localDate(new Date()) });
      const dateInput = el("cds-date-picker-input", { kind: "single", "label-text": t("locks.until.date"), placeholder: t("locks.until.datePlaceholder") });
      datePicker.append(dateInput);
      datePicker.value = day || "";
      const timePicker = el("cds-time-picker", { "label-text": t("locks.until.time"), placeholder: t("locks.until.timePlaceholder"), "max-length": "5" });
      timePicker.value = time || "";
      const row = el("div", { class: "date-time" }, datePicker, timePicker);
      return {
        fields: [row],
        read: async () => {
          invalid(dateInput, null);
          invalid(timePicker, null);
          if (invalid(dateInput, datePicker.value ? null : t("locks.until.dateMissing"))) return null;
          if (invalid(timePicker, timePicker.value.trim() ? null : t("locks.until.timeMissing"))) return null;
          const until = `${datePicker.value} ${timePicker.value.trim()}`;
          const fault = await invoke("describe_until", { text: until }).then(() => null, (e) => String(e));
          return invalid(timePicker, fault) ? null : { method: "until", until };
        },
      };
    },
    describe: (lock) => t("locks.until.describe", { when: untilText(lock.until) }),
    confirm: async (session) => {
      // A time that has passed does not start; the service says so.
      const end = await invoke("describe_until", { text: session.lock.until || "" }).catch(() => null);
      if (!end) return null;
      const values = { name: quoted(session.name), when: end.when, left: end.left };
      return {
        heading: t("locks.until.confirmHeading", values),
        body: t("locks.until.confirmBody", values),
        action: t("locks.until.confirmAction", values),
      };
    },
  },
  until_restart: {
    label: t("locks.until_restart.label"),
    describe: () => t("locks.until_restart.describe"),
    confirm: async (session) => ({
      heading: t("locks.until_restart.confirmHeading", { name: quoted(session.name) }),
      body: t("locks.until_restart.confirmBody", { name: quoted(session.name) }),
      action: t("locks.until_restart.confirmAction", { name: quoted(session.name) }),
    }),
  },
  condition: {
    label: t("locks.condition.label"),
    // A condition of its own: with the session's own condition, it is locked exactly while active.
    // A weekly schedule locks at its times; a flash drive locks while it is not connected, so
    // the drive is the key. Only with a condition to choose (`setLock` offers to make one).
    form: (session) => {
      const select = el("cds-dropdown", { autoalign: true, "title-text": t("locks.condition.field"), "helper-text": t("locks.condition.helper") });
      select.append(...overview.conditions.map((c) => el("cds-dropdown-item", { value: c.id }, c.name)));
      // The one it had, else the session's own condition, else the first there is.
      const exists = (id) => overview.conditions.some((c) => c.id === id);
      const chosen = [settingsOf(session, "condition").condition, session.condition].find(exists);
      select.value = chosen || (overview.conditions[0] || {}).id || "";
      return { fields: [select], read: async () => (invalid(select, select.value ? null : t("locks.condition.missing")) ? null : { method: "condition", condition: select.value }) };
    },
    describe: (lock) => {
      const condition = overview.conditions.find((c) => c.id === lock.condition);
      if (!condition) return t("locks.condition.describeGone");
      return conditionLockText(wordingOf(condition), ref(condition.name, editHref("conditions", condition.id)));
    },
    confirm: async (session) => {
      // Without its condition the service refuses the start, and says why.
      const condition = overview.conditions.find((c) => c.id === session.lock.condition);
      if (!condition) return null;
      const detail = condition.type === "flash_drive" ? t("locks.condition.driveDetail", { drive: quoted(condition.drive) }) : "";
      const values = { name: quoted(session.name), condition: quoted(condition.name), detail };
      return {
        heading: t("locks.condition.confirmHeading", values),
        body: t("locks.condition.confirmBody", values),
        action: t("locks.condition.confirmAction", values),
      };
    },
  },
  delay: {
    label: t("locks.delay.label"),
    // Like unlocking in Settings: stopping is asked for, counts down, and can be aborted.
    form: (session) => {
      const input = el("cds-text-input", { label: t("locks.delay.field"), placeholder: t("locks.delay.placeholder"), "helper-text": t("locks.delay.helper") });
      input.value = settingsOf(session, "delay").delay || "";
      return {
        fields: [input],
        read: async () => {
          const delay = input.value.trim();
          const fault = await invoke("describe_duration", { text: delay }).then(() => null, (e) => String(e));
          return invalid(input, fault) ? null : { method: "delay", delay };
        },
      };
    },
    describe: (lock) => t("locks.delay.describe", { delay: durationText(lock.delay) }),
    confirm: async (session) => {
      const delay = await invoke("describe_duration", { text: session.lock.delay || "" }).catch(() => session.lock.delay);
      const values = { name: quoted(session.name), delay };
      return {
        heading: t("locks.delay.confirmHeading", values),
        body: t("locks.delay.confirmBody", values),
        action: t("locks.delay.confirmAction", values),
      };
    },
  },
  text: {
    label: t("locks.text.label"),
    // A text of its own: stopping the session means typing it in exactly as it is shown, with
    // nothing pasted (`askText` in tiles.js). It is no secret, so it is kept as typed here.
    form: (session) => {
      const area = el("cds-textarea", { label: t("locks.text.field"), placeholder: t("locks.text.placeholder"), "helper-text": t("locks.text.helper"), rows: 4, "max-count": "1000" });
      area.value = settingsOf(session, "text").text || "";
      return { fields: [area], read: async () => (invalid(area, area.value.trim() ? null : t("locks.text.missing")) ? null : { method: "text", text: area.value }) };
    },
    describe: () => t("locks.text.describe"),
    confirm: async (session) => ({
      heading: t("locks.text.confirmHeading", { name: quoted(session.name) }),
      body: t("locks.text.confirmBody", { name: quoted(session.name) }),
      action: t("locks.text.confirmAction", { name: quoted(session.name) }),
    }),
  },
  password: {
    label: t("locks.password.label"),
    // Sent once; the service keeps only a salted hash and never sends either back, so a session
    // that has one just says so. Typed twice, as nothing ever shows it again: a typo would leave a lock only the unlock
    // password opens.
    form: (session) => {
      // A password already set is never known here (only its hash is kept), so the fields hold
      // 8 dots in its place, as text of normal weight rather than a placeholder. They go as soon
      // as either field is entered and come back if both are left empty; saved as they are, the
      // session keeps its password.
      const set = settingsOf(session, "password").method === "password";
      const password = el("cds-password-input", { label: t("locks.password.field"), autocomplete: "new-password", "helper-text": t(set ? "locks.password.helperSet" : "locks.password.helper") });
      const repeat = el("cds-password-input", { label: t("locks.password.repeat"), autocomplete: "new-password" });
      const fields = [password, repeat];
      if (set) {
        const stand = () => fields.forEach((field) => (field.value = KEPT));
        stand();
        for (const field of fields) {
          field.dataset.kept = "1";
          field.addEventListener("focusin", () => {
            if (field.value === KEPT) fields.forEach((f) => (f.value = ""));
          });
          field.addEventListener("focusout", () =>
            setTimeout(() => {
              if (!fields.some((f) => f.matches(":focus-within") || f.value)) stand();
            }, 0),
          );
        }
      }
      return {
        fields,
        read: async () => {
          if (set && fields.every((field) => field.value === KEPT)) return { method: "password" };
          if (invalid(password, password.value ? null : t("locks.password.missing"))) return null;
          if (invalid(repeat, repeat.value === password.value ? null : t("locks.password.differ"))) return null;
          return { method: "password", password: password.value };
        },
      };
    },
    describe: () => t("locks.password.describe"),
    confirm: async (session) => ({
      heading: t("locks.password.confirmHeading", { name: quoted(session.name) }),
      body: t("locks.password.confirmBody", { name: quoted(session.name) }),
      action: t("locks.password.confirmAction", { name: quoted(session.name) }),
    }),
  },
};

export { KEPT, settingsOf, LOCKS, conditionLockText };
