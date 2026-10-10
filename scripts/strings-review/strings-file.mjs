// Reads src/strings.js as text: every text in it, in file order, with its path and where its value
// sits (`start`, `end`: the offsets between its quotes), so the review page can put an edited text
// back in exactly that place and leave every other character of the file as it is.
//
// The file is the small subset of JavaScript that `strings.rs` reads too: nested objects, unquoted
// keys, double-quoted strings without escapes, `//` comments.

import { readFileSync } from "node:fs";

export function readStrings(file) {
  const text = readFileSync(file, "utf8");
  const entries = [];
  let at = text.indexOf("window.STRINGS =");
  if (at < 0) throw new Error("strings.js: no `window.STRINGS =`");
  at += "window.STRINGS =".length;
  const fail = (why) => {
    const line = text.slice(0, at).split("\n").length;
    throw new Error(`strings.js line ${line}: ${why}`);
  };
  const space = () => {
    for (;;) {
      while (/\s/.test(text[at] || "")) at++;
      if (!text.startsWith("//", at)) return;
      while (at < text.length && text[at] !== "\n") at++;
    }
  };
  const group = (path) => {
    space();
    if (text[at] !== "{") fail("expected {");
    at++;
    for (;;) {
      space();
      if (text[at] === "}") return at++;
      const from = at;
      while (/[A-Za-z0-9_]/.test(text[at] || "")) at++;
      const key = text.slice(from, at);
      if (!key) fail("expected a key");
      space();
      if (text[at] !== ":") fail(`expected : after ${key}`);
      at++;
      space();
      const here = [...path, key];
      if (text[at] === "{") group(here);
      else if (text[at] === '"') {
        const start = ++at;
        while (text[at] !== '"') {
          if (at >= text.length || text[at] === "\n") fail("unterminated text");
          at++;
        }
        entries.push({ path: here.join("."), start, end: at, value: text.slice(start, at) });
        at++;
      } else fail(`expected a text or a group for ${key}`);
      space();
      if (text[at] === ",") at++;
    }
  };
  group([]);
  return { text, entries };
}
