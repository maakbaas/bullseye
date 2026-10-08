// Making a target or condition on the way from a session's tile: for its targets or condition
// (the + beside them, `fields.js`) or its lock condition (`tiles.js`). It remembers where to go
// back (`back`), the breadcrumb up to there, and what to do with the new item once saved
// (`onSaved`). Used up when its editor is left.
const detour = { current: null };

export { detour };
