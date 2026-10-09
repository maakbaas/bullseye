// Put into the window before its own scripts (Playwright's init script): every text of strings.js
// gets an invisible mark with its number around it, so wherever a text ends up (in the page, in a
// Carbon component's shadow DOM, in an attribute, in a text the stand-in service sends back) it can
// be told exactly which text it is, and where it is drawn. Matching by the words instead would mix up
// texts that read alike ("Locked", "Active").
//
// The marks are format characters that draw nothing and allow no line break, so the window looks and
// wraps as it does without them.

(() => {
  const DIGITS = ["⁡", "⁢", "⁣", "⁤"];
  const START = "⁪";
  const END = "⁫";
  const WIDTH = 5; // digits of a number, base 4: up to 1024 texts
  const code = (n) => {
    let s = "";
    for (let i = 0; i < WIDTH; i++) {
      s = DIGITS[n % 4] + s;
      n = Math.floor(n / 4);
    }
    return s;
  };
  const decode = (s, at) => {
    let n = 0;
    for (let i = 0; i < WIDTH; i++) {
      const d = DIGITS.indexOf(s[at + i]);
      if (d < 0) return -1;
      n = n * 4 + d;
    }
    return n;
  };

  // The texts in the order the file has them (the build checks that it reads the same order). An
  // empty text stays empty: the page tests some for being empty.
  const paths = [];
  const mark = (node, prefix) => {
    for (const key of Object.keys(node)) {
      const path = prefix ? `${prefix}.${key}` : key;
      if (typeof node[key] === "string") {
        const id = paths.push(path) - 1;
        if (node[key] !== "") node[key] = START + code(id) + node[key] + END;
      } else mark(node[key], path);
    }
  };
  let strings;
  Object.defineProperty(window, "STRINGS", {
    configurable: true,
    get: () => strings,
    set: (value) => {
      mark(value, "");
      strings = value;
    },
  });
  window.__paths = paths;
  window.__unmark = (s) => String(s).replace(new RegExp(`${START}[${DIGITS.join("")}]{${WIDTH}}|${END}`, "g"), "");

  // ---- Finding the texts on screen.

  // The parent in the tree as drawn: a slotted node's slot, a shadow root's host.
  const up = (node) => node.assignedSlot || (node.parentNode instanceof ShadowRoot ? node.parentNode.host : node.parentNode);
  // Whether an element is drawn. One that only passes its content on (`display: contents`, as Carbon's
  // slots are) has no box of its own: its nearest ancestor with one says.
  const shown = (element) => {
    let box = element;
    while (box && box.nodeType === 1 && getComputedStyle(box).display === "contents") box = up(box);
    if (!box || box.nodeType !== 1) return false;
    return !box.checkVisibility || box.checkVisibility({ opacityProperty: true, visibilityProperty: true, contentVisibilityAuto: true });
  };
  const area = (r) => r.width * r.height;
  const union = (rects) => {
    const l = Math.min(...rects.map((r) => r.left));
    const t = Math.min(...rects.map((r) => r.top));
    const r = Math.max(...rects.map((r) => r.right));
    const b = Math.max(...rects.map((r) => r.bottom));
    return { left: l, top: t, right: r, bottom: b, width: r - l, height: b - t };
  };
  // What clips an element: the window, and every scrolling or clipping box it is drawn in, up to one
  // that is fixed to the window (a dialog, a menu Carbon moved out).
  const clipOf = (element) => {
    let c = { left: 0, top: 0, right: innerWidth, bottom: innerHeight };
    for (let n = element; n && n.nodeType === 1; n = up(n)) {
      const style = getComputedStyle(n);
      if (n !== element && (style.overflowX !== "visible" || style.overflowY !== "visible")) {
        const b = n.getBoundingClientRect();
        c = { left: Math.max(c.left, b.left), top: Math.max(c.top, b.top), right: Math.min(c.right, b.right), bottom: Math.min(c.bottom, b.bottom) };
      }
      if (style.position === "fixed") break;
    }
    return c;
  };
  const within = (r, c) => r.left >= c.left - 1 && r.top >= c.top - 1 && r.right <= c.right + 1 && r.bottom <= c.bottom + 1;
  // The nearest drawn ancestor with a size (where a hidden text, a tooltip or a label for screen
  // readers, belongs), unless that is most of the window and says nothing.
  const holder = (node) => {
    for (let n = node; n; n = up(n)) {
      if (n.nodeType !== 1 || !shown(n)) continue;
      const r = n.getBoundingClientRect();
      if (area(r) > 0) return area(r) < innerWidth * innerHeight * 0.4 ? n : null;
    }
    return null;
  };

  // Every place a marked text is, now: [{ key, id, q, how, rect, inView }]. `q` ranks how well a
  // place shows the text (lower is better): 0 drawn as text, 1 the value of a field, 2 an attribute
  // (a placeholder, a label for screen readers) or a tooltip not shown, highlighted on its element,
  // 3 drawn but under the dialog.
  window.__scan = () => {
    const found = [];
    const elements = [];
    const open = [];
    const modal = document.getElementById("confirm");
    const modalOpen = !!(modal && modal.open);
    const inModal = (node) => {
      for (let n = node; n; n = up(n)) if (n === modal) return true;
      return false;
    };
    // The rects of a text, one per line it is drawn on.
    const byLine = (rects) => {
      const lines = [];
      for (const r of rects) {
        const line = lines.find((l) => Math.abs(l.top - r.top) < 3 && Math.abs(l.bottom - r.bottom) < 3);
        if (line) Object.assign(line, union([line, r]));
        else lines.push({ left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: r.width, height: r.height });
      }
      return lines.map((r) => ({ x: r.left, y: r.top, w: r.width, h: r.height }));
    };
    const add = (id, element, rect, q, how, parts = [rect]) => {
      if (!rect || area(rect) === 0) return;
      const clip = clipOf(element);
      const seen = { left: Math.max(rect.left, clip.left), top: Math.max(rect.top, clip.top), right: Math.min(rect.right, clip.right), bottom: Math.min(rect.bottom, clip.bottom) };
      found.push({ id, q, how, element, rect: { x: rect.left, y: rect.top, w: rect.width, h: rect.height }, rects: byLine(parts), inView: within(rect, clip), visible: seen.right > seen.left && seen.bottom > seen.top });
    };
    const finish = (entry) => {
      const element = entry.parent;
      const rects = entry.rects;
      if (rects.length && element && shown(element)) {
        const occluded = modalOpen && !inModal(element);
        add(entry.id, element, union(rects), occluded ? 3 : 0, "text", rects);
      } else if (element && rects.length) {
        // Laid out but not seen (a tooltip until hovered): boxed on what it belongs to. Text with no
        // box at all (in a hidden element) is not seen anywhere and is no place.
        const h = holder(element);
        if (h) add(entry.id, h, h.getBoundingClientRect(), 2, "hidden");
      }
    };
    const text = (node) => {
      const s = node.data;
      let from = 0;
      const flush = (a, b) => {
        if (b <= a || !open.length) return;
        const range = document.createRange();
        range.setStart(node, a);
        range.setEnd(node, b);
        const rects = [...range.getClientRects()].filter((r) => r.width > 0 && r.height > 0);
        for (const entry of open) {
          entry.rects.push(...rects);
          if (!entry.parent) entry.parent = node.parentElement || up(node);
        }
      };
      for (let i = 0; i < s.length; i++) {
        if (s[i] === START) {
          flush(from, i);
          open.push({ id: decode(s, i + 1), rects: [], parent: null });
          i += WIDTH;
          from = i + 1;
        } else if (s[i] === END) {
          flush(from, i);
          const entry = open.pop();
          if (entry) finish(entry);
          from = i + 1;
        }
      }
      flush(from, s.length);
    };
    const ids = (s) => {
      const out = [];
      for (let i = s.indexOf(START); i >= 0; i = s.indexOf(START, i + 1)) {
        const id = decode(s, i + 1);
        if (id >= 0) out.push(id);
      }
      return out;
    };
    const attributes = (element) => {
      for (const { name, value } of element.attributes) {
        if (!value.includes(START)) continue;
        // Only on an element that is drawn: a hidden field's attributes are not seen anywhere.
        if (!shown(element) || area(element.getBoundingClientRect()) === 0) continue;
        // A placeholder shows only while its field is empty.
        if (name === "placeholder" && "value" in element && String(element.value) !== "") continue;
        for (const id of ids(value)) add(id, element, element.getBoundingClientRect(), 2, `attr:${name}`);
      }
      if ((element.tagName === "INPUT" || element.tagName === "TEXTAREA") && String(element.value).includes(START)) {
        for (const id of ids(element.value)) add(id, element, element.getBoundingClientRect(), shown(element) ? 1 : 2, "value");
      }
    };
    const walk = (node) => {
      if (node.nodeType === 3) return text(node);
      if (node.nodeType === 1) {
        if (node.tagName === "SCRIPT" || node.tagName === "STYLE" || node.tagName === "TITLE") return;
        attributes(node);
        if (node.tagName === "SLOT") {
          const assigned = node.assignedNodes({ flatten: true });
          return (assigned.length ? assigned : [...node.childNodes]).forEach(walk);
        }
        if (node.shadowRoot) return [...node.shadowRoot.childNodes].forEach(walk);
      }
      [...node.childNodes].forEach(walk);
    };
    walk(document.body);
    // Each place gets a key that stays the same while the page does: the text's number and how many
    // places of it came before.
    const count = {};
    window.__places = {};
    return found.map((place) => {
      count[place.id] = (count[place.id] || 0) + 1;
      const key = `${place.id}#${count[place.id]}`;
      window.__places[key] = place.element;
      elements.push(place.element);
      return { key, id: place.id, q: place.q, how: place.how, rect: place.rect, rects: place.rects, inView: place.inView, visible: place.visible };
    });
  };

  // Brings a place into view: whatever scrolls around it (the page, the dialog's body) scrolls.
  window.__reveal = (key) => {
    const element = window.__places && window.__places[key];
    if (element) element.scrollIntoView({ block: "center", inline: "nearest", behavior: "instant" });
  };
})();
