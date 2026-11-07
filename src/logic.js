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

/** `ids` with `id` moved to just before `before`, or to the end for `null`. */
export function moveId(ids, id, before) {
  const rest = ids.filter((other) => other !== id);
  const at = before === null ? rest.length : rest.indexOf(before);
  rest.splice(at < 0 ? rest.length : at, 0, id);
  return rest;
}

export function matches(note, query) {
  const wanted = query.trim().toLowerCase();
  return wanted === '' || note.text.toLowerCase().includes(wanted);
}

// To-do lines, the same rule as the Rust side: optional spaces, [ ], [x] or
// [X], then one optional space. The editor shows a box in their place.
const TODO_MARK = /^\s*\[( |x|X)\] ?/;

export function todoLines(text) {
  return text.split('\n').map((line) => {
    const mark = TODO_MARK.exec(line);
    return mark
      ? { todo: true, checked: mark[1] !== ' ', text: line.slice(mark[0].length) }
      : { todo: false, checked: false, text: line };
  });
}

/** The note text for these lines, with every to-do written as `[ ] ` or `[x] `. */
export function todoText(lines) {
  return lines.map((line) => (line.todo ? (line.checked ? '[x] ' : '[ ] ') : '') + line.text).join('\n');
}

function lineAround(text, offset) {
  const start = text.lastIndexOf('\n', offset - 1) + 1;
  const next = text.indexOf('\n', offset);
  const line = text.slice(start, next === -1 ? text.length : next);
  const mark = TODO_MARK.exec(line);
  return { start, line, markLength: mark ? mark[0].length : 0 };
}

/**
 * Enter: a new line, which is a to-do too when pressed on one. Enter on an
 * empty to-do makes it a plain line instead, the usual way out of a list.
 */
export function enterAt(text, start, end) {
  const { start: lineStart, line, markLength } = lineAround(text, start);
  if (markLength && start === end && line.slice(markLength).trim() === '') {
    return { text: text.slice(0, lineStart) + text.slice(lineStart + markLength), caret: lineStart };
  }
  const inserted = markLength ? '\n[ ] ' : '\n';
  return { text: text.slice(0, start) + inserted + text.slice(end), caret: start + inserted.length };
}

/**
 * Backspace at the start of a line's text: a to-do loses its box first, a
 * plain line joins the line above. Anywhere else it is an ordinary delete,
 * left to the editor: returns null.
 */
export function backspaceAt(text, caret) {
  const { start, markLength } = lineAround(text, caret);
  if (caret !== start + markLength) return null;
  if (markLength) return { text: text.slice(0, start) + text.slice(start + markLength), caret: start };
  if (start === 0) return { text, caret: 0 };
  return { text: text.slice(0, start - 1) + text.slice(start), caret: start - 1 };
}

/** Ticks or unticks the to-do on line `index`; other lines keep their text. */
export function toggleDone(text, index) {
  const lines = todoLines(text);
  if (!lines[index]?.todo) return text;
  lines[index] = { ...lines[index], checked: !lines[index].checked };
  return todoText(lines);
}

/**
 * Makes the caret's line a to-do, or a plain line again if it already is
 * one, so pressing the button twice undoes it. The caret moves with its text.
 */
export function toggleTodo(text, caret) {
  const at = Math.min(Math.max(caret, 0), text.length);
  const lineStart = text.lastIndexOf('\n', at - 1) + 1;
  // The same rule as the Rust side: optional spaces, [ ], [x] or [X], one space.
  const mark = /^\s*\[( |x|X)\] ?/.exec(text.slice(lineStart));
  if (mark) {
    const end = lineStart + mark[0].length;
    return { text: text.slice(0, lineStart) + text.slice(end), caret: Math.max(lineStart, at - mark[0].length) };
  }
  return { text: text.slice(0, lineStart) + '[ ] ' + text.slice(lineStart), caret: at + 4 };
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
