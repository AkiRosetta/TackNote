import { test } from 'node:test';
import assert from 'node:assert/strict';

import {
  EditorSync,
  caretAfterReplace,
  colorClass,
  columnCountFor,
  distribute,
  dropBefore,
  insertCheckbox,
  matches,
  normalTheme,
} from '../src/logic.js';

const note = (text) => ({ text });

test('notes go to the shortest column, in order', () => {
  // Heights 7, then 4 and 8 in the second column, so "g" goes back to the first.
  const notes = [note('a\nb\nc\nd'), note('e'), note('f'), note('g')];
  const columns = distribute(notes, 2);
  assert.deepEqual(columns.map((c) => c.map((n) => n.text)), [['a\nb\nc\nd', 'g'], ['e', 'f']]);
  assert.equal(distribute(notes, 1)[0].length, 4);
});

test('column count follows the width, from 1 to 6', () => {
  assert.equal(columnCountFor(300), 1);
  assert.equal(columnCountFor(900), 3);
  assert.equal(columnCountFor(5000), 6);
  assert.equal(columnCountFor(0), 1);
});

test('colour classes stay within the palette', () => {
  assert.equal(colorClass(2), 'color-2');
  assert.equal(colorClass(99), 'color-5');
  assert.equal(colorClass(-1), 'color-0');
});

test('an unknown or missing theme follows the system', () => {
  assert.equal(normalTheme('dark'), 'dark');
  assert.equal(normalTheme('light'), 'light');
  assert.equal(normalTheme(null), 'system');
  assert.equal(normalTheme('blue'), 'system');
});

test('a dragged card lands before the card under it, or after its lower half', () => {
  const notes = [
    { id: 'p1', pinned: true }, { id: 'p2', pinned: true },
    { id: 'a', pinned: false }, { id: 'b', pinned: false }, { id: 'c', pinned: false },
  ];
  assert.equal(dropBefore(notes, 'c', 'a', false), 'a');
  assert.equal(dropBefore(notes, 'a', 'b', true), 'c');
  assert.equal(dropBefore(notes, 'a', 'c', true), null, 'after the last card is the end');
  assert.equal(dropBefore(notes, 'b', 'a', true), 'c', 'skips the dragged card itself');
  assert.equal(dropBefore(notes, 'p1', 'p2', true), 'a', 'after the last pinned note');
  assert.equal(dropBefore(notes, 'a', 'p2', false), undefined, 'not into the pinned group');
  assert.equal(dropBefore(notes, 'p1', 'a', false), undefined, 'not out of it');
  assert.equal(dropBefore(notes, 'a', 'a', false), undefined, 'not onto itself');
});

test('search ignores case and surrounding spaces', () => {
  assert.ok(matches(note('Buy Milk'), '  milk '));
  assert.ok(matches(note('买牛奶'), '牛奶'));
  assert.ok(matches(note('anything'), '   '));
  assert.ok(!matches(note('买牛奶'), '面包'));
});

test('a checkbox goes to the start of the caret line', () => {
  assert.deepEqual(insertCheckbox('one\ntwo', 5), { text: 'one\n[ ] two', caret: 9 });
  assert.deepEqual(insertCheckbox('', 0), { text: '[ ] ', caret: 4 });
  assert.deepEqual(insertCheckbox('one', 99), { text: '[ ] one', caret: 7 });
});

test('the caret keeps its place when text changes before or after it', () => {
  // Change after the caret.
  assert.equal(caretAfterReplace('hello world', 'hello there world', 2), 2);
  // Change before the caret: it moves with its text.
  assert.equal(caretAfterReplace('hello world', 'oh hello world', 8), 11);
  // Caret inside a deleted part: never past the end.
  assert.equal(caretAfterReplace('hello world', 'hello', 9), 5);
  // Everything replaced by something shorter.
  assert.equal(caretAfterReplace('abcdef', 'x', 6), 1);
  assert.equal(caretAfterReplace('abc', '', 3), 0);
  // Out-of-range caret from the old text.
  assert.equal(caretAfterReplace('abc', 'abc', 50), 3);
});

test('the caret is always within the new text', () => {
  const texts = ['', 'a', 'ab', 'ba', 'abc', 'aXc', '买牛奶', 'a\nb'];
  for (const before of texts) {
    for (const after of texts) {
      for (let caret = 0; caret <= before.length; caret++) {
        const moved = caretAfterReplace(before, after, caret);
        assert.ok(moved >= 0 && moved <= after.length, `${before} -> ${after} at ${caret}: ${moved}`);
      }
    }
  }
});

/** An editor whose saves and loads finish only when the test says so. */
function harness(saved) {
  const calls = [];
  const saves = [];
  const editor = { value: saved };
  const sync = new EditorSync({
    base: saved,
    save: (text, base) => {
      saves.push({ text, base });
      return new Promise((resolve) => calls.push({ kind: 'save', text, done: () => { saved = text; resolve(true); } }));
    },
    load: () => new Promise((resolve) => calls.push({ kind: 'load', done: () => resolve(saved) })),
    getValue: () => editor.value,
    setValue: (text) => { editor.value = text; },
  });
  const finish = async (kind) => {
    const index = calls.findIndex((call) => call.kind === kind);
    assert.ok(index >= 0, `no pending ${kind}`);
    calls.splice(index, 1)[0].done();
    await new Promise((resolve) => setTimeout(resolve, 0));
  };
  return {
    sync,
    editor,
    calls,
    saves,
    finish,
    remote: (text) => { saved = text; },
    type: (text) => { editor.value = text; sync.typed(); },
  };
}

test('a change from a sync shows in the editor', async () => {
  const h = harness('old');
  h.remote('from another device');
  h.sync.refresh();
  await h.finish('load');
  assert.equal(h.editor.value, 'from another device');
});

test('a sync during an unfinished save waits, then shows the latest text', async () => {
  const h = harness('a');
  h.type('ab');
  h.sync.refresh();
  assert.equal(h.calls.filter((c) => c.kind === 'load').length, 0, 'must not load while saving');
  await h.finish('save');
  await h.finish('load');
  assert.equal(h.editor.value, 'ab');
});

test('typing while a load is on its way is never undone', async () => {
  const h = harness('a');
  h.sync.refresh();
  h.type('ab');
  await h.finish('load'); // Loaded "a", older than the editor.
  assert.equal(h.editor.value, 'ab');
  await h.finish('save');
  await h.finish('load');
  assert.equal(h.editor.value, 'ab');
});

test('typing finished during a load still triggers a fresh load', async () => {
  const h = harness('a');
  h.sync.refresh();
  h.type('ab');
  await h.finish('save');
  h.remote('ab and more');
  await h.finish('load'); // Stale: the editor changed meanwhile.
  await h.finish('load');
  assert.equal(h.editor.value, 'ab and more');
});

test('pinyin being composed is neither saved nor replaced', async () => {
  const h = harness('');
  h.sync.compositionStarted();
  h.type('ni');
  assert.equal(h.calls.length, 0, 'composing text is not saved');
  h.remote('synced');
  h.sync.refresh();
  assert.equal(h.calls.length, 0, 'no load while composing');
  h.editor.value = '你';
  h.sync.compositionEnded();
  // The store sees that the editor never showed "synced" and keeps it.
  assert.deepEqual(h.saves, [{ text: '你', base: '' }]);
  await h.finish('save');
  await h.finish('load');
  assert.equal(h.editor.value, '你');
});

test('each save names the text the editor last showed', async () => {
  const h = harness('a');
  h.type('ab');
  h.type('abc');
  assert.deepEqual(h.saves, [{ text: 'ab', base: 'a' }, { text: 'abc', base: 'ab' }]);
  await h.finish('save');
  await h.finish('save');
  h.remote('from the phone');
  h.sync.refresh();
  await h.finish('load');
  h.type('from the phone!');
  assert.deepEqual(h.saves.at(-1), { text: 'from the phone!', base: 'from the phone' });
});

test('an input event that changes nothing saves nothing', () => {
  const h = harness('a');
  h.type('a');
  assert.equal(h.saves.length, 0);
});

test('a closed editor stops refreshing', async () => {
  const h = harness('a');
  h.sync.close();
  h.remote('b');
  await h.sync.refresh();
  assert.equal(h.calls.length, 0);
  assert.equal(h.editor.value, 'a');
});
