// Making a target or condition on the way to something else: from the session editor, for
// the session being made or changed, or for a session's lock condition. It remembers where to go
// back (`back`), the breadcrumb up to there, the session as it was being edited (`draft`), and
// what to do with the new item once saved (`onSaved`). Used up when its editor is left.
const detour = { current: null };

export { detour };
