// Page logic without the page, so `npm test` can check it with plain Node.

export const COLOR_COUNT = 6;
export const CARD_MAX_LINES = 14;
const COLUMN_WIDTH = 250;
const GAP = 12;

export const THEMES = ['system', 'light', 'dark'];

/** Anything unknown, including nothing saved yet, follows the system. */
export function normalTheme(value) {
  return THEMES.includes(value) ? value : 'system';
}

export function colorClass(index) {
  return `color-${Math.min(Math.max(index | 0, 0), COLOR_COUNT - 1)}`;
}

export function columnCountFor(width) {
  return Math.min(6, Math.max(1, Math.floor((width - GAP * 2) / COLUMN_WIDTH)));
}

/** Each note goes to the shortest column so far, so the order reads across. */
export function distribute(notes, columnCount) {
  const columns = Array.from({ length: columnCount }, () => []);
  const heights = new Array(columnCount).fill(0);
  for (const note of notes) {
    let shortest = 0;
    for (let i = 1; i < columnCount; i++) {
      if (heights[i] < heights[shortest]) shortest = i;
    }
    columns[shortest].push(note);
    heights[shortest] += 3 + Math.min(note.text.split('\n').length, CARD_MAX_LINES);
  }
  return columns;
}

/**
 * Where a dragged card lands, as the `before` id `move_note` takes: the card
 * it is over, or the one after that card when over its lower half. `null` is
 * the end of the list. `undefined` means it cannot go there: pinned notes
 * stay above the others, so a card only moves within its own group.
 */
export function dropBefore(notes, draggedId, targetId, lowerHalf) {
  const dragged = notes.find((note) => note.id === draggedId);
  const target = notes.find((note) => note.id === targetId);
  if (!dragged || !target || dragged === target || dragged.pinned !== target.pinned) return undefined;
  if (!lowerHalf) return target.id;
  const next = notes.slice(notes.indexOf(target) + 1).find((note) => note.id !== draggedId);
  return next ? next.id : null;
}

export function matches(note, query) {
  const wanted = query.trim().toLowerCase();
  return wanted === '' || note.text.toLowerCase().includes(wanted);
}

/** Puts `[ ] ` at the start of the caret's line; the caret moves with it. */
export function insertCheckbox(text, caret) {
  const at = Math.min(Math.max(caret, 0), text.length);
  const lineStart = text.lastIndexOf('\n', at - 1) + 1;
  const mark = '[ ] ';
  return { text: text.slice(0, lineStart) + mark + text.slice(lineStart), caret: at + mark.length };
}

/**
 * Where the caret goes when the text is replaced from elsewhere: it keeps its
 * place when the change is before or after it, and is never out of range.
 */
export function caretAfterReplace(oldText, newText, caret) {
  const at = Math.min(Math.max(caret, 0), oldText.length);
  const shorter = Math.min(oldText.length, newText.length);
  let prefix = 0;
  while (prefix < shorter && oldText[prefix] === newText[prefix]) prefix++;
  if (at <= prefix) return at;
  let suffix = 0;
  while (
    suffix < shorter - prefix &&
    oldText[oldText.length - 1 - suffix] === newText[newText.length - 1 - suffix]
  ) suffix++;
  const fromEnd = oldText.length - at;
  // Inside the changed part: after the new text of that part.
  return fromEnd <= suffix ? newText.length - fromEnd : newText.length - suffix;
}

/**
 * Keeps an editor's text in step with the saved note. Typing is saved at once;
 * a change from elsewhere (a sync, another window) is shown only once every
 * save has finished and nothing is being typed. Each save names the text the
 * editor last showed, so the store can keep a change the editor never showed.
 */
export class EditorSync {
  /**
   * save(text, base) and load() return promises; load gives the saved text or
   * null. getValue/setValue read and replace what the editor shows.
   */
  constructor({ base, save, load, getValue, setValue, onError = () => {} }) {
    Object.assign(this, { base, save, load, getValue, setValue, onError });
    this.pending = 0;
    this.composing = false;
    this.wanted = false;
    this.closed = false;
  }

  /** Call on every input event. Text being composed (pinyin) waits. */
  typed() {
    if (this.composing || this.closed) return;
    const text = this.getValue();
    if (text === this.base) return;
    const base = this.base;
    this.base = text;
    this.pending++;
    this.save(text, base)
      .catch(this.onError)
      .finally(() => {
        this.pending--;
        if (this.pending === 0 && this.wanted) this.refresh();
      });
  }

  compositionStarted() {
    this.composing = true;
  }

  compositionEnded() {
    this.composing = false;
    this.typed();
  }

  /** Call whenever the saved notes may have changed. */
  async refresh() {
    if (this.closed) return;
    if (this.pending > 0 || this.composing) {
      this.wanted = true;
      return;
    }
    this.wanted = false;
    const before = this.getValue();
    let text;
    try {
      text = await this.load();
    } catch (error) {
      this.onError(error);
      return;
    }
    if (this.closed) return;
    // Typed while loading: what was loaded may be older than the editor.
    if (this.pending > 0 || this.composing || this.getValue() !== before) {
      this.refresh();
      return;
    }
    if (text === null) return;
    this.base = text;
    if (text !== before) this.setValue(text);
  }

  close() {
    this.closed = true;
  }
}
