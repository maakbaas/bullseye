// Making a block list or condition on the way to something else: from the block editor, for
// the block being made or changed, or for a block's lock condition. It remembers where to go
// back (`back`), the breadcrumb up to there, the block as it was being edited (`draft`), and
// what to do with the new item once saved (`onSaved`). Used up when its editor is left.
const detour = { current: null };

export { detour };
