// The window in a browser: the sync server serves this page (`server/src/http.rs`) for managing every
// computer that syncs. In the app Tauri gives the page `window.__TAURI__`; here this stands in for it,
// before the page's modules run: each command goes to the server's `POST api/invoke` with the token
// (`web.rs` answers it as the app does), and what only a computer has is refused there with a
// message. The token is asked for once, in a dialog, and kept in this browser (`localStorage`) until
// Sign out in Settings; a wrong one is asked for again. A plain script, not a module, so it runs first; in the app it does
// nothing.

(() => {
  if (window.__TAURI__) return;
  // What the page hides that only a computer has (`main.js`).
  window.BULLSEYE_WEB = true;

  const KEY = "bullseye-token";
  const kept = () => localStorage.getItem(KEY);
  const keep = (token) => localStorage.setItem(KEY, token);
  // Sign out (`main.js`): the token forgotten, and the page started again, which asks for it.
  window.BULLSEYE_SIGN_OUT = () => {
    localStorage.removeItem(KEY);
    location.reload();
  };
  // A text of `strings.js`, with its {placeholders} filled.
  const t = (path, values = {}) => {
    const text = path.split(".").reduce((node, key) => (node ? node[key] : undefined), window.STRINGS);
    return typeof text === "string" ? text.replace(/\{(\w+)\}/g, (all, key) => (key in values ? values[key] : all)) : path;
  };

  // The browser's offset from UTC in minutes east: the server words times in it.
  const post = (token, cmd, args) =>
    fetch("api/invoke", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: JSON.stringify({ cmd, args, zone: -new Date().getTimezoneOffset() }),
    });

  // The sign-in dialog: Carbon's modal with a password field, which only closes on a token the server
  // takes. Resolves with that token.
  function signIn() {
    const el = (tag, attributes = {}, ...children) => {
      const node = document.createElement(tag);
      for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, value);
      node.append(...children);
      return node;
    };
    const field = el("cds-password-input", { label: t("web.token"), autocomplete: "off" });
    const problem = el("cds-inline-notification", { kind: "error", "low-contrast": "", "hide-close-button": "" });
    problem.hidden = true;
    const button = el("cds-modal-footer-button", { kind: "primary" }, t("web.signIn"));
    const modal = el(
      "cds-modal",
      { class: "sign-in", size: "xs", "prevent-close-on-click-outside": "" },
      el("cds-modal-header", {}, el("cds-modal-heading", {}, t("web.heading"))),
      el("cds-modal-body", {}, el("cds-modal-body-content", {}, t("web.text")), problem, el("div", { class: "modal-fields" }, field)),
      el("cds-modal-footer", {}, button),
    );
    // Nothing works without it: Escape does not close it.
    modal.addEventListener("cds-modal-beingclosed", (event) => event.preventDefault());
    document.body.append(modal);
    modal.open = true;
    setTimeout(() => field.focus(), 100);
    return new Promise((resolve) => {
      const tryIt = async () => {
        const token = (field.value || "").trim();
        field.invalid = false;
        problem.hidden = true;
        if (!token) {
          field.invalidText = t("web.wrong");
          field.invalid = true;
          return;
        }
        button.disabled = true;
        try {
          const response = await post(token, "status", {});
          if (response.status === 401) {
            field.invalidText = t("web.wrong");
            field.invalid = true;
          } else if (!response.ok) {
            problem.setAttribute("subtitle", t("web.unreachable", { why: `${response.status} ${response.statusText}` }));
            problem.hidden = false;
          } else {
            keep(token);
            modal.open = false;
            setTimeout(() => modal.remove(), 500);
            resolve(token);
          }
        } catch (e) {
          problem.setAttribute("subtitle", t("web.unreachable", { why: String(e) }));
          problem.hidden = false;
        }
        button.disabled = false;
      };
      button.addEventListener("click", tryIt);
      field.addEventListener("keydown", (event) => {
        if (event.key === "Enter") tryIt();
      });
    });
  }

  let signing = null;
  const token = () => {
    const known = kept();
    if (known) return Promise.resolve(known);
    return (signing = signing || signIn());
  };

  // A command, answered as Tauri answers the app's: its value, or a refusal with what went wrong.
  async function invoke(cmd, args = {}) {
    // The window is shown by the app when the page has drawn; a browser shows it anyway.
    if (cmd === "window_ready") return null;
    const given = await token();
    let response;
    try {
      response = await post(given, cmd, args);
    } catch (e) {
      throw t("web.unreachable", { why: String(e) });
    }
    if (response.status === 401) {
      // The token was changed on the server: asked for again (once, however many were on their way).
      if (kept() === given) {
        localStorage.removeItem(KEY);
        signing = null;
      }
      return invoke(cmd, args);
    }
    const body = await response.json().catch(() => null);
    if (!response.ok) throw body && "error" in body ? body.error : t("web.unreachable", { why: `${response.status} ${response.statusText}` });
    return body;
  }

  // The app tells the page when its window is hidden; a browser tab has `document.hidden`.
  window.__TAURI__ = { core: { invoke }, event: { listen: () => Promise.resolve(() => {}) } };
})();
